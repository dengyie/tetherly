// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Clipboard policy, history and apply hooks.

use super::*;
use crate::events::UiEvent;
use crate::store::{save_clipboard_policy, ClipboardPolicy};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tetherly_core::frame::ClipSet;
use tetherly_core::ports::Clip;
use tetherly_core::{
    ClipApply, ClipDirection, ClipMetaEntry, CoreError, DeviceId, SensitiveContentGuard,
    CLIPBOARD_OTP_CLEAR_MS,
};
use tracing::debug;

/// Comprehensive status and configuration report for granular clipboard auto-sync policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClipboardPolicyReport {
    pub ok: bool,
    pub policy: ClipboardPolicy,
    pub auto_synced_out_count: u64,
    pub auto_synced_in_count: u64,
    pub sensitive_blocked_count: u64,
    pub message: String,
}

/// Status report for the node's clipboard state and statistics without leaking plaintext.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClipStatusReport {
    pub ok: bool,
    pub has_text: bool,
    pub bytes: usize,
    pub hash: Option<String>,
    pub last_seq: u64,
    pub inbound_count: u64,
    pub outbound_count: u64,
    pub history_count: usize,
    /// False on hosts with no display server (headless Linux): the OS
    /// clipboard port itself is absent, sync statistics remain observable.
    pub clipboard_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Bounded history report of clipboard synchronization events without leaking plaintext.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClipHistoryReport {
    pub ok: bool,
    pub total: usize,
    pub entries: Vec<ClipMetaEntryDto>,
}

/// Metadata entry representing a single inbound or outbound clipboard synchronization.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClipMetaEntryDto {
    pub seq: u64,
    pub source: String,
    pub direction: String,
    pub bytes: usize,
    pub hash: String,
    pub timestamp_ms: u64,
}

