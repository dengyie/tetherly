// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Locate-ring policy and status.

use super::*;
use crate::events::UiEvent;
use crate::store::{save_locator_policy, LocatorPolicy};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tetherly_core::{
    CoreError, DeviceId, LocatePattern, LocateRequestFrame, LocateStopFrame, LocateValidator,
    DEFAULT_LOCATE_DURATION_SECS,
};
use tracing::{debug, warn};

/// Active acoustic ringing session on this local device.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActiveLocateRing {
    pub request_id: String,
    pub initiator_peer: String,
    pub initiator_alias: String,
    pub duration_secs: u32,
    pub pattern: LocatePattern,
    pub override_silent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub started_at_ms: u64,
    pub expires_at_ms: u64,
}

/// Status report for locator ringing on this node.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocateStatusReport {
    pub ok: bool,
    pub is_ringing: bool,
    pub active_ring: Option<ActiveLocateRing>,
    pub policy: LocatorPolicy,
    pub message: String,
}

/// Report after requesting a target mesh device to start ringing.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocateRingReport {
    pub ok: bool,
    pub request_id: String,
    pub target_peer: String,
    pub duration_secs: u32,
    pub pattern: LocatePattern,
    pub override_silent: bool,
    pub message: String,
}

/// Report after cancelling/stopping a locator ringing session.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocateStopReport {
    pub ok: bool,
    pub request_id: Option<String>,
    pub target_peer: Option<String>,
    pub reason: String,
    pub message: String,
}

/// Status and configuration report for device locator policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocatorPolicyReport {
    pub ok: bool,
    pub policy: LocatorPolicy,
    pub message: String,
}

impl Node {
    /// Read the current device locator policy.
    pub fn locator_policy(&self) -> LocatorPolicy {
        self.shared
            .locator_policy
            .lock()
            .expect("locator_policy")
            .clone()
    }

    /// Status and configuration report for device locator policy.
    pub fn locator_policy_report(&self) -> LocatorPolicyReport {
        LocatorPolicyReport {
            ok: true,
            policy: self.locator_policy(),
            message: "locator policy report".to_string(),
        }
    }

    /// Update the device locator policy and persist to disk atomically with 0o600 permissions.
    pub fn set_locator_policy(
        &self,
        mut policy: LocatorPolicy,
    ) -> Result<LocatorPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_locator_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save locator_policy.json: {e}")))?;
        *self.shared.locator_policy.lock().expect("locator_policy") = policy.clone();

        self.emit(UiEvent::locate_policy_changed(
            policy.enabled,
            policy.allow_override_silent,
            policy.max_duration_secs,
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
            "locate_policy_changed",
            None::<String>,
            format!(
                "Updated locator policy: enabled={}, allow_override_silent={}, max_duration_secs={}, groups={:?}, tags={:?}, types={:?}",
                policy.enabled,
                policy.allow_override_silent,
                policy.max_duration_secs,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
            ),
            true,
        );

        Ok(LocatorPolicyReport {
            ok: true,
            policy,
            message: "locator policy updated successfully".to_string(),
        })
    }

    /// Query current device locator status and active ringing state.
    pub fn locate_status(&self) -> LocateStatusReport {
        let active = self
            .shared
            .active_locate_ring
            .lock()
            .expect("active_locate_ring")
            .clone();
        let is_ringing = active.is_some();
        let message = if is_ringing {
            "device locator active ring in progress".to_string()
        } else {
            "device locator idle".to_string()
        };
        LocateStatusReport {
            ok: true,
            is_ringing,
            active_ring: active,
            policy: self.locator_policy(),
            message,
        }
    }

    /// Trigger acoustic ringing on a remote peer to locate it.
    pub async fn locate_ring(
        &self,
        peer_target: &str,
        duration_secs: Option<u32>,
        pattern: Option<LocatePattern>,
        override_silent: Option<bool>,
        message: Option<String>,
    ) -> Result<LocateRingReport, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let live = self.live_peers();
        if !live.iter().any(|p| p.device_id == peer_id) {
            return Err(CoreError::PeerNotFound(format!(
                "peer {} is not connected or offline",
                peer_id
            )));
        }

        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer_id.to_string();

        // Anti-harassment rate limiting: 10s cooldown per peer target
        {
            let mut last_map = self
                .shared
                .peer_last_locate_req_ms
                .lock()
                .expect("peer_last_locate_req_ms");
            if let Some(&last_ts) = last_map.get(&peer_str) {
                if now_ms.saturating_sub(last_ts) < 10_000 {
                    let wait_secs = (10_000 - now_ms.saturating_sub(last_ts)).div_ceil(1000);
                    return Err(CoreError::InvalidPayload(format!(
                        "anti-harassment rate limit: please wait at least {}s before ringing peer '{}' again",
                        wait_secs, peer_str
                    )));
                }
            }
            last_map.insert(peer_str.clone(), now_ms);
        }

