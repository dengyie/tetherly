// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Peer text messages and the offline outbox.

use super::*;
use crate::events::UiEvent;
use crate::store::TargetSelector;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use tetherly_core::frame::TextMessage;
use tetherly_core::{CoreError, DeviceId};

/// Direction of a peer direct secure message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageDirection {
    Inbound,
    Outbound,
}

impl std::fmt::Display for MessageDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageDirection::Inbound => write!(f, "inbound"),
            MessageDirection::Outbound => write!(f, "outbound"),
        }
    }
}

/// A direct message sent point-to-point to or received from a peer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMessage {
    pub id: String,
    pub peer: String,
    pub direction: MessageDirection,
    pub text: String,
    pub timestamp_ms: u64,
    pub read: bool,
}

/// In-memory bounded ring buffer for peer direct messages (default 200 items).
#[derive(Debug)]
pub struct MessageEngine {
    pub(super) max_capacity: usize,
    pub(super) messages: std::collections::VecDeque<PeerMessage>,
}

impl MessageEngine {
    pub fn new(max_capacity: usize) -> Self {
        Self {
            max_capacity: max_capacity.max(10),
            messages: std::collections::VecDeque::new(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.max_capacity
    }

    pub fn push(&mut self, msg: PeerMessage) {
        if self.messages.len() >= self.max_capacity {
            self.messages.pop_front();
        }
        self.messages.push_back(msg);
    }

    pub fn query(
        &self,
        peer: Option<&str>,
        limit: Option<usize>,
        unread_only: bool,
    ) -> (usize, usize, Vec<PeerMessage>) {
        let total = self.messages.len();
        let unread = self.messages.iter().filter(|m| !m.read).count();
        let lim = limit.unwrap_or(50).clamp(1, 500);
        let mut list: Vec<PeerMessage> = self
            .messages
            .iter()
            .rev()
            .filter(|m| {
                if let Some(p) = peer {
                    if m.peer != p {
                        return false;
                    }
                }
                if unread_only && m.read {
                    return false;
                }
                true
            })
            .take(lim)
            .cloned()
            .collect();
        // Return in chronological order
        list.reverse();
        (total, unread, list)
    }

    pub fn mark_read(&mut self, peer: Option<&str>) -> usize {
        let mut count = 0;
        for m in self.messages.iter_mut() {
            if !m.read {
                if let Some(p) = peer {
                    if m.peer != p {
                        continue;
                    }
                }
                m.read = true;
                count += 1;
            }
        }
        count
    }

    pub fn clear(&mut self, peer: Option<&str>) -> usize {
        if let Some(p) = peer {
            let before = self.messages.len();
            self.messages.retain(|m| m.peer != p);
            before - self.messages.len()
        } else {
            let count = self.messages.len();
            self.messages.clear();
            count
        }
    }
}

/// Report after sending a peer direct message.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMessageSendReport {
    pub ok: bool,
    pub message: PeerMessage,
}

/// Paginated/filtered history of peer direct messages.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMessageHistoryReport {
    pub ok: bool,
    pub total: usize,
    pub returned: usize,
    pub unread: usize,
    pub messages: Vec<PeerMessage>,
}

/// Clear confirmation report.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMessageClearReport {
    pub ok: bool,
    pub target: String,
    pub cleared_count: usize,
    pub message: String,
}

/// Status of an outbound file transfer in the node's outbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxStatus {
    InFlight,
    Completed,
    Rejected,
    Failed,
    Cancelled,
}

impl OutboxStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InFlight => "in_flight",
            Self::Completed => "completed",
            Self::Rejected => "rejected",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl std::str::FromStr for OutboxStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "in_flight" | "inflight" | "sending" => Ok(Self::InFlight),
            "completed" | "complete" | "done" => Ok(Self::Completed),
            "rejected" | "reject" => Ok(Self::Rejected),
            "failed" | "fail" | "error" => Ok(Self::Failed),
            "cancelled" | "cancel" | "canceled" => Ok(Self::Cancelled),
            other => Err(format!("unknown outbox status '{other}'")),
        }
    }
}