/// Report returned when clearing the system clipboard and resetting hub state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ClipClearReport {
    pub ok: bool,
    pub cleared_history_count: usize,
    pub timestamp_ms: u64,
    /// False when no OS clipboard exists (headless): history still resets,
    /// there is simply nothing on the display server to clear.
    pub clipboard_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Node {
    pub async fn copy_candidate(&self, id: &str) -> Result<(), CoreError> {
        let code = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.copy_otp(id, &*self.shared.clock)?
        };
        self.shared.clip.set_text(&code)?;
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.note_local_write(&code);
        }
        let clip = self.shared.clip.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(CLIPBOARD_OTP_CLEAR_MS)).await;
            if let Ok(Some(cur)) = clip.get_text() {
                if cur == code {
                    let _ = clip.set_text("");
                }
            }
        });
        Ok(())
    }

    /// Retrieve the OTP code associated with a candidate directly without
    /// copying to the system clipboard. Follows ADR-010 (on explicit user action)
    /// and never logs the returned code.
    pub fn get_candidate_otp(&self, id: &str) -> Result<String, CoreError> {
        let mut hubs = self.shared.hubs.lock().expect("hubs");
        hubs.notify.copy_otp(id, &*self.shared.clock)
    }

    /// Open the app that produced a candidate, on explicit user click. The url
    /// always comes from the allowlist; a url carried by the notification is
    /// never consulted (M4.2).
    pub fn open_candidate(
        &self,
        id: &str,
        opener: &dyn tetherly_core::Opener,
    ) -> Result<String, CoreError> {
        let app_id = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify
                .get(id)
                .map(|c| c.app_id.clone())
                .ok_or(CoreError::UnknownCandidate)?
        };
        let url = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .url_for(&app_id)
            .map(str::to_string)
            .ok_or_else(|| CoreError::OpenRefused(format!("{app_id} is not allowlisted")))?;
        opener.open(&url)?;
        Ok(url)
    }

    pub(super) fn apply_clip(
        &self,
        peer: &DeviceId,
        clip: ClipSet,
    ) -> Result<(), tetherly_net::NetError> {
        let policy = self.clipboard_policy();
        if !policy.mode.can_receive() {
            debug!(peer = %peer, "clipboard sync ignored due to receive_only/manual mode policy");
            return Ok(());
        }
        let profile = self.peer_profile(peer);
        if !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "clipboard sync rejected: sender profile not eligible under clipboard policy");
            self.record_audit(
                AuditCategory::Clipboard,
                AuditSeverity::Warning,
                "clipboard_inbound_ineligible",
                Some(peer.to_string()),
                format!(
                    "Ignored inbound clipboard sync from {} due to profile scope policy",
                    peer
                ),
                false,
            );
            return Ok(());
        }

        let now = self.shared.clock.unix_ms();
        let bytes = clip.text.as_ref().map_or(0, |s| s.len());
        let hash = clip.hash.clone();
        let seq = clip.clip_seq;
        let apply = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.apply_remote(peer, clip, now)?
        };
        match apply {
            ClipApply::Apply(text) => {
                self.shared.clip.set_text(&text)?;
                *self
                    .shared
                    .last_auto_sync_hash
                    .lock()
                    .expect("last_auto_sync_hash") = Some(hash.clone());
                self.shared
                    .clip_auto_synced_in_count
                    .fetch_add(1, Ordering::Relaxed);
                {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.clip.record_entry(ClipMetaEntry {
                        seq,
                        source: Some(peer.clone()),
                        direction: ClipDirection::Inbound,
                        bytes,
                        hash: hash.clone(),
                        timestamp_ms: now,
                    });
                }
                self.emit(UiEvent::ClipboardSynced {
                    seq,
                    source: peer.to_string(),
                    direction: "inbound".to_string(),
                    bytes,
                    hash,
                    timestamp_ms: now,
                    sent_peers: 0,
                });
                if let Some(secs) = policy.auto_clear_timeout_secs {
                    if secs > 0 {
                        let clip = self.shared.clip.clone();
                        let expected_text = text.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs(secs)).await;
                            if let Ok(Some(cur)) = clip.get_text() {
                                if cur == expected_text {
                                    let _ = clip.set_text("");
                                }
                            }
                        });
                    }
                }
            }
            ClipApply::Echo | ClipApply::RateLimited => {}
        }
        Ok(())
    }

    pub fn clip(&self) -> Arc<dyn Clip> {
        self.shared.clip.clone()
    }

    pub fn clipboard_policy(&self) -> ClipboardPolicy {
        self.shared
            .clipboard_policy
            .lock()
            .expect("clipboard_policy")
            .clone()
    }

    /// Comprehensive status and configuration report for granular clipboard auto-sync policy.
    pub fn clipboard_policy_report(&self) -> ClipboardPolicyReport {
        ClipboardPolicyReport {
            ok: true,
            policy: self.clipboard_policy(),
            auto_synced_out_count: self
                .shared
                .clip_auto_synced_out_count
                .load(Ordering::Relaxed),
            auto_synced_in_count: self
                .shared
                .clip_auto_synced_in_count
                .load(Ordering::Relaxed),
            sensitive_blocked_count: self
                .shared
                .clip_sensitive_blocked_count
                .load(Ordering::Relaxed),
            message: "clipboard policy report".to_string(),
        }
    }

    /// Update the granular clipboard policy and persist to disk atomically.
    pub fn set_clipboard_policy(
        &self,
        policy: ClipboardPolicy,
    ) -> Result<ClipboardPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        save_clipboard_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save clipboard_policy.json: {e}")))?;
        *self
            .shared
            .clipboard_policy
            .lock()
            .expect("clipboard_policy") = policy.clone();

        self.emit(UiEvent::clipboard_policy_changed(
            policy.mode.as_str(),
            policy.sensitive_guard,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            policy.max_payload_bytes,
            policy.auto_clear_timeout_secs,
            now,
        ));

        self.record_audit(
            AuditCategory::Clipboard,
            AuditSeverity::Info,
            "clipboard_policy_changed",
            None::<String>,
            format!(
                "Updated clipboard policy: mode={}, sensitive_guard={}, groups={:?}, tags={:?}, types={:?}, max_bytes={}, auto_clear={:?}",
                policy.mode.as_str(),
                policy.sensitive_guard,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
                policy.max_payload_bytes,
                policy.auto_clear_timeout_secs
            ),
            true,
        );

        Ok(ClipboardPolicyReport {
            ok: true,
            policy,
            auto_synced_out_count: self
                .shared
                .clip_auto_synced_out_count
                .load(Ordering::Relaxed),
            auto_synced_in_count: self
                .shared
                .clip_auto_synced_in_count
                .load(Ordering::Relaxed),
            sensitive_blocked_count: self
                .shared
                .clip_sensitive_blocked_count
                .load(Ordering::Relaxed),
            message: "clipboard policy updated successfully".to_string(),
        })
    }

    /// Polls system clipboard for automatic outbound synchronization.
    /// Checks mode, changes, size constraints, sensitive content guard, and profile scoping.
    /// Returns the number of eligible peers to which the clipboard text was broadcast.
    pub async fn poll_clipboard_auto_sync(&self) -> Result<usize, CoreError> {
        let policy = self.clipboard_policy();
        if !policy.mode.can_send() {
            return Ok(0);
        }

        // Skip the full read + SHA-256 while the OS change counter has not
        // moved: every 800 ms on both ends otherwise, for an unchanged
        // clipboard. The token is taken before the read so a change racing
        // the read is picked up next round.
        let token = self.shared.clip.change_token();
        {
            let mut last = self.shared.last_clip_token.lock().expect("last_clip_token");
            if !clip_read_needed(*last, token) {
                return Ok(0);
            }
            *last = token;
        }

        let text = match self.shared.clip.get_text() {
            Ok(Some(t)) if !t.is_empty() => t,
            Ok(_) => return Ok(0),
            Err(e) => {
                // Unread generation: retry it next round.
                *self.shared.last_clip_token.lock().expect("last_clip_token") = None;
                return Err(e);
            }
        };

        let hash = tetherly_core::hex_sha256(text.as_bytes());

        // Check against last synced hash
        {
            let last = self
                .shared
                .last_auto_sync_hash
                .lock()
                .expect("last_auto_sync_hash");
            if last.as_deref() == Some(&hash) {
                return Ok(0);
            }
        }

        // Also check if hub already knows this hash as the latest
        {
            let hubs = self.shared.hubs.lock().expect("hubs");
            if hubs.clip.last_hash() == Some(&hash) {
                *self
                    .shared
                    .last_auto_sync_hash
                    .lock()
                    .expect("last_auto_sync_hash") = Some(hash.clone());
                return Ok(0);
            }
        }

        let now = self.shared.clock.unix_ms();

        // 1. Max payload ceiling guard
        if text.len() > policy.max_payload_bytes {
            *self
                .shared
                .last_auto_sync_hash
                .lock()
                .expect("last_auto_sync_hash") = Some(hash.clone());
            self.shared
                .clip_sensitive_blocked_count
                .fetch_add(1, Ordering::Relaxed);
            self.emit(UiEvent::clipboard_sync_blocked(
                "payload_too_large",
                text.len(),
                &hash,
                now,
            ));
            self.record_audit(
                AuditCategory::Clipboard,
                AuditSeverity::Warning,
                "clipboard_payload_too_large",
                None::<String>,
                format!(
                    "Blocked auto-sync of large clipboard payload ({} bytes exceeds limit {})",
                    text.len(),
                    policy.max_payload_bytes
                ),
                false,
            );
            return Ok(0);
        }

        // 2. Sensitive content leak guard
        if policy.sensitive_guard {
            if let Some(reason) = SensitiveContentGuard::inspect(&text) {
                *self
                    .shared
                    .last_auto_sync_hash
                    .lock()
                    .expect("last_auto_sync_hash") = Some(hash.clone());
                self.shared
                    .clip_sensitive_blocked_count
                    .fetch_add(1, Ordering::Relaxed);
                let reason_str = reason.as_str();
                self.emit(UiEvent::clipboard_sync_blocked(
                    reason_str,
                    text.len(),
                    &hash,
                    now,
                ));
                self.record_audit(
                    AuditCategory::Clipboard,
                    AuditSeverity::Warning,
                    "sensitive_clipboard_blocked",
                    None::<String>,
                    format!(
                        "Blocked auto-sync of sensitive {} ({} bytes, sha256={})",
                        reason_str,
                        text.len(),
                        hash
                    ),
                    false,
                );
                return Ok(0);
            }
        }

        // 3. Resolve eligible peers based on profile rules
        let live_peers = self.live_peers();
        let eligible_peers: Vec<DeviceId> = live_peers
            .into_iter()
            .map(|p| p.device_id)
            .filter(|peer_id| {
                let profile = self.peer_profile(peer_id);
                policy.is_peer_eligible(&profile)
            })
            .collect();

        // Mark current hash as observed so we don't re-process
        *self
            .shared
            .last_auto_sync_hash
            .lock()
            .expect("last_auto_sync_hash") = Some(hash.clone());

        if eligible_peers.is_empty() {
            return Ok(0);
        }

        // Prepare frame
        let set = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.prepare_local(&text)?
        };
        let Some(set) = set else {
            return Ok(0);
        };

        let seq = set.clip_seq;
        let bytes = text.len();
        let frame = set.to_frame()?;
        let count = eligible_peers.len();

        for peer in &eligible_peers {
            let _ = self.send_to(peer, frame.clone()).await;
        }

        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.record_entry(ClipMetaEntry {
                seq,
                source: None,
                direction: ClipDirection::Outbound,
                bytes,
                hash: hash.clone(),
                timestamp_ms: now,
            });
        }

        self.shared
            .clip_auto_synced_out_count
            .fetch_add(1, Ordering::Relaxed);

        self.emit(UiEvent::ClipboardSynced {
            seq,
            source: "local".to_string(),
            direction: "outbound".to_string(),
            bytes,
            hash,
            timestamp_ms: now,
            sent_peers: count,
        });

        // Auto clear local clipboard if requested by policy
        if let Some(secs) = policy.auto_clear_timeout_secs {
            if secs > 0 {
                let clip = self.shared.clip.clone();
                let expected_text = text.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    if let Ok(Some(cur)) = clip.get_text() {
                        if cur == expected_text {
                            let _ = clip.set_text("");
                        }
                    }
                });
            }
        }

        Ok(count)
    }

    pub(super) fn spawn_clipboard_auto_sync(&self) {
        let node = self.clone_handle();
        tokio::spawn(async move {
            let mut stop = node.shared.shutdown.subscribe();
            loop {
                if *stop.borrow() {
                    break;
                }
                tokio::select! {
                    _ = stop.changed() => {
                        if *stop.borrow() {
                            break;
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(800)) => {
                        let _ = node.poll_clipboard_auto_sync().await;
                    }
                }
            }
        });
    }

    /// Syncs local clipboard to a specific peer or all connected peers.
    /// Returns the number of peers that were sent the frame (0 if unchanged/echo or no peers).
    pub async fn send_clipboard_to(
        &self,
        target_peer: Option<&DeviceId>,
    ) -> Result<usize, CoreError> {
        let text = self
            .shared
            .clip
            .get_text()?
            .ok_or_else(|| CoreError::Json("empty clipboard".into()))?;
        if text.len() > tetherly_core::CLIP_TEXT_MAX {
            return Err(CoreError::ClipboardTooLarge);
        }
        let now = self.shared.clock.unix_ms();
        let bytes = text.len();
        let set = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.prepare_local(&text)?
        };
        let Some(set) = set else {
            return Ok(0);
        };
        let seq = set.clip_seq;
        let hash = set.hash.clone();
        let frame = set.to_frame()?;
        let targets = self.resolve_clip_targets(target_peer)?;
        let count = targets.len();
        for p in &targets {
            let _ = self.send_to(p, frame.clone()).await;
        }
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.record_entry(ClipMetaEntry {
                seq,
                source: None,
                direction: ClipDirection::Outbound,
                bytes,
                hash: hash.clone(),
                timestamp_ms: now,
            });
        }
        *self
            .shared
            .last_auto_sync_hash
            .lock()
            .expect("last_auto_sync_hash") = Some(hash.clone());
        self.emit(UiEvent::ClipboardSynced {
            seq,
            source: "local".to_string(),
            direction: "outbound".to_string(),
            bytes,
            hash,
            timestamp_ms: now,
            sent_peers: count,
        });
        Ok(count)
    }

    /// Syncs local clipboard to all connected peers.
    pub async fn send_clipboard(&self) -> Result<(), CoreError> {
        self.send_clipboard_to(None).await.map(|_| ())
    }

    /// Explicitly broadcasts `text` to a specific peer or all connected peers.
    /// Writes `text` to the local clipboard and advances sequence even if previously sent.
    pub async fn send_clipboard_text(
        &self,
        text: &str,
        target_peer: Option<&DeviceId>,
    ) -> Result<usize, CoreError> {
        if text.len() > tetherly_core::CLIP_TEXT_MAX {
            return Err(CoreError::ClipboardTooLarge);
        }
        self.shared.clip.set_text(text)?;
        let now = self.shared.clock.unix_ms();
        let bytes = text.len();
        let set = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.force_prepare_local(text)?
        };
        let seq = set.clip_seq;
        let hash = set.hash.clone();
        let frame = set.to_frame()?;
        let targets = self.resolve_clip_targets(target_peer)?;
        let count = targets.len();
        for p in &targets {
            let _ = self.send_to(p, frame.clone()).await;
        }
        {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.record_entry(ClipMetaEntry {
                seq,
                source: None,
                direction: ClipDirection::Outbound,
                bytes,
                hash: hash.clone(),
                timestamp_ms: now,
            });
        }
        *self
            .shared
            .last_auto_sync_hash
            .lock()
            .expect("last_auto_sync_hash") = Some(hash.clone());
        self.emit(UiEvent::ClipboardSynced {
            seq,
            source: "local".to_string(),
            direction: "outbound".to_string(),
            bytes,
            hash,
            timestamp_ms: now,
            sent_peers: count,
        });
        Ok(count)
    }

    pub(super) fn resolve_clip_targets(
        &self,
        target_peer: Option<&DeviceId>,
    ) -> Result<Vec<DeviceId>, CoreError> {
        match target_peer {
            Some(target) => {
                if !self.live_peers().iter().any(|p| &p.device_id == target) {
                    return Err(CoreError::Json(format!("peer {} is not connected", target)));
                }
                Ok(vec![target.clone()])
            }
            None => Ok(self.live_peers().into_iter().map(|p| p.device_id).collect()),
        }
    }

    /// Reads the current text from the local clipboard.
    pub fn get_clipboard_text(&self) -> Result<Option<String>, CoreError> {
        self.shared.clip.get_text()
    }

    /// Inspect clipboard status, size, and synchronization statistics without leaking plaintext.
    ///
    /// An absent OS clipboard (headless Linux, no display server) is a state,
    /// not a failure: the report degrades with `clipboard_available: false`
    /// while sync statistics stay observable, instead of answering 500.
    pub fn clipboard_status(&self) -> Result<ClipStatusReport, CoreError> {
        let (has_text, bytes, clipboard_available, reason) = match self.shared.clip.get_text() {
            Ok(Some(ref t)) if !t.is_empty() => (true, t.len(), true, None),
            Ok(_) => (false, 0, true, None),
            Err(e) => (false, 0, false, Some(e.to_string())),
        };
        let hubs = self.shared.hubs.lock().expect("hubs");
        let (inbound_count, outbound_count) = hubs.clip.stats();
        let last_seq = hubs.clip.last_seq();
        let hash = hubs.clip.last_hash().map(|s| s.to_string());
        let history_count = hubs.clip.history_count();
        Ok(ClipStatusReport {
            ok: true,
            has_text,
            bytes,
            hash,
            last_seq,
            inbound_count,
            outbound_count,
            history_count,
            clipboard_available,
            reason,
        })
    }

    /// Bounded clipboard history query without leaking plaintext.
    pub fn clipboard_history(&self, limit: Option<usize>) -> ClipHistoryReport {
        let hubs = self.shared.hubs.lock().expect("hubs");
        let entries = hubs.clip.history(limit);
        let total = entries.len();
        let dtos = entries
            .into_iter()
            .map(|e| ClipMetaEntryDto {
                seq: e.seq,
                source: e
                    .source
                    .map_or_else(|| "local".to_string(), |s| s.to_string()),
                direction: match e.direction {
                    ClipDirection::Inbound => "inbound".to_string(),
                    ClipDirection::Outbound => "outbound".to_string(),
                },
                bytes: e.bytes,
                hash: e.hash,
                timestamp_ms: e.timestamp_ms,
            })
            .collect();
        ClipHistoryReport {
            ok: true,
            total,
            entries: dtos,
        }
    }

    /// Clears the system clipboard and resets hub state and history buffer.
    pub fn clear_clipboard(&self) -> Result<ClipClearReport, CoreError> {
        // A host with no OS clipboard has nothing to clear: reset the sync
        // state anyway and report the absence instead of failing.
        let clip_err = self.shared.clip.set_text("").err();
        let cleared_history_count = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.reset_state()
        };
        let now_ms = self.shared.clock.unix_ms();
        self.emit(UiEvent::ClipboardCleared {
            timestamp_ms: now_ms,
        });
        Ok(ClipClearReport {
            ok: true,
            cleared_history_count,
            timestamp_ms: now_ms,
            clipboard_available: clip_err.is_none(),
            reason: clip_err.map(|e| e.to_string()),
        })
    }
}

/// Whether the auto-sync poller must read the clipboard: always without a
/// change token, otherwise only once the token moved since the last read.
pub(crate) fn clip_read_needed(last: Option<u64>, current: Option<u64>) -> bool {
    current.is_none() || last != current
}
