// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Clipboard policy, history and apply hooks.

use super::*;
use crate::events::UiEvent;
use crate::store::{save_clipboard_policy, ClipboardPolicy};
use std::collections::HashMap;
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

const CLIPBOARD_MAX_ATTEMPTS: u8 = 3;
const CLIPBOARD_RETRY_DELAYS_MS: [u64; 2] = [800, 2_400];
const CLIPBOARD_MAX_AGE_MS: u64 = 8_000;

#[derive(Debug, Clone)]
struct ClipboardDeliveryPeer {
    attempts: u8,
    next_retry_ms: u64,
    delivered: bool,
    terminal: bool,
    last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct ClipboardDelivery {
    set: ClipSet,
    frame: tetherly_core::frame::InnerFrame,
    bytes: usize,
    created_ms: u64,
    automatic: bool,
    peers: HashMap<DeviceId, ClipboardDeliveryPeer>,
}

#[derive(Debug, Default)]
pub(crate) struct ClipboardDeliveryState {
    pending: Option<ClipboardDelivery>,
    /// A terminal failure suppresses repeated polling of an unchanged local
    /// clipboard without pretending that the frame was delivered.
    terminal_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardDeliveryStateKind {
    Delivered,
    Retrying,
    Failed,
}

impl ClipboardDeliveryState {
    fn begin(
        &mut self,
        set: ClipSet,
        frame: tetherly_core::frame::InnerFrame,
        bytes: usize,
        targets: Vec<DeviceId>,
        now_ms: u64,
        automatic: bool,
    ) {
        self.terminal_hash = None;
        self.pending = Some(ClipboardDelivery {
            set,
            frame,
            bytes,
            created_ms: now_ms,
            automatic,
            peers: targets
                .into_iter()
                .map(|peer| {
                    (
                        peer,
                        ClipboardDeliveryPeer {
                            attempts: 0,
                            next_retry_ms: now_ms,
                            delivered: false,
                            terminal: false,
                            last_error: None,
                        },
                    )
                })
                .collect(),
        });
    }

    #[allow(clippy::type_complexity)]
    fn due(
        &mut self,
        now_ms: u64,
    ) -> Option<(
        ClipSet,
        tetherly_core::frame::InnerFrame,
        Vec<(DeviceId, u8)>,
    )> {
        let pending = self.pending.as_mut()?;
        if now_ms.saturating_sub(pending.created_ms) > CLIPBOARD_MAX_AGE_MS {
            for state in pending.peers.values_mut() {
                if !state.delivered {
                    state.terminal = true;
                }
            }
        }
        let frame = pending.frame.clone();
        let set = pending.set.clone();
        let peers = pending
            .peers
            .iter_mut()
            .filter_map(|(peer, state)| {
                if state.delivered || state.terminal || state.next_retry_ms > now_ms {
                    return None;
                }
                state.attempts = state.attempts.saturating_add(1);
                Some((peer.clone(), state.attempts))
            })
            .collect();
        Some((set, frame, peers))
    }

    fn result(
        &mut self,
        peer: &DeviceId,
        success: bool,
        error: Option<String>,
        now_ms: u64,
    ) -> Option<(ClipboardDeliveryStateKind, u8, Option<u64>, Option<String>)> {
        let pending = self.pending.as_mut()?;
        let state = pending.peers.get_mut(peer)?;
        if success {
            state.delivered = true;
            state.last_error = None;
            return Some((
                ClipboardDeliveryStateKind::Delivered,
                state.attempts,
                None,
                None,
            ));
        }
        state.last_error = error.clone();
        if state.attempts >= CLIPBOARD_MAX_ATTEMPTS
            || now_ms.saturating_sub(pending.created_ms) >= CLIPBOARD_MAX_AGE_MS
        {
            state.terminal = true;
            Some((
                ClipboardDeliveryStateKind::Failed,
                state.attempts,
                None,
                error,
            ))
        } else {
            let delay = CLIPBOARD_RETRY_DELAYS_MS
                .get(state.attempts.saturating_sub(1) as usize)
                .copied()
                .unwrap_or(*CLIPBOARD_RETRY_DELAYS_MS.last().unwrap());
            state.next_retry_ms = now_ms.saturating_add(delay);
            Some((
                ClipboardDeliveryStateKind::Retrying,
                state.attempts,
                Some(state.next_retry_ms),
                error,
            ))
        }
    }