/// A structured outbox record tracking a sent file transfer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutboxEntry {
    pub id: String,
    pub transfer_id: String,
    pub peer: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub filename: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub status: OutboxStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_ms: Option<u64>,
    #[serde(default)]
    pub speed_bps: u64,
}

/// Bounded in-memory circular ring buffer for outbox records.
pub struct OutboxEngine {
    pub capacity: usize,
    pub entries: VecDeque<OutboxEntry>,
}

impl Default for OutboxEngine {
    fn default() -> Self {
        Self::new(200)
    }
}

impl OutboxEngine {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.clamp(10, 1000),
            entries: VecDeque::with_capacity(capacity.min(200)),
        }
    }

    pub fn push(&mut self, entry: OutboxEntry) {
        if self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    pub fn update_status(
        &mut self,
        transfer_id: &str,
        status: OutboxStatus,
        error: Option<String>,
        completed_ms: Option<u64>,
        speed_bps: Option<u64>,
    ) -> bool {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .rev()
            .find(|e| e.transfer_id == transfer_id)
        {
            entry.status = status;
            if let Some(err) = error {
                entry.error = Some(err);
            }
            if let Some(cm) = completed_ms {
                entry.completed_ms = Some(cm);
            }
            if let Some(spd) = speed_bps {
                entry.speed_bps = spd;
            }
            true
        } else {
            false
        }
    }

    pub fn query(
        &self,
        peer: Option<&str>,
        status: Option<OutboxStatus>,
        limit: Option<usize>,
    ) -> (usize, Vec<OutboxEntry>) {
        let total = self.entries.len();
        let lim = limit.unwrap_or(50).clamp(1, 500);
        let mut list: Vec<OutboxEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| {
                if let Some(p) = peer {
                    if e.peer != p && e.peer_alias.as_deref() != Some(p) {
                        return false;
                    }
                }
                if let Some(st) = status {
                    if e.status != st {
                        return false;
                    }
                }
                true
            })
            .take(lim)
            .cloned()
            .collect();
        list.reverse();
        (total, list)
    }

    pub fn clear(&mut self, peer: Option<&str>, status: Option<OutboxStatus>) -> usize {
        let before = self.entries.len();
        self.entries.retain(|e| {
            if let Some(p) = peer {
                if e.peer == p || e.peer_alias.as_deref() == Some(p) {
                    if let Some(st) = status {
                        return e.status != st;
                    }
                    return false;
                }
            } else if let Some(st) = status {
                return e.status != st;
            } else {
                return false;
            }
            true
        });
        before - self.entries.len()
    }

    pub fn get_by_id(&self, id_or_transfer_id: &str) -> Option<OutboxEntry> {
        let needle = id_or_transfer_id.trim();
        self.entries
            .iter()
            .rev()
            .find(|e| e.id == needle || e.transfer_id == needle || e.id.starts_with(needle))
            .cloned()
    }
}

/// Outbox query response report.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutboxReport {
    pub ok: bool,
    pub total: usize,
    pub returned: usize,
    pub entries: Vec<OutboxEntry>,
}

/// Report after clearing outbox records.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutboxClearReport {
    pub ok: bool,
    pub cleared_count: usize,
    pub target_peer: Option<String>,
    pub target_status: Option<OutboxStatus>,
    pub message: String,
}

/// Report after re-offering a sent file from outbox.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OutboxResendReport {
    pub ok: bool,
    pub original_id: String,
    pub new_transfer_id: String,
    pub message: String,
}

/// Single target dispatch outcome for multi-peer message fan-out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMessageSendItem {
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Comprehensive aggregated report for multi-peer targeted message dispatch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MultiMessageSendReport {
    pub ok: bool,
    pub selector: TargetSelector,
    pub target_count: usize,
    pub success_count: usize,
    pub skipped_offline_count: usize,
    pub failed_count: usize,
    pub dispatches: Vec<PeerMessageSendItem>,
}

