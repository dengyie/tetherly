// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Focus mode policy and peer focus state.

use super::*;
use crate::events::UiEvent;
use crate::store::{save_focus_policy, FocusPolicy};
use tetherly_core::{
    CoreError, DeviceId, FocusMode, FocusStateFrame, FocusSyncReqFrame, FocusValidator,
};
use tracing::debug;

/// Status report for the local device's focus and DND state, effective status, and active quiet hours.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FocusStatusReport {
    pub ok: bool,
    pub enabled: bool,
    pub mode: FocusMode,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_secs: Option<u64>,
    pub quiet_hours_active: bool,
    pub policy: FocusPolicy,
    pub message: String,
}

/// Report for a specific peer device's focus state and active countdown.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerFocusReport {
    pub ok: bool,
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub state: FocusStateFrame,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_secs: Option<u64>,
}

/// Aggregated report of known mesh peers and their advertised focus states.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeersFocusReport {
    pub ok: bool,
    pub count: usize,
    pub peers: Vec<PeerFocusReport>,
}

/// Report returned when activating or deactivating focus mode locally and broadcasting to mesh peers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FocusSetReport {
    pub ok: bool,
    pub enabled: bool,
    pub mode: FocusMode,
    pub active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub broadcast_count: usize,
    pub message: String,
}

/// Status and governance report for focus and quiet hours policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FocusPolicyReport {
    pub ok: bool,
    pub policy: FocusPolicy,
    pub message: String,
}

impl Node {
    /// Returns the currently active focus and quiet hours policy.
    pub fn focus_policy(&self) -> FocusPolicy {
        self.shared
            .focus_policy
            .lock()
            .expect("focus_policy")
            .clone()
    }

    /// Status and configuration report for focus and quiet hours policy.
    pub fn focus_policy_report(&self) -> FocusPolicyReport {
        FocusPolicyReport {
            ok: true,
            policy: self.focus_policy(),
            message: "focus policy report".to_string(),
        }
    }

    /// Update the focus and quiet hours policy and persist to disk atomically with 0o600 permissions.
    pub fn set_focus_policy(
        &self,
        mut policy: FocusPolicy,
    ) -> Result<FocusPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_focus_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save focus_policy.json: {e}")))?;
        *self.shared.focus_policy.lock().expect("focus_policy") = policy.clone();

        self.emit(UiEvent::focus_policy_changed(
            policy.enabled,
            policy.default_mode.as_str(),
            policy.mirror_inbound_focus,
            policy.allow_emergency_breakthrough,
            policy.quiet_hours_enabled,
            policy.quiet_hours_start_minute,
            policy.quiet_hours_end_minute,
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
            AuditCategory::Focus,
            AuditSeverity::Info,
            "focus_policy_updated",
            None::<String>,
            format!(
                "Updated focus policy: enabled={}, default_mode={}, mirror={}, breakthrough={}, quiet_hours={}",
                policy.enabled,
                policy.default_mode.as_str(),
                policy.mirror_inbound_focus,
                policy.allow_emergency_breakthrough,
                policy.quiet_hours_enabled,
            ),
            true,
        );

        Ok(FocusPolicyReport {
            ok: true,
            policy,
            message: "focus policy updated successfully".to_string(),
        })
    }

