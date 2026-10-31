// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Virtual-display policy, sessions and status.

use super::*;
use crate::events::UiEvent;
use crate::store::{save_virtual_display_policy, DeviceType, VirtualDisplayPolicy};
use tetherly_core::{
    CoreError, DeviceId, DisplayEdge, VirtualDisplayAnnounceFrame, VirtualDisplayAttachFrame,
    VirtualDisplayCodec, VirtualDisplayDetachFrame, VirtualDisplayFrameAck, VirtualDisplayMode,
    VirtualDisplayPixelFormat, VirtualDisplayRole, VirtualDisplayValidator,
};
use tracing::{debug, warn};

/// Active virtual secondary display session metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ActiveVirtualDisplaySession {
    pub session_id: String,
    pub peer_id: String,
    pub role: VirtualDisplayRole,
    pub assigned_edge: DisplayEdge,
    pub mode: VirtualDisplayMode,
    pub codec: VirtualDisplayCodec,
    pub target_fps: u32,
    pub current_fps: u32,
    pub rtt_us: u64,
    pub dropped_frames: u32,
    pub started_at_ms: u64,
    pub last_activity_ms: u64,
}

/// Comprehensive status report of virtual secondary display streaming.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VirtualDisplayStatusReport {
    pub ok: bool,
    pub active: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ActiveVirtualDisplaySession>,
    pub policy: VirtualDisplayPolicy,
    pub timestamp_ms: u64,
}

/// Decision report when attaching or detaching a virtual display session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct VirtualDisplayDecisionReport {
    pub ok: bool,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<VirtualDisplayRole>,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Governance policy report for virtual secondary display.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct VirtualDisplayPolicyReport {
    pub ok: bool,
    pub policy: VirtualDisplayPolicy,
    pub message: String,
}