        let seq = self.shared.locate_req_seq.fetch_add(1, Ordering::Relaxed);
        let request_id = format!("loc_{}_{}", now_ms, seq);

        let duration =
            LocateValidator::clamp_duration(duration_secs.unwrap_or(DEFAULT_LOCATE_DURATION_SECS));
        let pattern_val = pattern.unwrap_or_default();
        let override_val = override_silent.unwrap_or(true);
        let clean_msg = message.as_deref().map(LocateValidator::sanitize_message);

        let frame = LocateRequestFrame {
            request_id: request_id.clone(),
            duration_secs: duration,
            pattern: pattern_val,
            override_silent: override_val,
            message: clean_msg.clone(),
            timestamp_ms: now_ms,
        };

        self.send_to(&peer_id, frame.to_inner_frame()?).await?;

        let peer_str = peer_id.to_string();

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "locate_ring_sent",
            Some(peer_str.clone()),
            format!(
                "Sent acoustic locate request {} (pattern: {}, duration: {}s, override_silent: {}) to peer '{}'",
                request_id,
                pattern_val.as_str(),
                duration,
                override_val,
                peer_str
            ),
            true,
        );

        Ok(LocateRingReport {
            ok: true,
            request_id,
            target_peer: peer_str,
            duration_secs: duration,
            pattern: pattern_val,
            override_silent: override_val,
            message: clean_msg.unwrap_or_default(),
        })
    }

    /// Stop active acoustic ringing either locally or by sending TYPE_LOCATE_STOP to a remote peer.
    pub async fn locate_stop(
        &self,
        peer_target: Option<&str>,
        request_id: Option<String>,
        reason: Option<String>,
    ) -> Result<LocateStopReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let clean_reason = reason
            .as_deref()
            .map(LocateValidator::sanitize_reason)
            .unwrap_or_else(|| "manual_stop".to_string());

        let target_peer = if let Some(target) = peer_target {
            let t = target.trim();
            if t.is_empty() || t == "local" || t == self.shared.identity.device_id().as_str() {
                None
            } else {
                Some(self.resolve_peer(t)?)
            }
        } else {
            None
        };

        if let Some(peer_id) = target_peer {
            let clean_req_id = request_id
                .as_deref()
                .map(LocateValidator::sanitize_request_id);
            let frame = LocateStopFrame {
                request_id: clean_req_id.clone(),
                reason: clean_reason.clone(),
                timestamp_ms: now_ms,
            };
            self.send_to(&peer_id, frame.to_inner_frame()?).await?;

            let peer_str = peer_id.to_string();

            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "locate_stop_sent",
                Some(peer_str.clone()),
                format!(
                    "Sent locate stop to peer '{}' (request_id: {:?}, reason: '{}')",
                    peer_str, clean_req_id, clean_reason
                ),
                true,
            );

            Ok(LocateStopReport {
                ok: true,
                request_id: clean_req_id,
                target_peer: Some(peer_str),
                reason: clean_reason,
                message: "locate stop request dispatched to peer".to_string(),
            })
        } else {
            // Local stop
            let stopped_ring = {
                let mut active = self
                    .shared
                    .active_locate_ring
                    .lock()
                    .expect("active_locate_ring");
                if let Some(ring) = active.as_ref() {
                    if request_id.is_none() || request_id.as_deref() == Some(&ring.request_id) {
                        active.take()
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            let (req_id, peer_str) = if let Some(ring) = stopped_ring {
                self.emit(UiEvent::locate_ring_stopped(
                    Some(&ring.request_id),
                    &ring.initiator_peer,
                    &ring.initiator_alias,
                    &clean_reason,
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Info,
                    "locate_ring_stopped_locally",
                    Some(ring.initiator_peer.clone()),
                    format!(
                        "Locally stopped active acoustic ring {} from {} (reason: '{}')",
                        ring.request_id, ring.initiator_alias, clean_reason
                    ),
                    true,
                );
                (Some(ring.request_id), Some(ring.initiator_peer))
            } else {
                (request_id, None)
            };

            let message = if peer_str.is_some() {
                "local acoustic ring stopped".to_string()
            } else {
                "no active acoustic ring to stop".to_string()
            };

            Ok(LocateStopReport {
                ok: true,
                request_id: req_id,
                target_peer: peer_str,
                reason: clean_reason,
                message,
            })
        }
    }

    /// Ingest and process an inbound acoustic locate request from a connected peer.
    pub fn receive_locate_request(&self, peer: &DeviceId, frame: LocateRequestFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        let policy = self.locator_policy();
        if !policy.enabled {
            debug!(peer = %peer, "inbound locate request ignored: locator service disabled by policy");
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "locate_request_disabled",
                Some(peer_str.clone()),
                format!(
                    "Ignored locate request from {} because locator policy is disabled",
                    peer_str
                ),
                false,
            );
            return;
        }

        let profile = self.peer_profile(peer);
        if !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "inbound locate request rejected: peer profile not eligible under locator policy");
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "locate_request_ineligible",
                Some(peer_str.clone()),
                format!(
                    "Rejected locate request from {} due to profile scoping rules",
                    peer_str
                ),
                false,
            );
            return;
        }

        // Anti-harassment rate limiting: 10s cooldown per peer
        {
            let mut last_map = self
                .shared
                .peer_last_locate_req_ms
                .lock()
                .expect("peer_last_locate_req_ms");
            if let Some(&last_ts) = last_map.get(&peer_str) {
                if now_ms.saturating_sub(last_ts) < 10_000 {
                    warn!(peer = %peer, "inbound locate request rate-limited (<10s cooldown)");
                    self.record_audit(
                        AuditCategory::Security,
                        AuditSeverity::Warning,
                        "locate_request_rate_limited",
                        Some(peer_str.clone()),
                        format!(
                            "Rate-limited locate request from {} (<10s cooldown)",
                            peer_str
                        ),
                        false,
                    );
                    return;
                }
            }
            last_map.insert(peer_str.clone(), now_ms);
        }

        let clamped_duration =
            LocateValidator::clamp_duration(frame.duration_secs).min(policy.max_duration_secs);
        let override_silent = frame.override_silent && policy.allow_override_silent;
        let clean_msg = frame
            .message
            .as_deref()
            .map(LocateValidator::sanitize_message);

        let ring = ActiveLocateRing {
            request_id: frame.request_id.clone(),
            initiator_peer: peer_str.clone(),
            initiator_alias: alias.clone(),
            duration_secs: clamped_duration,
            pattern: frame.pattern,
            override_silent,
            message: clean_msg.clone(),
            started_at_ms: now_ms,
            expires_at_ms: now_ms + (clamped_duration as u64) * 1000,
        };

        {
            let mut active = self
                .shared
                .active_locate_ring
                .lock()
                .expect("active_locate_ring");
            *active = Some(ring);
        }

        self.emit(UiEvent::locate_ring_triggered(
            &frame.request_id,
            &peer_str,
            &alias,
            clamped_duration,
            frame.pattern.as_str(),
            override_silent,
            clean_msg.as_deref(),
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Alert,
            "locate_ring_triggered",
            Some(peer_str.clone()),
            format!(
                "Acoustic locate ring triggered by {} (pattern: {}, duration: {}s, override_silent: {})",
                alias,
                frame.pattern.as_str(),
                clamped_duration,
                override_silent
            ),
            true,
        );

        // Auto expiration task
        let shared = Arc::clone(&self.shared);
        let req_id = frame.request_id.clone();
        let wait_secs = clamped_duration;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                tokio::time::sleep(Duration::from_secs(wait_secs as u64)).await;
                let expired_ring = {
                    let mut active = shared
                        .active_locate_ring
                        .lock()
                        .expect("active_locate_ring");
                    if let Some(r) = active.as_ref() {
                        if r.request_id == req_id {
                            active.take()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(r) = expired_ring {
                    let exp_now = shared.clock.unix_ms();
                    let _ = shared.events.send(UiEvent::locate_ring_stopped(
                        Some(&r.request_id),
                        &r.initiator_peer,
                        &r.initiator_alias,
                        "duration_expired",
                        exp_now,
                    ));
                }
            });
        }
    }

    /// Ingest and process an inbound locate stop frame from a connected peer.
    pub fn receive_locate_stop(&self, peer: &DeviceId, frame: LocateStopFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());
        let clean_reason = LocateValidator::sanitize_reason(&frame.reason);

        let stopped_ring = {
            let mut active = self
                .shared
                .active_locate_ring
                .lock()
                .expect("active_locate_ring");
            if let Some(ring) = active.as_ref() {
                if frame.request_id.is_none()
                    || frame.request_id.as_deref() == Some(&ring.request_id)
                {
                    active.take()
                } else {
                    None
                }
            } else {
                None
            }
        };

        if let Some(ring) = stopped_ring {
            self.emit(UiEvent::locate_ring_stopped(
                Some(&ring.request_id),
                &ring.initiator_peer,
                &ring.initiator_alias,
                &clean_reason,
                now_ms,
            ));
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Info,
                "locate_ring_stopped_remote",
                Some(peer_str),
                format!(
                    "Acoustic locate ring {} stopped by remote peer {} (reason: '{}')",
                    ring.request_id, alias, clean_reason
                ),
                true,
            );
        }
    }
}