    /// Returns the current focus status, combining local state, expiry, and quiet hours.
    pub fn focus_status(&self) -> FocusStatusReport {
        let now_ms = self.shared.clock.unix_ms();
        let policy = self.focus_policy();
        let minute_of_day = ((now_ms / 1000) % 86400 / 60) as u16;
        let quiet_hours_active = policy.is_quiet_hours_active(minute_of_day);

        let local_state = self
            .shared
            .local_focus_state
            .lock()
            .expect("local_focus_state")
            .clone();

        let manual_active = local_state.is_active_at(now_ms);
        let enabled = local_state.enabled;
        let mode = local_state.mode;
        let reason = local_state.reason.clone();
        let expires_ms = local_state.expires_ms;

        let overall_active = policy.enabled && (manual_active || quiet_hours_active);
        let effective_mode = if manual_active {
            mode
        } else if quiet_hours_active {
            FocusMode::Sleep
        } else {
            mode
        };

        let remaining_secs = expires_ms.and_then(|exp| {
            if exp > now_ms {
                Some((exp - now_ms) / 1000)
            } else {
                None
            }
        });

        FocusStatusReport {
            ok: true,
            enabled,
            mode: effective_mode,
            active: overall_active,
            reason,
            expires_ms,
            remaining_secs,
            quiet_hours_active,
            policy,
            message: if overall_active {
                format!("focus mode active ({:?})", effective_mode)
            } else {
                "focus mode inactive".to_string()
            },
        }
    }

    /// Set local focus state (on/off, mode, duration, reason) and optionally broadcast to mesh peers.
    pub async fn set_focus_state(
        &self,
        enabled: bool,
        mode: Option<FocusMode>,
        duration_mins: Option<u32>,
        reason: Option<String>,
        broadcast: bool,
    ) -> Result<FocusSetReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let clean_reason = reason.as_deref().map(FocusValidator::sanitize_reason);
        let policy = self.focus_policy();
        let chosen_mode = mode.unwrap_or(policy.default_mode);

        let (expires_ms, frame) = if enabled {
            let duration = duration_mins.map(FocusValidator::clamp_duration_mins);
            let exp = duration.map(|d| now_ms + (d as u64) * 60 * 1000);
            let f = FocusStateFrame {
                enabled: true,
                mode: chosen_mode,
                expires_ms: exp,
                reason: clean_reason.clone(),
                timestamp_ms: now_ms,
            };
            (exp, f)
        } else {
            let f = FocusStateFrame {
                enabled: false,
                mode: chosen_mode,
                expires_ms: None,
                reason: clean_reason.clone(),
                timestamp_ms: now_ms,
            };
            (None, f)
        };

        *self
            .shared
            .local_focus_state
            .lock()
            .expect("local_focus_state") = frame.clone();

        let active = frame.is_active_at(now_ms);
        self.emit(UiEvent::focus_state_changed(
            frame.enabled,
            frame.mode.as_str(),
            active,
            frame.reason.as_deref(),
            frame.expires_ms,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Focus,
            AuditSeverity::Info,
            if enabled {
                "focus_mode_activated"
            } else {
                "focus_mode_deactivated"
            },
            None::<String>,
            format!(
                "Focus mode {}: mode={:?}, duration={:?}, reason={:?}",
                if enabled { "activated" } else { "deactivated" },
                frame.mode,
                duration_mins,
                frame.reason
            ),
            true,
        );

        let mut broadcast_count = 0;
        if broadcast {
            if let Ok(inner) = frame.to_frame() {
                let live = self.live_peers();
                for p in live {
                    let profile = self.peer_profile(&p.device_id);
                    if policy.is_peer_eligible(&profile)
                        && self.send_to(&p.device_id, inner.clone()).await.is_ok()
                    {
                        broadcast_count += 1;
                    }
                }
            }
        }

        Ok(FocusSetReport {
            ok: true,
            enabled: frame.enabled,
            mode: frame.mode,
            active,
            expires_ms,
            reason: frame.reason,
            broadcast_count,
            message: if enabled {
                "focus mode activated".to_string()
            } else {
                "focus mode deactivated".to_string()
            },
        })
    }