/// Partial update payload for virtual display policy.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VirtualDisplayPolicyUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_hosting: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_remote_extend: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution_width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_resolution_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fps: Option<u32>,
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
    /// Ingest and process an inbound VirtualDisplayAnnounceFrame from a peer.
    pub fn receive_virtual_display_announce(
        &self,
        peer: &DeviceId,
        frame: VirtualDisplayAnnounceFrame,
    ) {
        let peer_str = peer.to_string();
        debug!(
            peer = %peer_str,
            display = %frame.display_name,
            mode = ?frame.preferred_mode,
            "received virtual display announcement"
        );
    }

    /// Ingest and process an inbound VirtualDisplayAttachFrame to act as virtual display sink.
    pub async fn receive_virtual_display_attach(
        &self,
        peer: &DeviceId,
        mut frame: VirtualDisplayAttachFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        if VirtualDisplayValidator::validate_session_id(&frame.session_id).is_err() {
            warn!(peer = %peer, session_id = %frame.session_id, "invalid virtual display session_id rejected");
            return;
        }

        let policy = self.virtual_display_policy();
        let profile = self.peer_profile(peer);

        let rejection_reason = if !policy.enabled {
            Some("virtual secondary display disabled".to_string())
        } else if !policy.allow_hosting {
            Some("hosting virtual secondary display disabled by policy".to_string())
        } else if !policy.is_peer_eligible(&profile, true) {
            Some("peer ineligible under virtual display policy filters".to_string())
        } else {
            None
        };

        if let Some(reason) = rejection_reason {
            let detach = VirtualDisplayDetachFrame {
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
                "virtual_display_rejected",
                Some(peer_str.clone()),
                format!(
                    "Rejected inbound virtual secondary display attach from {}: {}",
                    peer_str, reason
                ),
                false,
            );
            return;
        }

        let validated_mode = match VirtualDisplayValidator::validate_mode(&frame.mode) {
            Ok(m) => m,
            Err(_) => {
                let detach = VirtualDisplayDetachFrame {
                    session_id: frame.session_id.clone(),
                    reason: Some("invalid mode specifications".to_string()),
                    timestamp_ms: now_ms,
                };
                if let Ok(inner) = detach.to_frame() {
                    let _ = self.send_to(peer, inner).await;
                }
                return;
            }
        };

        let target_fps = frame.target_fps.clamp(10, policy.max_fps);
        let session = ActiveVirtualDisplaySession {
            session_id: frame.session_id.clone(),
            peer_id: peer_str.clone(),
            role: VirtualDisplayRole::Sink,
            assigned_edge: frame.assigned_edge,
            mode: validated_mode.clone(),
            codec: frame.codec,
            target_fps,
            current_fps: target_fps,
            rtt_us: 0,
            dropped_frames: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_virtual_display
            .lock()
            .expect("active_virtual_display") = Some(session);

        self.emit(UiEvent::virtual_display_session_started(
            &frame.session_id,
            &peer_str,
            &alias,
            "sink",
            validated_mode.width,
            validated_mode.height,
            target_fps,
            frame.assigned_edge.as_str(),
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "virtual_display_sink_started",
            Some(peer_str.clone()),
            format!(
                "Virtual secondary display session {} attached as sink for host {}",
                frame.session_id, peer_str
            ),
            true,
        );
    }

    /// Ingest and process an inbound VirtualDisplayDetachFrame from the other end.
    pub async fn receive_virtual_display_detach(
        &self,
        peer: &DeviceId,
        mut frame: VirtualDisplayDetachFrame,
    ) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.sanitize();
        let was_active = {
            let mut guard = self
                .shared
                .active_virtual_display
                .lock()
                .expect("active_virtual_display");
            if let Some(session) = guard.take() {
                session.session_id == frame.session_id || session.peer_id == peer_str
            } else {
                false
            }
        };

        if was_active {
            self.emit(UiEvent::virtual_display_session_stopped(
                &frame.session_id,
                &peer_str,
                &alias,
                frame.reason.as_deref(),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "virtual_display_detached",
                Some(peer_str.clone()),
                format!(
                    "Virtual secondary display detached by peer {}: {:?}",
                    peer_str, frame.reason
                ),
                true,
            );
        }
    }

    /// Ingest and process frame ACK feedback for adaptive frame rate and backpressure tuning.
    pub fn receive_virtual_display_frame_ack(&self, peer: &DeviceId, ack: VirtualDisplayFrameAck) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();

        let mut guard = self
            .shared
            .active_virtual_display
            .lock()
            .expect("active_virtual_display");
        if let Some(ref mut session) = *guard {
            if session.session_id == ack.session_id && session.peer_id == peer_str {
                session.last_activity_ms = now_ms;
                session.rtt_us = ack.render_delay_us;
                session.dropped_frames = session.dropped_frames.saturating_add(ack.dropped_frames);

                // Adaptive frame rate tuning based on backpressure and render latency:
                if ack.dropped_frames > 2 || ack.render_delay_us > 50_000 {
                    session.current_fps = session.current_fps.saturating_sub(5).max(10);
                } else if ack.dropped_frames == 0 && ack.render_delay_us < 20_000 {
                    session.current_fps = (session.current_fps + 2).min(session.target_fps);
                }
            }
        }
    }

    /// Query status of active virtual secondary display session and governance policy.
    pub fn virtual_display_status(&self) -> VirtualDisplayStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.virtual_display_policy();
        let mut guard = self
            .shared
            .active_virtual_display
            .lock()
            .expect("active_virtual_display");

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

                self.emit(UiEvent::virtual_display_session_stopped(
                    &session.session_id,
                    &session.peer_id,
                    &alias,
                    Some("idle timeout exceeded"),
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Notice,
                    "virtual_display_idle_timeout",
                    Some(session.peer_id.clone()),
                    format!(
                        "Virtual display session {} closed due to idle timeout",
                        session.session_id
                    ),
                    true,
                );
                return VirtualDisplayStatusReport {
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

        VirtualDisplayStatusReport {
            ok: true,
            active,
            session,
            policy,
            timestamp_ms: now_ms,
        }
    }

    /// Request to attach a peer as a virtual secondary display (extending desktop).
    pub async fn virtual_display_attach(
        &self,
        peer_query: &str,
        edge: Option<DisplayEdge>,
        width: Option<u32>,
        height: Option<u32>,
        fps: Option<u32>,
    ) -> Result<VirtualDisplayDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let peer_id = self.resolve_peer(peer_query)?;
        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());

        let policy = self.virtual_display_policy();
        if !policy.enabled {
            return Err(CoreError::InvalidPayload(
                "virtual secondary display disabled".to_string(),
            ));
        }
        if !policy.allow_remote_extend {
            return Err(CoreError::InvalidPayload(
                "extending desktop to remote secondary display disabled by policy".to_string(),
            ));
        }
        let profile = self.peer_profile(&peer_id);
        if !policy.is_peer_eligible(&profile, false) {
            return Err(CoreError::InvalidPayload(
                "peer not eligible under virtual display policy".to_string(),
            ));
        }

        let req_width = width
            .unwrap_or(1920)
            .clamp(320, policy.max_resolution_width.min(7680));
        let req_height = height
            .unwrap_or(1080)
            .clamp(240, policy.max_resolution_height.min(4320));
        let req_fps = fps.unwrap_or(60).clamp(10, policy.max_fps.min(120));
        let assigned_edge = edge.unwrap_or(DisplayEdge::Right);

        let mode = VirtualDisplayMode {
            width: req_width,
            height: req_height,
            refresh_rate: req_fps,
            scale_factor: 1.0,
            pixel_format: VirtualDisplayPixelFormat::Bgra8,
        };

        let session_id = format!("vdisp_{}", now_ms);
        let my_id = self.shared.identity.device_id().to_string();

        let attach_frame = VirtualDisplayAttachFrame {
            session_id: session_id.clone(),
            peer_id: my_id,
            assigned_edge,
            mode: mode.clone(),
            codec: VirtualDisplayCodec::ZstdBgra,
            target_fps: req_fps,
            timestamp_ms: now_ms,
        };

        let inner = attach_frame.to_frame()?;
        self.send_to(&peer_id, inner).await.map_err(|e| {
            CoreError::InvalidPayload(format!(
                "failed to send virtual display attach frame to peer: {e}"
            ))
        })?;

        let session = ActiveVirtualDisplaySession {
            session_id: session_id.clone(),
            peer_id: peer_str.clone(),
            role: VirtualDisplayRole::Host,
            assigned_edge,
            mode,
            codec: VirtualDisplayCodec::ZstdBgra,
            target_fps: req_fps,
            current_fps: req_fps,
            rtt_us: 0,
            dropped_frames: 0,
            started_at_ms: now_ms,
            last_activity_ms: now_ms,
        };

        *self
            .shared
            .active_virtual_display
            .lock()
            .expect("active_virtual_display") = Some(session);

        self.emit(UiEvent::virtual_display_session_started(
            &session_id,
            &peer_str,
            &alias,
            "host",
            req_width,
            req_height,
            req_fps,
            assigned_edge.as_str(),
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Notice,
            "virtual_display_attached",
            Some(peer_str.clone()),
            format!(
                "Extended desktop to peer {} ({}x{}@{}fps on edge {:?})",
                peer_str, req_width, req_height, req_fps, assigned_edge
            ),
            true,
        );

        Ok(VirtualDisplayDecisionReport {
            ok: true,
            action: "attach".to_string(),
            session_id: Some(session_id),
            peer_id: Some(peer_str),
            role: Some(VirtualDisplayRole::Host),
            message: "virtual secondary display attached to peer".to_string(),
            timestamp_ms: now_ms,
        })
    }

    /// Detach the active virtual secondary display session.
    pub async fn virtual_display_detach(
        &self,
        reason: Option<&str>,
    ) -> Result<VirtualDisplayDecisionReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let reason_str = reason.unwrap_or("user requested detach");
        let sanitized_reason = VirtualDisplayValidator::sanitize_reason(reason_str);

        let active_opt = {
            let mut guard = self
                .shared
                .active_virtual_display
                .lock()
                .expect("active_virtual_display");
            guard.take()
        };

        if let Some(session) = active_opt {
            let peer_id = DeviceId::parse(&session.peer_id).ok();
            let detach_frame = VirtualDisplayDetachFrame {
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

            self.emit(UiEvent::virtual_display_session_stopped(
                &session.session_id,
                &session.peer_id,
                &alias,
                Some(&sanitized_reason),
                now_ms,
            ));

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Notice,
                "virtual_display_detached",
                Some(session.peer_id.clone()),
                format!(
                    "Virtual display session {} detached: {}",
                    session.session_id, sanitized_reason
                ),
                true,
            );

            Ok(VirtualDisplayDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: Some(session.session_id),
                peer_id: Some(session.peer_id),
                role: Some(session.role),
                message: format!("virtual display session detached: {}", sanitized_reason),
                timestamp_ms: now_ms,
            })
        } else {
            Ok(VirtualDisplayDecisionReport {
                ok: true,
                action: "detach".to_string(),
                session_id: None,
                peer_id: None,
                role: None,
                message: "no active virtual display session to detach".to_string(),
                timestamp_ms: now_ms,
            })
        }
    }

    /// Returns the current virtual display governance policy.
    pub fn virtual_display_policy(&self) -> VirtualDisplayPolicy {
        self.shared
            .virtual_display_policy
            .lock()
            .expect("virtual_display_policy")
            .clone()
    }

    /// Status and configuration report for virtual display policy.
    pub fn virtual_display_policy_report(&self) -> VirtualDisplayPolicyReport {
        VirtualDisplayPolicyReport {
            ok: true,
            policy: self.virtual_display_policy(),
            message: "virtual display policy report".to_string(),
        }
    }

    /// Update virtual display policy and persist to disk atomically with 0o600 permissions.
    pub fn set_virtual_display_policy(
        &self,
        mut policy: VirtualDisplayPolicy,
    ) -> Result<VirtualDisplayPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_virtual_display_policy(&self.shared.cfg.data_dir, &policy).map_err(|e| {
            CoreError::File(format!("failed to save virtual_display_policy.json: {e}"))
        })?;
        *self
            .shared
            .virtual_display_policy
            .lock()
            .expect("virtual_display_policy") = policy.clone();

        self.emit(UiEvent::virtual_display_policy_changed(
            policy.enabled,
            policy.allow_hosting,
            policy.allow_remote_extend,
            policy.max_resolution_width,
            policy.max_resolution_height,
            policy.max_fps,
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
            "virtual_display_policy_changed",
            None::<String>,
            format!(
                "Updated virtual display policy: enabled={}, allow_hosting={}, allow_remote_extend={}, max_res={}x{}, max_fps={}",
                policy.enabled,
                policy.allow_hosting,
                policy.allow_remote_extend,
                policy.max_resolution_width,
                policy.max_resolution_height,
                policy.max_fps
            ),
            true,
        );

        Ok(VirtualDisplayPolicyReport {
            ok: true,
            policy,
            message: "virtual display policy updated".to_string(),
        })
    }

    /// Partially update virtual display policy and persist to disk.
    pub fn update_virtual_display_policy(
        &self,
        update: VirtualDisplayPolicyUpdate,
    ) -> Result<VirtualDisplayPolicyReport, CoreError> {
        let mut policy = self.virtual_display_policy();
        if let Some(v) = update.enabled {
            policy.enabled = v;
        }
        if let Some(v) = update.allow_hosting {
            policy.allow_hosting = v;
        }
        if let Some(v) = update.allow_remote_extend {
            policy.allow_remote_extend = v;
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
        self.set_virtual_display_policy(policy)
    }
}