    fn finished(&self) -> Option<bool> {
        let pending = self.pending.as_ref()?;
        if pending.peers.values().all(|p| p.delivered) {
            return Some(true);
        }
        if pending.peers.values().all(|p| p.delivered || p.terminal) {
            return Some(false);
        }
        None
    }

    fn take_finished(&mut self) -> Option<ClipboardDelivery> {
        self.pending.take()
    }

    fn reset(&mut self) {
        self.pending = None;
        self.terminal_hash = None;
    }
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

    #[allow(clippy::too_many_arguments)]
    fn emit_clipboard_delivery(
        &self,
        set: &ClipSet,
        peer: &DeviceId,
        status: ClipboardDeliveryStateKind,
        attempt: u8,
        retry_at_ms: Option<u64>,
        error: Option<String>,
        timestamp_ms: u64,
    ) {
        self.emit(UiEvent::ClipboardDelivery {
            seq: set.clip_seq,
            peer: peer.to_string(),
            hash: set.hash.clone(),
            status: match status {
                ClipboardDeliveryStateKind::Delivered => "delivered",
                ClipboardDeliveryStateKind::Retrying => "retrying",
                ClipboardDeliveryStateKind::Failed => "failed",
            }
            .to_string(),
            attempt,
            retry_at_ms,
            error,
            timestamp_ms,
        });
    }

