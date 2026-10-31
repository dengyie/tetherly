// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Resource handoff policy and sends.

use super::*;
use crate::events::UiEvent;
use crate::store::{save_handoff_inbox, save_handoff_policy, HandoffPolicy, TargetSelector};
use std::sync::atomic::Ordering;
use tetherly_core::frame::ResourceHandoffFrame;
use tetherly_core::{CoreError, DeviceId, HandoffEntry, HandoffStatus, ResourceHandoffValidator};
use tracing::{debug, info, warn};

/// Single target dispatch outcome for multi-peer resource handoff.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerHandoffSendItem {
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff_id: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Comprehensive aggregated report for multi-peer targeted resource handoff dispatch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MultiHandoffSendReport {
    pub ok: bool,
    pub selector: TargetSelector,
    pub url: String,
    pub title: String,
    pub target_count: usize,
    pub success_count: usize,
    pub skipped_offline_count: usize,
    pub failed_count: usize,
    pub dispatches: Vec<PeerHandoffSendItem>,
}

/// Single peer handoff send result.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandoffSendResult {
    pub ok: bool,
    pub id: String,
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub url: String,
    pub title: String,
    pub auto_open_requested: bool,
    pub timestamp_ms: u64,
}

/// Status and configuration report for resource handoff policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HandoffPolicyReport {
    pub ok: bool,
    pub policy: HandoffPolicy,
    pub message: String,
}

impl Node {
    /// Read the current inbound resource handoff policy.
    pub fn handoff_policy(&self) -> HandoffPolicy {
        self.shared
            .handoff_policy
            .lock()
            .expect("handoff_policy")
            .clone()
    }

    /// Comprehensive status and configuration report for resource handoff policy.
    pub fn handoff_policy_report(&self) -> HandoffPolicyReport {
        HandoffPolicyReport {
            ok: true,
            policy: self.handoff_policy(),
            message: "handoff policy retrieved successfully".to_string(),
        }
    }

    /// Update the resource handoff policy and persist to disk atomically.
    pub fn set_handoff_policy(
        &self,
        policy: HandoffPolicy,
    ) -> Result<HandoffPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        save_handoff_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save handoff_policy.json: {e}")))?;
        *self.shared.handoff_policy.lock().expect("handoff_policy") = policy.clone();

        self.emit(UiEvent::handoff_policy_changed(
            policy.action.as_str(),
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            policy.max_inbox_entries,
            now,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_policy_changed",
            None::<String>,
            format!(
                "Updated handoff policy: action={}, groups={:?}, tags={:?}, types={:?}, max_inbox_entries={}",
                policy.action.as_str(),
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
                policy.max_inbox_entries,
            ),
            true,
        );

        Ok(HandoffPolicyReport {
            ok: true,
            policy,
            message: "handoff policy updated successfully".to_string(),
        })
    }

    /// Dispatch a resource or URL handoff to a specific peer over the Noise session.
    pub async fn handoff_send(
        &self,
        peer_target: &str,
        raw_url: &str,
        title: Option<&str>,
        auto_open: bool,
    ) -> Result<HandoffSendResult, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let sanitized = {
            let allowlist = self.shared.allowlist.lock().expect("allowlist");
            ResourceHandoffValidator::validate(raw_url, Some(&allowlist))?
        };
        let now_ms = self.shared.clock.unix_ms();
        let seq = self.shared.handoff_seq.fetch_add(1, Ordering::Relaxed);
        let handoff_id = format!("ho_{}_{}", now_ms, seq);
        let title_str = title.unwrap_or("").trim().to_string();

        let frame = ResourceHandoffFrame {
            id: handoff_id.clone(),
            url: sanitized.url.clone(),
            title: title_str.clone(),
            auto_open_requested: auto_open,
            timestamp_ms: now_ms,
        };

        self.send_to(&peer_id, frame.to_frame()?).await?;