    /// Returns focus state of all connected/known peers.
    pub fn peer_focus_states(&self) -> PeersFocusReport {
        let now_ms = self.shared.clock.unix_ms();
        let map = self
            .shared
            .peer_focus_states
            .lock()
            .expect("peer_focus_states");
        let mut peers = Vec::new();
        for (peer_id_str, frame) in map.iter() {
            let alias = self
                .live_peers()
                .iter()
                .find(|p| p.device_id.to_string() == *peer_id_str)
                .map(|p| p.name.clone());

            let active = frame.is_active_at(now_ms);
            let remaining_secs = frame.expires_ms.and_then(|exp| {
                if exp > now_ms {
                    Some((exp - now_ms) / 1000)
                } else {
                    None
                }
            });
            peers.push(PeerFocusReport {
                ok: true,
                peer: peer_id_str.clone(),
                peer_alias: alias,
                state: frame.clone(),
                active,
                remaining_secs,
            });
        }
        peers.sort_by(|a, b| a.peer.cmp(&b.peer));
        PeersFocusReport {
            ok: true,
            count: peers.len(),
            peers,
        }
    }

    /// Ingest and process an inbound FocusStateFrame from a connected peer.
    pub fn receive_focus_state(&self, peer: &DeviceId, frame: FocusStateFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        let policy = self.focus_policy();
        let profile = self.peer_profile(peer);

        if !policy.enabled || !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "focus state ignored: policy disabled or peer not eligible");
            return;
        }

        let active = frame.is_active_at(now_ms);
        {
            let mut map = self
                .shared
                .peer_focus_states
                .lock()
                .expect("peer_focus_states");
            map.insert(peer_str.clone(), frame.clone());
        }

        self.emit(UiEvent::peer_focus_state_changed(
            &peer_str,
            &alias,
            frame.enabled,
            frame.mode.as_str(),
            active,
            frame.reason.as_deref(),
            frame.expires_ms,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Focus,
            AuditSeverity::Info,
            "peer_focus_state_updated",
            Some(peer_str.clone()),
            format!(
                "Peer {} ({}) updated focus state: enabled={}, mode={:?}, active={}",
                alias, peer_str, frame.enabled, frame.mode, active
            ),
            true,
        );

        if policy.mirror_inbound_focus && active {
            let mut local = self
                .shared
                .local_focus_state
                .lock()
                .expect("local_focus_state");
            let should_mirror = !local.is_active_at(now_ms);
            if should_mirror {
                let mirrored_frame = FocusStateFrame {
                    enabled: true,
                    mode: frame.mode,
                    expires_ms: frame.expires_ms,
                    reason: Some(format!("mirrored from {}", alias)),
                    timestamp_ms: now_ms,
                };
                *local = mirrored_frame;
                self.emit(UiEvent::focus_state_changed(
                    true,
                    frame.mode.as_str(),
                    true,
                    Some(&format!("mirrored from {}", alias)),
                    frame.expires_ms,
                    now_ms,
                ));
            }
        }
    }

    /// Ingest and process an inbound FocusSyncReqFrame from a connected peer.
    pub fn receive_focus_sync_req(&self, peer: &DeviceId, _frame: FocusSyncReqFrame) {
        let policy = self.focus_policy();
        let profile = self.peer_profile(peer);

        if !policy.enabled || !policy.is_peer_eligible(&profile) {
            return;
        }

        let frame = self
            .shared
            .local_focus_state
            .lock()
            .expect("local_focus_state")
            .clone();

        let node = self.clone_handle();
        let target = peer.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Ok(inner) = frame.to_frame() {
                    let _ = node.send_to(&target, inner).await;
                }
            });
        }
    }

    /// Request focus state sync from all eligible peers.
    pub async fn request_peers_focus_sync(&self) -> Result<usize, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let req_frame = FocusSyncReqFrame {
            request_id: format!("req_{}", now_ms),
            timestamp_ms: now_ms,
        };
        let inner = req_frame.to_frame()?;
        let live = self.live_peers();
        let policy = self.focus_policy();
        let mut sent = 0;
        for p in live {
            let profile = self.peer_profile(&p.device_id);
            if policy.is_peer_eligible(&profile)
                && self.send_to(&p.device_id, inner.clone()).await.is_ok()
            {
                sent += 1;
            }
        }
        Ok(sent)
    }
}