    /// Send due clipboard entries and commit the hub only after every target
    /// has accepted the frame. A failed peer is retried at most three times and
    /// for at most eight seconds; only the latest frame is retained.
    async fn drive_clipboard_delivery(&self, now_ms: u64) -> Result<usize, CoreError> {
        let Some((set, frame, peers)) = self
            .shared
            .clipboard_delivery
            .lock()
            .expect("clipboard_delivery")
            .due(now_ms)
        else {
            return Ok(0);
        };
        let mut delivered_now = 0;
        for (peer, _attempt) in peers {
            let result = self.send_to(&peer, frame.clone()).await;
            let (success, error) = match result {
                Ok(()) => (true, None),
                Err(e) => (false, Some(e.to_string())),
            };
            if success {
                delivered_now += 1;
            }
            let outcome = self
                .shared
                .clipboard_delivery
                .lock()
                .expect("clipboard_delivery")
                .result(&peer, success, error, now_ms);
            if let Some((status, attempt, retry_at_ms, error)) = outcome {
                self.emit_clipboard_delivery(
                    &set,
                    &peer,
                    status,
                    attempt,
                    retry_at_ms,
                    error,
                    now_ms,
                );
            }
        }

        let finished = self
            .shared
            .clipboard_delivery
            .lock()
            .expect("clipboard_delivery")
            .finished();
        if let Some(all_delivered) = finished {
            let delivery = self
                .shared
                .clipboard_delivery
                .lock()
                .expect("clipboard_delivery")
                .take_finished()
                .expect("finished clipboard delivery");
            if all_delivered {
                let now = self.shared.clock.unix_ms();
                let hash = delivery.set.hash.clone();
                let seq = delivery.set.clip_seq;
                {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.clip.commit_local(&delivery.set);
                    hubs.clip.record_entry(ClipMetaEntry {
                        seq,
                        source: None,
                        direction: ClipDirection::Outbound,
                        bytes: delivery.bytes,
                        hash: hash.clone(),
                        timestamp_ms: now,
                    });
                }
                *self
                    .shared
                    .last_auto_sync_hash
                    .lock()
                    .expect("last_auto_sync_hash") = Some(hash.clone());
                if delivery.automatic {
                    self.shared
                        .clip_auto_synced_out_count
                        .fetch_add(1, Ordering::Relaxed);
                }
                self.emit(UiEvent::ClipboardSynced {
                    seq,
                    source: "local".to_string(),
                    direction: "outbound".to_string(),
                    bytes: delivery.bytes,
                    hash,
                    timestamp_ms: now,
                    sent_peers: delivery.peers.len(),
                });
                if delivery.automatic {
                    if let Some(text) = delivery.set.text {
                        if let Some(secs) = self.clipboard_policy().auto_clear_timeout_secs {
                            if secs > 0 {
                                let clip = self.shared.clip.clone();
                                tokio::spawn(async move {
                                    tokio::time::sleep(Duration::from_secs(secs)).await;
                                    if let Ok(Some(cur)) = clip.get_text() {
                                        if cur == text {
                                            let _ = clip.set_text("");
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
            } else {
                self.shared
                    .clipboard_delivery
                    .lock()
                    .expect("clipboard_delivery")
                    .terminal_hash = Some(delivery.set.hash);
            }
        }
        Ok(delivered_now)
    }

    async fn begin_clipboard_delivery(
        &self,
        set: ClipSet,
        targets: Vec<DeviceId>,
        now_ms: u64,
        automatic: bool,
    ) -> Result<usize, CoreError> {
        if targets.is_empty() {
            return Ok(0);
        }
        let frame = set.to_frame()?;
        let bytes = set.text.as_ref().map_or(0, |text| text.len());
        self.shared
            .clipboard_delivery
            .lock()
            .expect("clipboard_delivery")
            .begin(set, frame, bytes, targets, now_ms, automatic);
        self.drive_clipboard_delivery(now_ms).await
    }

    /// Polls system clipboard for automatic outbound synchronization.
    /// Hash/sequence state is committed only after all eligible peers accept the
    /// frame; failed peers retain a bounded retry entry.
    pub async fn poll_clipboard_auto_sync(&self) -> Result<usize, CoreError> {
        let policy = self.clipboard_policy();
        if !policy.mode.can_send() {
            return Ok(0);
        }
        let _send_guard = self.shared.clipboard_send_lock.lock().await;
        let now = self.shared.clock.unix_ms();
        let mut delivered = self.drive_clipboard_delivery(now).await?;

        let token = self.shared.clip.change_token();
        {
            let mut last = self.shared.last_clip_token.lock().expect("last_clip_token");
            if !clip_read_needed(*last, token) {
                return Ok(delivered);
            }
            *last = token;
        }
        let text = match self.shared.clip.get_text() {
            Ok(Some(t)) if !t.is_empty() => t,
            Ok(_) => return Ok(delivered),
            Err(e) => {
                *self.shared.last_clip_token.lock().expect("last_clip_token") = None;
                return Err(e);
            }
        };
        let hash = tetherly_core::hex_sha256(text.as_bytes());
        {
            let state = self
                .shared
                .clipboard_delivery
                .lock()
                .expect("clipboard_delivery");
            if state.pending.as_ref().is_some_and(|p| p.set.hash == hash)
                || state.terminal_hash.as_deref() == Some(hash.as_str())
            {
                return Ok(delivered);
            }
        }
        {
            let last = self
                .shared
                .last_auto_sync_hash
                .lock()
                .expect("last_auto_sync_hash");
            if last.as_deref() == Some(&hash) {
                return Ok(delivered);
            }
        }
        {
            let hubs = self.shared.hubs.lock().expect("hubs");
            if hubs.clip.last_hash() == Some(&hash) {
                *self
                    .shared
                    .last_auto_sync_hash
                    .lock()
                    .expect("last_auto_sync_hash") = Some(hash);
                return Ok(delivered);
            }
        }
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
            return Ok(delivered);
        }
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
                return Ok(delivered);
            }
        }
        let eligible_peers: Vec<DeviceId> = self
            .live_peers()
            .into_iter()
            .map(|p| p.device_id)
            .filter(|peer_id| {
                self.clipboard_policy()
                    .is_peer_eligible(&self.peer_profile(peer_id))
            })
            .collect();
        if eligible_peers.is_empty() {
            return Ok(delivered);
        }
        let set = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.preview_local(&text)?
        };
        let Some(set) = set else {
            return Ok(delivered);
        };
        delivered += self
            .begin_clipboard_delivery(set, eligible_peers, now, true)
            .await?;
        Ok(delivered)
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
    /// Returns the number accepted into peer queues immediately. Failed peers
    /// stay in the bounded retry state and are never counted as delivered.
    pub async fn send_clipboard_to(
        &self,
        target_peer: Option<&DeviceId>,
    ) -> Result<usize, CoreError> {
        let text = self
            .shared
            .clip
            .get_text()?
            .ok_or_else(|| CoreError::Json("empty clipboard".into()))?;
        let targets = self.resolve_clip_targets(target_peer)?;
        let _send_guard = self.shared.clipboard_send_lock.lock().await;
        let set = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.preview_local(&text)?
        };
        let Some(set) = set else {
            return Ok(0);
        };
        self.begin_clipboard_delivery(set, targets, self.shared.clock.unix_ms(), false)
            .await
    }

    /// Syncs local clipboard to all connected peers.
    pub async fn send_clipboard(&self) -> Result<(), CoreError> {
        self.send_clipboard_to(None).await.map(|_| ())
    }

    /// Explicitly broadcasts `text` to a specific peer or all connected peers.
    /// Writes `text` locally, then uses the same bounded per-peer delivery path
    /// as automatic synchronization.
    pub async fn send_clipboard_text(
        &self,
        text: &str,
        target_peer: Option<&DeviceId>,
    ) -> Result<usize, CoreError> {
        if text.len() > tetherly_core::CLIP_TEXT_MAX {
            return Err(CoreError::ClipboardTooLarge);
        }
        self.shared.clip.set_text(text)?;
        let targets = self.resolve_clip_targets(target_peer)?;
        let _send_guard = self.shared.clipboard_send_lock.lock().await;
        let set = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            hubs.clip.preview_force_local(text)?
        };
        self.begin_clipboard_delivery(set, targets, self.shared.clock.unix_ms(), false)
            .await
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
        self.shared
            .clipboard_delivery
            .lock()
            .expect("clipboard_delivery")
            .reset();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(byte: u8) -> DeviceId {
        DeviceId::from_id_pk(&[byte; 32])
    }

    #[test]
    fn delivery_retries_only_failed_peer_and_commits_only_after_all_success() {
        let first = peer(1);
        let second = peer(2);
        let set = ClipSet::text(1, "clipboard").unwrap();
        let frame = set.to_frame().unwrap();
        let mut delivery = ClipboardDeliveryState::default();
        delivery.begin(
            set.clone(),
            frame,
            set.text.as_ref().unwrap().len(),
            vec![first.clone(), second.clone()],
            1_000,
            true,
        );

        let (_, _, due) = delivery.due(1_000).unwrap();
        assert_eq!(due.len(), 2);
        assert!(matches!(
            delivery.result(&first, true, None, 1_000),
            Some((ClipboardDeliveryStateKind::Delivered, 1, None, None))
        ));
        assert!(matches!(
            delivery.result(&second, false, Some("queue full".into()), 1_000),
            Some((
                ClipboardDeliveryStateKind::Retrying,
                1,
                Some(1_800),
                Some(_)
            ))
        ));
        assert_eq!(delivery.finished(), None);

        let (_, _, not_due) = delivery.due(1_799).unwrap();
        assert!(not_due.is_empty());
        let (_, retry_frame, retry_due) = delivery.due(1_800).unwrap();
        assert_eq!(retry_due, vec![(second.clone(), 2)]);
        assert_eq!(retry_frame, set.to_frame().unwrap());
        assert!(matches!(
            delivery.result(&second, true, None, 1_800),
            Some((ClipboardDeliveryStateKind::Delivered, 2, None, None))
        ));
        assert_eq!(delivery.finished(), Some(true));
    }

    #[test]
    fn delivery_stops_after_bounded_attempts_without_success() {
        let target = peer(3);
        let set = ClipSet::text(1, "clipboard").unwrap();
        let frame = set.to_frame().unwrap();
        let mut delivery = ClipboardDeliveryState::default();
        delivery.begin(set, frame, 9, vec![target.clone()], 0, true);

        for (now, attempt, next) in [(0, 1, 800), (800, 2, 3_200)] {
            let (_, _, due) = delivery.due(now).unwrap();
            assert_eq!(due, vec![(target.clone(), attempt)]);
            assert!(matches!(
                delivery.result(&target, false, Some("peer busy".into()), now),
                Some((ClipboardDeliveryStateKind::Retrying, a, Some(at), Some(_)))
                    if a == attempt && at == next
            ));
        }
        let (_, _, due) = delivery.due(3_200).unwrap();
        assert_eq!(due, vec![(target.clone(), 3)]);
        assert!(matches!(
            delivery.result(&target, false, Some("peer gone".into()), 3_200),
            Some((ClipboardDeliveryStateKind::Failed, 3, None, Some(_)))
        ));
        assert_eq!(delivery.finished(), Some(false));
    }

    #[test]
    fn clip_preview_does_not_commit_hash_until_delivery_finishes() {
        let mut hub = tetherly_core::ClipHub::default();
        let set = hub.preview_local("clipboard").unwrap().unwrap();
        assert_eq!(hub.last_hash(), None);
        hub.commit_local(&set);
        assert_eq!(hub.last_hash(), Some(set.hash.as_str()));
        assert_eq!(hub.last_seq(), set.clip_seq);
    }
}