impl Node {
    /// Send a direct secure text or note message to a connected peer.
    pub async fn send_message_to_peer(
        &self,
        peer_target: &str,
        text: &str,
    ) -> Result<PeerMessageSendReport, CoreError> {
        let peer_id = if peer_target.trim().is_empty() {
            let live = self.live_peers();
            match live.len() {
                0 => return Err(CoreError::PeerNotFound("no connected peer".to_string())),
                1 => live[0].device_id.clone(),
                _ => {
                    return Err(CoreError::PeerNotFound(
                        "multiple peers connected; specify target peer".to_string(),
                    ))
                }
            }
        } else {
            self.resolve_peer(peer_target)?
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Json("message text cannot be empty".to_string()));
        }
        if trimmed.len() > tetherly_core::frame::TEXT_MESSAGE_MAX {
            return Err(CoreError::PayloadTooLarge);
        }

        static MSG_OUT_COUNTER: AtomicU64 = AtomicU64::new(1);
        let now_ms = self.shared.clock.unix_ms();
        let msg_id = format!(
            "msg_{}_{}",
            now_ms,
            MSG_OUT_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let wire_msg = TextMessage::new(&msg_id, trimmed, now_ms)?;
        let frame = wire_msg.to_frame()?;
        self.send_to(&peer_id, frame).await?;

        let peer_str = peer_id.to_string();
        let msg = PeerMessage {
            id: msg_id.clone(),
            peer: peer_str.clone(),
            direction: MessageDirection::Outbound,
            text: trimmed.to_string(),
            timestamp_ms: now_ms,
            read: true,
        };

        {
            let mut engine = self
                .shared
                .message_engine
                .lock()
                .expect("message_engine lock");
            engine.push(msg.clone());
        }

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "message_sent",
            Some(peer_str.clone()),
            format!("Sent direct text message (length: {} bytes)", trimmed.len()),
            true,
        );

        self.emit(UiEvent::MessageSent {
            id: msg_id,
            peer: peer_str,
            len: trimmed.len(),
            timestamp_ms: now_ms,
        });