        let peer_str = peer_id.to_string();
        let alias = self.peer_alias(&peer_id);

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_sent",
            Some(peer_str.clone()),
            format!(
                "Sent resource handoff '{}' (host: {}) to peer '{}'",
                if title_str.is_empty() {
                    &sanitized.host
                } else {
                    &title_str
                },
                sanitized.host,
                peer_str
            ),
            true,
        );

        self.emit(UiEvent::resource_handoff_dispatched(
            &handoff_id,
            1,
            1,
            &sanitized.host,
            now_ms,
        ));

        Ok(HandoffSendResult {
            ok: true,
            id: handoff_id,
            peer: peer_str,
            peer_alias: alias,
            url: sanitized.url,
            title: title_str,
            auto_open_requested: auto_open,
            timestamp_ms: now_ms,
        })
    }

    /// Fan out a resource or URL handoff to multiple peers matching a target selector.
    pub async fn handoff_send_multi(
        &self,
        selector: &TargetSelector,
        raw_url: &str,
        title: Option<&str>,
        auto_open: bool,
    ) -> Result<MultiHandoffSendReport, CoreError> {
        let sanitized = {
            let allowlist = self.shared.allowlist.lock().expect("allowlist");
            ResourceHandoffValidator::validate(raw_url, Some(&allowlist))?
        };
        let targets = self.resolve_dispatch_targets(selector)?;
        let live = self.live_peers();
        let live_map: std::collections::HashSet<DeviceId> =
            live.into_iter().map(|p| p.device_id).collect();

        let now_ms = self.shared.clock.unix_ms();
        let title_str = title.unwrap_or("").trim().to_string();
        let mut dispatches = Vec::with_capacity(targets.len());
        let mut success_count = 0;
        let mut skipped_offline_count = 0;
        let mut failed_count = 0;

        for peer_id in &targets {
            let peer_str = peer_id.to_string();
            let alias = self.peer_alias(peer_id);

            if !live_map.contains(peer_id) {
                skipped_offline_count += 1;
                dispatches.push(PeerHandoffSendItem {
                    peer: peer_str,
                    peer_alias: alias,
                    handoff_id: None,
                    success: false,
                    error: Some("peer is offline".to_string()),
                });
                continue;
            }

            let seq = self.shared.handoff_seq.fetch_add(1, Ordering::Relaxed);
            let handoff_id = format!("ho_{}_{}", now_ms, seq);
            let frame = ResourceHandoffFrame {
                id: handoff_id.clone(),
                url: sanitized.url.clone(),
                title: title_str.clone(),
                auto_open_requested: auto_open,
                timestamp_ms: now_ms,
            };

            match frame.to_frame() {
                Ok(inner_frame) => match self.send_to(peer_id, inner_frame).await {
                    Ok(_) => {
                        success_count += 1;
                        dispatches.push(PeerHandoffSendItem {
                            peer: peer_str,
                            peer_alias: alias,
                            handoff_id: Some(handoff_id),
                            success: true,
                            error: None,
                        });
                    }
                    Err(e) => {
                        failed_count += 1;
                        dispatches.push(PeerHandoffSendItem {
                            peer: peer_str,
                            peer_alias: alias,
                            handoff_id: Some(handoff_id),
                            success: false,
                            error: Some(e.to_string()),
                        });
                    }
                },
                Err(e) => {
                    failed_count += 1;
                    dispatches.push(PeerHandoffSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        handoff_id: Some(handoff_id),
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_multi_sent",
            None::<String>,
            format!(
                "Multicast resource handoff '{}' (host: {}) via {} (targets: {}, success: {}, skipped: {}, failed: {})",
                if title_str.is_empty() { &sanitized.host } else { &title_str },
                sanitized.host,
                selector.as_label(),
                targets.len(),
                success_count,
                skipped_offline_count,
                failed_count
            ),
            failed_count == 0,
        );

        let batch_id = format!("ho_multi_{now_ms}");
        self.emit(UiEvent::resource_handoff_dispatched(
            &batch_id,
            targets.len(),
            success_count,
            &sanitized.host,
            now_ms,
        ));

        self.emit(UiEvent::multi_dispatch_completed(
            "handoff",
            selector.as_label(),
            targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            now_ms,
        ));

        Ok(MultiHandoffSendReport {
            ok: success_count > 0 || (targets.is_empty() && skipped_offline_count == 0),
            selector: selector.clone(),
            url: sanitized.url,
            title: title_str,
            target_count: targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            dispatches,
        })
    }

    /// Ingest and process an inbound resource handoff frame from a connected peer.
    pub fn receive_resource_handoff(&self, peer: &DeviceId, frame: ResourceHandoffFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        // 1. Strict URL validator firewall check
        let sanitized = {
            let allowlist = self.shared.allowlist.lock().expect("allowlist");
            match ResourceHandoffValidator::validate(&frame.url, Some(&allowlist)) {
                Ok(s) => s,
                Err(e) => {
                    warn!(peer = %peer, error = %e, "rejected invalid inbound resource handoff URL");
                    self.record_audit(
                        AuditCategory::Peer,
                        AuditSeverity::Warning,
                        "handoff_rejected_invalid_url",
                        Some(peer_str.clone()),
                        format!("Blocked inbound handoff URL from {peer}: {e}"),
                        false,
                    );
                    return;
                }
            }
        };

        // 2. Evaluate ingress policy
        let policy = self.handoff_policy();
        if !policy.action.is_enabled() {
            debug!(peer = %peer, "inbound handoff ignored: policy is disabled");
            self.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Notice,
                "handoff_disabled_rejected",
                Some(peer_str.clone()),
                format!("Rejected inbound handoff from {peer} due to disabled policy"),
                false,
            );
            return;
        }

        // 3. Check auto-open eligibility
        let profile = self.peer_profile(peer);
        let auto_open_eligible = policy.is_peer_eligible_for_auto_open(&profile);
        let should_auto_open =
            frame.auto_open_requested && policy.action.allows_auto_open() && auto_open_eligible;

        let (status, opened_ms) = if should_auto_open {
            let opener = self.shared.opener.lock().expect("opener");
            match opener.open(&sanitized.url) {
                Ok(()) => {
                    info!(peer = %peer, host = %sanitized.host, "auto-opened inbound resource handoff");
                    (HandoffStatus::Opened, Some(now_ms))
                }
                Err(e) => {
                    warn!(peer = %peer, host = %sanitized.host, error = %e, "failed to auto-open URL, saving as pending");
                    (HandoffStatus::Pending, None)
                }
            }
        } else {
            (HandoffStatus::Pending, None)
        };

        let title = if frame.title.trim().is_empty() {
            sanitized.host.clone()
        } else {
            frame.title.trim().to_string()
        };

        let entry = HandoffEntry {
            id: frame.id.clone(),
            sender_id: peer_str.clone(),
            sender_alias: alias.clone(),
            url: sanitized.url.clone(),
            title: title.clone(),
            status,
            received_ms: now_ms,
            opened_ms,
        };

        // 4. Atomic persistence in bounded inbox
        let total_count = {
            let mut inbox = self.shared.handoff_inbox.lock().expect("handoff_inbox");
            inbox.add(entry.clone(), policy.max_inbox_entries);
            let _ = save_handoff_inbox(&self.shared.cfg.data_dir, &inbox);
            inbox.entries.len()
        };

        // 5. Zero-plaintext observability emission
        self.emit(UiEvent::resource_handoff_received(
            &entry.id,
            &peer_str,
            &alias,
            &sanitized.host,
            &title,
            status.as_str(),
            should_auto_open,
            now_ms,
        ));

        self.emit(UiEvent::handoff_inbox_updated(
            "received",
            Some(&entry.id),
            total_count,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            if status == HandoffStatus::Opened {
                "handoff_received_auto_opened"
            } else {
                "handoff_received"
            },
            Some(peer_str),
            format!(
                "Received resource handoff '{}' (host: {}) from {} [status={}]",
                title,
                sanitized.host,
                alias,
                status.as_str()
            ),
            true,
        );
    }

    /// Query items from the resource handoff inbox, with optional status filter.
    pub fn handoff_inbox_list(&self, status_filter: Option<HandoffStatus>) -> Vec<HandoffEntry> {
        let inbox = self.shared.handoff_inbox.lock().expect("handoff_inbox");
        match status_filter {
            Some(st) => inbox
                .entries
                .iter()
                .filter(|e| e.status == st)
                .cloned()
                .collect(),
            None => inbox.entries.clone(),
        }
    }

    /// Explicitly open a pending resource handoff from the inbox using the system opener.
    pub fn handoff_inbox_open(&self, id: &str) -> Result<HandoffEntry, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let (entry_to_open, total_count) = {
            let mut inbox = self.shared.handoff_inbox.lock().expect("handoff_inbox");
            let entry = inbox
                .find_mut(id)
                .ok_or_else(|| CoreError::Json(format!("handoff entry '{id}' not found")))?;
            entry.status = HandoffStatus::Opened;
            entry.opened_ms = Some(now_ms);
            let cloned = entry.clone();
            let _ = save_handoff_inbox(&self.shared.cfg.data_dir, &inbox);
            (cloned, inbox.entries.len())
        };

        let opener = self.shared.opener.lock().expect("opener");
        opener.open(&entry_to_open.url)?;

        self.emit(UiEvent::handoff_inbox_updated(
            "opened",
            Some(id),
            total_count,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_opened",
            Some(entry_to_open.sender_id.clone()),
            format!("Opened resource handoff '{}' ({})", entry_to_open.title, id),
            true,
        );

        Ok(entry_to_open)
    }

    /// Mark a pending resource handoff entry as dismissed/ignored.
    pub fn handoff_inbox_dismiss(&self, id: &str) -> Result<HandoffEntry, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let (entry, total_count) = {
            let mut inbox = self.shared.handoff_inbox.lock().expect("handoff_inbox");
            let entry = inbox
                .find_mut(id)
                .ok_or_else(|| CoreError::Json(format!("handoff entry '{id}' not found")))?;
            entry.status = HandoffStatus::Dismissed;
            let cloned = entry.clone();
            let _ = save_handoff_inbox(&self.shared.cfg.data_dir, &inbox);
            (cloned, inbox.entries.len())
        };

        self.emit(UiEvent::handoff_inbox_updated(
            "dismissed",
            Some(id),
            total_count,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_dismissed",
            Some(entry.sender_id.clone()),
            format!("Dismissed resource handoff '{}' ({})", entry.title, id),
            true,
        );

        Ok(entry)
    }

    /// Clear all records in the resource handoff inbox.
    pub fn handoff_inbox_clear(&self) -> Result<usize, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        let cleared = {
            let mut inbox = self.shared.handoff_inbox.lock().expect("handoff_inbox");
            let count = inbox.clear();
            let _ = save_handoff_inbox(&self.shared.cfg.data_dir, &inbox);
            count
        };

        self.emit(UiEvent::handoff_inbox_updated(
            "cleared",
            None::<String>,
            0,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "handoff_inbox_cleared",
            None::<String>,
            format!("Cleared {cleared} entries from handoff inbox"),
            true,
        );

        Ok(cleared)
    }
}