        Ok(PeerMessageSendReport {
            ok: true,
            message: msg,
        })
    }

    /// Send a direct secure text or note message to multiple targets specified by a TargetSelector.
    /// Fans out message delivery to each online peer, records Outbound messages in MessageEngine,
    /// logs audit trails and emits UiEvent::MultiDispatchCompleted without leaking plaintext.
    pub async fn send_message_multi(
        &self,
        selector: &TargetSelector,
        text: &str,
    ) -> Result<MultiMessageSendReport, CoreError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Json("message text cannot be empty".to_string()));
        }
        if trimmed.len() > tetherly_core::frame::TEXT_MESSAGE_MAX {
            return Err(CoreError::PayloadTooLarge);
        }

        let targets = self.resolve_dispatch_targets(selector)?;
        let live = self.live_peers();
        let live_map: std::collections::HashSet<DeviceId> =
            live.into_iter().map(|p| p.device_id).collect();

        static MSG_OUT_COUNTER: AtomicU64 = AtomicU64::new(1);
        let now_ms = self.shared.clock.unix_ms();

        let mut dispatches = Vec::with_capacity(targets.len());
        let mut success_count = 0;
        let mut skipped_offline_count = 0;
        let mut failed_count = 0;

        for peer_id in &targets {
            let peer_str = peer_id.to_string();
            let alias = self.peer_alias(peer_id);

            if !live_map.contains(peer_id) {
                skipped_offline_count += 1;
                dispatches.push(PeerMessageSendItem {
                    peer: peer_str,
                    peer_alias: alias,
                    message_id: None,
                    success: false,
                    error: Some("peer is offline".to_string()),
                });
                continue;
            }

            let msg_id = format!(
                "msg_{}_{}",
                now_ms,
                MSG_OUT_COUNTER.fetch_add(1, Ordering::Relaxed)
            );
            let wire_msg = TextMessage::new(&msg_id, trimmed, now_ms)?;
            let frame = wire_msg.to_frame()?;

            match self.send_to(peer_id, frame).await {
                Ok(_) => {
                    let msg = PeerMessage {
                        id: msg_id.clone(),
                        peer: peer_str.clone(),
                        direction: MessageDirection::Outbound,
                        text: trimmed.to_string(),
                        timestamp_ms: now_ms,
                        read: true,
                    };
                    {
                        let mut engine = self
                            .shared
                            .message_engine
                            .lock()
                            .expect("message_engine lock");
                        engine.push(msg);
                    }
                    self.emit(UiEvent::MessageSent {
                        id: msg_id.clone(),
                        peer: peer_str.clone(),
                        len: trimmed.len(),
                        timestamp_ms: now_ms,
                    });
                    success_count += 1;
                    dispatches.push(PeerMessageSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        message_id: Some(msg_id),
                        success: true,
                        error: None,
                    });
                }
                Err(e) => {
                    failed_count += 1;
                    dispatches.push(PeerMessageSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        message_id: Some(msg_id),
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "message_multi_sent",
            None::<String>,
            format!(
                "Multicast message (length: {} bytes) via {} (targets: {}, success: {}, skipped: {}, failed: {})",
                trimmed.len(),
                selector.as_label(),
                targets.len(),
                success_count,
                skipped_offline_count,
                failed_count
            ),
            failed_count == 0,
        );

        self.emit(UiEvent::multi_dispatch_completed(
            "message",
            selector.as_label(),
            targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            now_ms,
        ));

        Ok(MultiMessageSendReport {
            ok: success_count > 0 || (targets.is_empty() && skipped_offline_count == 0),
            selector: selector.clone(),
            target_count: targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            dispatches,
        })
    }

    /// Ingest a direct message received over the Noise connection from a peer.
    pub fn receive_message(&self, peer_str: &str, wire_msg: TextMessage) {
        let now_ms = self.shared.clock.unix_ms();
        let len = wire_msg.text.len();
        let msg = PeerMessage {
            id: wire_msg.id.clone(),
            peer: peer_str.to_string(),
            direction: MessageDirection::Inbound,
            text: wire_msg.text,
            timestamp_ms: if wire_msg.timestamp_ms > 0 {
                wire_msg.timestamp_ms
            } else {
                now_ms
            },
            read: false,
        };

        {
            let mut engine = self
                .shared
                .message_engine
                .lock()
                .expect("message_engine lock");
            engine.push(msg);
        }

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "message_received",
            Some(peer_str.to_string()),
            format!("Received direct text message (length: {} bytes)", len),
            true,
        );

        self.emit(UiEvent::MessageReceived {
            id: wire_msg.id,
            peer: peer_str.to_string(),
            len,
            timestamp_ms: now_ms,
        });
    }

    /// Query and filter direct message history, with optional unread-only filtering and mark_read flag.
    pub fn message_history(
        &self,
        peer_target: Option<&str>,
        limit: Option<usize>,
        unread_only: bool,
        mark_read: bool,
    ) -> Result<PeerMessageHistoryReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            if target.trim().is_empty() {
                None
            } else {
                Some(self.resolve_peer(target)?.to_string())
            }
        } else {
            None
        };

        let mut engine = self
            .shared
            .message_engine
            .lock()
            .expect("message_engine lock");
        let (total, unread_before, mut messages) =
            engine.query(resolved_peer.as_deref(), limit, unread_only);

        let unread = if mark_read {
            engine.mark_read(resolved_peer.as_deref());
            for m in messages.iter_mut() {
                if resolved_peer.is_none() || resolved_peer.as_deref() == Some(&m.peer) {
                    m.read = true;
                }
            }
            engine.messages.iter().filter(|m| !m.read).count()
        } else {
            unread_before
        };

        Ok(PeerMessageHistoryReport {
            ok: true,
            total,
            returned: messages.len(),
            unread,
            messages,
        })
    }

    /// Clear direct messages, optionally filtered by peer.
    pub fn clear_messages(
        &self,
        peer_target: Option<&str>,
    ) -> Result<PeerMessageClearReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            if target.trim().is_empty() {
                None
            } else {
                Some(self.resolve_peer(target)?.to_string())
            }
        } else {
            None
        };

        let cleared_count = {
            let mut engine = self
                .shared
                .message_engine
                .lock()
                .expect("message_engine lock");
            engine.clear(resolved_peer.as_deref())
        };

        let target_label = resolved_peer.clone().unwrap_or_else(|| "all".to_string());
        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Notice,
            "message_cleared",
            resolved_peer,
            format!("Cleared {cleared_count} direct messages for target: {target_label}"),
            true,
        );

        Ok(PeerMessageClearReport {
            ok: true,
            target: target_label,
            cleared_count,
            message: format!("successfully cleared {cleared_count} messages"),
        })
    }

    /// Query and filter outbox transmission history, with optional peer, status and limit filtering.
    pub fn outbox_history(
        &self,
        peer_target: Option<&str>,
        status: Option<OutboxStatus>,
        limit: Option<usize>,
    ) -> Result<OutboxReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            if target.trim().is_empty() {
                None
            } else {
                Some(self.resolve_peer(target)?.to_string())
            }
        } else {
            None
        };

        let engine = self
            .shared
            .outbox_engine
            .lock()
            .expect("outbox_engine lock");
        let (total, entries) = engine.query(resolved_peer.as_deref(), status, limit);

        Ok(OutboxReport {
            ok: true,
            total,
            returned: entries.len(),
            entries,
        })
    }

    /// Clear outbox transmission records, optionally filtered by peer and status.
    pub fn clear_outbox(
        &self,
        peer_target: Option<&str>,
        status: Option<OutboxStatus>,
    ) -> Result<OutboxClearReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            if target.trim().is_empty() {
                None
            } else {
                Some(self.resolve_peer(target)?.to_string())
            }
        } else {
            None
        };

        let cleared_count = {
            let mut engine = self
                .shared
                .outbox_engine
                .lock()
                .expect("outbox_engine lock");
            engine.clear(resolved_peer.as_deref(), status)
        };

        let target_label = resolved_peer.clone().unwrap_or_else(|| "all".to_string());
        let status_label = status
            .map(|s| s.as_str().to_string())
            .unwrap_or_else(|| "all".to_string());
        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Notice,
            "outbox_cleared",
            resolved_peer.clone(),
            format!("Cleared {cleared_count} outbox entries for peer: {target_label}, status: {status_label}"),
            true,
        );

        self.emit(UiEvent::outbox_cleared(
            cleared_count,
            resolved_peer.clone(),
            status.map(|s| s.as_str()),
            self.shared.clock.unix_ms(),
        ));

        Ok(OutboxClearReport {
            ok: true,
            cleared_count,
            target_peer: resolved_peer,
            target_status: status,
            message: format!("successfully cleared {cleared_count} outbox entries"),
        })
    }

    /// Resend an outbox file by original entry id or transfer id.
    pub async fn resend_outbox_file(
        &self,
        id_or_transfer_id: &str,
    ) -> Result<OutboxResendReport, CoreError> {
        let entry = {
            let engine = self
                .shared
                .outbox_engine
                .lock()
                .expect("outbox_engine lock");
            engine
                .get_by_id(id_or_transfer_id)
                .ok_or(CoreError::UnknownTransfer)?
        };

        let peer_id = self.resolve_peer(&entry.peer)?;

        let new_transfer_id = if let Some(ref path_str) = entry.source_path {
            let p = std::path::Path::new(path_str);
            if !p.exists() || !p.is_file() {
                return Err(CoreError::File(format!(
                    "source file '{path_str}' no longer exists on disk"
                )));
            }
            self.offer_and_send_file(&peer_id, p, Some(&entry.filename))
                .await?
        } else {
            return Err(CoreError::File(
                "cannot resend memory-only file transfer without local source file path".into(),
            ));
        };

        self.record_audit(
            AuditCategory::File,
            AuditSeverity::Notice,
            "outbox_resent",
            Some(peer_id.to_string()),
            format!(
                "Resent outbox file '{}' (original: {}, new: {})",
                entry.filename, entry.id, new_transfer_id
            ),
            true,
        );

        Ok(OutboxResendReport {
            ok: true,
            original_id: entry.id,
            new_transfer_id: new_transfer_id.clone(),
            message: format!(
                "successfully re-offered file '{}' to peer {}",
                entry.filename, entry.peer
            ),
        })
    }
}
