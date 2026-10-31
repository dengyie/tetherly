// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Media control policy, battery and media state.

use super::*;
use crate::events::UiEvent;
use crate::store::{
    save_battery_policy, save_media_policy, BatteryPolicy, MediaControlPolicy, TargetSelector,
};
use std::sync::atomic::Ordering;
use tetherly_core::{
    BatteryStateFrame, BatteryValidator, CoreError, DeviceId, MediaCommand, MediaCommandFrame,
    MediaStateFrame, MediaValidator, PlaybackState,
};
use tracing::debug;

/// Single target dispatch outcome for multi-peer media command.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PeerMediaCommandSendItem {
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Single peer media command send result.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MediaCommandSendResult {
    pub ok: bool,
    pub id: String,
    pub peer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_alias: Option<String>,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_player: Option<String>,
    pub timestamp_ms: u64,
}

/// Comprehensive aggregated report for multi-peer targeted media command dispatch.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MultiMediaCommandReport {
    pub ok: bool,
    pub selector: TargetSelector,
    pub command: String,
    pub target_count: usize,
    pub success_count: usize,
    pub skipped_offline_count: usize,
    pub failed_count: usize,
    pub dispatches: Vec<PeerMediaCommandSendItem>,
}

/// Status and configuration report for media control policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MediaPolicyReport {
    pub ok: bool,
    pub policy: MediaControlPolicy,
    pub message: String,
}

/// Status report for current local or peer media playback state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MediaStatusReport {
    pub ok: bool,
    pub peer: Option<String>,
    pub state: Option<MediaStateFrame>,
}

/// Status report for current local or peer battery and power state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BatteryStatusReport {
    pub ok: bool,
    pub peer: Option<String>,
    pub state: Option<BatteryStateFrame>,
}

/// Status and configuration report for battery and power policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BatteryPolicyReport {
    pub ok: bool,
    pub policy: BatteryPolicy,
    pub message: String,
}

/// Status report for local battery state broadcast to online mesh peers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BatteryBroadcastReport {
    pub ok: bool,
    pub state: BatteryStateFrame,
    #[serde(alias = "sent_count")]
    pub broadcast_count: usize,
    pub message: String,
}

impl Node {
    /// Read the current inbound media control policy.
    pub fn media_policy(&self) -> MediaControlPolicy {
        self.shared
            .media_policy
            .lock()
            .expect("media_policy")
            .clone()
    }

    /// Comprehensive status and configuration report for media control policy.
    pub fn media_policy_report(&self) -> MediaPolicyReport {
        MediaPolicyReport {
            ok: true,
            policy: self.media_policy(),
            message: "media policy retrieved successfully".to_string(),
        }
    }

    /// Update the media control policy and persist to disk atomically with 0o600 permissions.
    pub fn set_media_policy(
        &self,
        policy: MediaControlPolicy,
    ) -> Result<MediaPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        save_media_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save media_policy.json: {e}")))?;
        *self.shared.media_policy.lock().expect("media_policy") = policy.clone();

        self.emit(UiEvent::media_policy_changed(
            policy.mode.as_str(),
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            policy.max_volume_limit,
            policy.allow_seek,
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "media_policy_changed",
            None::<String>,
            format!(
                "Updated media control policy: mode={}, max_volume_limit={}, allow_seek={}, groups={:?}, tags={:?}, types={:?}",
                policy.mode.as_str(),
                policy.max_volume_limit,
                policy.allow_seek,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
            ),
            true,
        );

        Ok(MediaPolicyReport {
            ok: true,
            policy,
            message: "media control policy updated successfully".to_string(),
        })
    }

    /// Query current playback state for local device or a remote peer.
    pub fn media_get_state(
        &self,
        peer_target: Option<&str>,
    ) -> Result<MediaStatusReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            let t = target.trim();
            if t.is_empty() || t == "local" || t == self.shared.identity.device_id().as_str() {
                None
            } else {
                Some(self.resolve_peer(t)?.to_string())
            }
        } else {
            None
        };

        let state = match resolved_peer.as_deref() {
            Some(peer_id) => {
                let states = self
                    .shared
                    .peer_media_states
                    .lock()
                    .expect("peer_media_states");
                states.get(peer_id).cloned()
            }
            None => {
                let st = self
                    .shared
                    .local_media_state
                    .lock()
                    .expect("local_media_state");
                Some(st.clone())
            }
        };

        Ok(MediaStatusReport {
            ok: true,
            peer: resolved_peer,
            state,
        })
    }

    /// Update local media playback state, sanitize metadata, enforce bounds, and broadcast to peers.
    pub fn media_update_local_state(
        &self,
        mut state: MediaStateFrame,
    ) -> Result<MediaStateFrame, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        state.title =
            MediaValidator::sanitize_text(&state.title, tetherly_core::MAX_MEDIA_TITLE_LEN);
        state.artist =
            MediaValidator::sanitize_text(&state.artist, tetherly_core::MAX_MEDIA_ARTIST_LEN);
        state.album =
            MediaValidator::sanitize_text(&state.album, tetherly_core::MAX_MEDIA_ALBUM_LEN);
        state.player_id =
            MediaValidator::sanitize_text(&state.player_id, tetherly_core::MAX_MEDIA_PLAYER_ID_LEN);
        if state.player_id.is_empty() {
            state.player_id = "default".to_string();
        }
        state.volume_percent = MediaValidator::clamp_volume(state.volume_percent);
        state.timestamp_ms = now_ms;

        {
            let mut local = self
                .shared
                .local_media_state
                .lock()
                .expect("local_media_state");
            *local = state.clone();
        }

        let local_id = self.shared.identity.device_id().to_string();
        let name = self.name();

        self.emit(UiEvent::media_state_changed(
            &local_id,
            &name,
            &state.player_id,
            state.state.as_str(),
            &state.title,
            &state.artist,
            state.volume_percent,
            state.muted,
            now_ms,
        ));

        // Broadcast to all live sessions so remote peers have immediate synchronized playback state
        let frame_opt = state.to_frame().ok();
        if let Some(frame) = frame_opt {
            let node = self.clone_handle();
            tokio::spawn(async move {
                let live = node.live_peers();
                for peer in live {
                    let _ = node.send_to(&peer.device_id, frame.clone()).await;
                }
            });
        }

        Ok(state)
    }

    /// Dispatch a media playback command to a specific peer over the Noise session.
    pub async fn media_send_command(
        &self,
        peer_target: &str,
        command: MediaCommand,
        target_player: Option<String>,
    ) -> Result<MediaCommandSendResult, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let now_ms = self.shared.clock.unix_ms();
        let seq = self.shared.media_cmd_seq.fetch_add(1, Ordering::Relaxed);
        let cmd_id = format!("cmd_{}_{}", now_ms, seq);
        let cmd_label = command.as_label().to_string();

        let clean_player = target_player
            .map(|p| MediaValidator::sanitize_text(&p, tetherly_core::MAX_MEDIA_PLAYER_ID_LEN));

        let frame = MediaCommandFrame {
            id: cmd_id.clone(),
            command: command.clone(),
            target_player: clean_player.clone(),
            timestamp_ms: now_ms,
        };

        self.send_to(&peer_id, frame.to_frame()?).await?;

        let peer_str = peer_id.to_string();
        let alias = self.peer_alias(&peer_id);

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "media_command_sent",
            Some(peer_str.clone()),
            format!(
                "Sent media command '{}' to peer '{}' (id: {})",
                cmd_label, peer_str, cmd_id
            ),
            true,
        );

        self.emit(UiEvent::media_command_dispatched(
            &cmd_id, &peer_str, &cmd_label, true, now_ms,
        ));

        Ok(MediaCommandSendResult {
            ok: true,
            id: cmd_id,
            peer: peer_str,
            peer_alias: alias,
            command: cmd_label,
            target_player: clean_player,
            timestamp_ms: now_ms,
        })
    }

    /// Fan out a media playback command to multiple peers matching a target selector.
    pub async fn media_send_command_multi(
        &self,
        selector: &TargetSelector,
        command: MediaCommand,
        target_player: Option<String>,
    ) -> Result<MultiMediaCommandReport, CoreError> {
        let targets = self.resolve_dispatch_targets(selector)?;
        let live = self.live_peers();
        let live_map: std::collections::HashSet<DeviceId> =
            live.into_iter().map(|p| p.device_id).collect();

        let now_ms = self.shared.clock.unix_ms();
        let cmd_label = command.as_label().to_string();
        let clean_player = target_player
            .map(|p| MediaValidator::sanitize_text(&p, tetherly_core::MAX_MEDIA_PLAYER_ID_LEN));

        let mut dispatches = Vec::with_capacity(targets.len());
        let mut success_count = 0;
        let mut skipped_offline_count = 0;
        let mut failed_count = 0;

        for peer_id in &targets {
            let peer_str = peer_id.to_string();
            let alias = self.peer_alias(peer_id);

            if !live_map.contains(peer_id) {
                skipped_offline_count += 1;
                dispatches.push(PeerMediaCommandSendItem {
                    peer: peer_str,
                    peer_alias: alias,
                    command_id: None,
                    success: false,
                    error: Some("peer is offline".to_string()),
                });
                continue;
            }

            let seq = self.shared.media_cmd_seq.fetch_add(1, Ordering::Relaxed);
            let cmd_id = format!("cmd_{}_{}", now_ms, seq);
            let frame = MediaCommandFrame {
                id: cmd_id.clone(),
                command: command.clone(),
                target_player: clean_player.clone(),
                timestamp_ms: now_ms,
            };

            match frame.to_frame() {
                Ok(inner_frame) => match self.send_to(peer_id, inner_frame).await {
                    Ok(_) => {
                        success_count += 1;
                        self.emit(UiEvent::media_command_dispatched(
                            &cmd_id, &peer_str, &cmd_label, true, now_ms,
                        ));
                        dispatches.push(PeerMediaCommandSendItem {
                            peer: peer_str,
                            peer_alias: alias,
                            command_id: Some(cmd_id),
                            success: true,
                            error: None,
                        });
                    }
                    Err(e) => {
                        failed_count += 1;
                        dispatches.push(PeerMediaCommandSendItem {
                            peer: peer_str,
                            peer_alias: alias,
                            command_id: Some(cmd_id),
                            success: false,
                            error: Some(e.to_string()),
                        });
                    }
                },
                Err(e) => {
                    failed_count += 1;
                    dispatches.push(PeerMediaCommandSendItem {
                        peer: peer_str,
                        peer_alias: alias,
                        command_id: Some(cmd_id),
                        success: false,
                        error: Some(e.to_string()),
                    });
                }
            }
        }

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "media_multi_sent",
            None::<String>,
            format!(
                "Multicast media command '{}' via {} (targets: {}, success: {}, skipped: {}, failed: {})",
                cmd_label,
                selector.as_label(),
                targets.len(),
                success_count,
                skipped_offline_count,
                failed_count
            ),
            failed_count == 0,
        );

        self.emit(UiEvent::multi_dispatch_completed(
            "media",
            selector.as_label(),
            targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            now_ms,
        ));

        Ok(MultiMediaCommandReport {
            ok: success_count > 0 || (targets.is_empty() && skipped_offline_count == 0),
            selector: selector.clone(),
            command: cmd_label,
            target_count: targets.len(),
            success_count,
            skipped_offline_count,
            failed_count,
            dispatches,
        })
    }

    /// Ingest and process an inbound media control command frame from a connected peer.
    pub fn receive_media_command(&self, peer: &DeviceId, mut frame: MediaCommandFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());
        let policy = self.media_policy();

        // 1. Ingress mode gate
        if !policy.mode.is_enabled() {
            debug!(peer = %peer, "inbound media command ignored: policy is disabled");
            self.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Notice,
                "media_command_blocked",
                Some(peer_str.clone()),
                format!("Blocked media command from {peer} due to disabled policy"),
                false,
            );
            return;
        }

        // 2. Profile eligibility check
        let profile = self.peer_profile(peer);
        if !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "inbound media command blocked: sender profile not eligible");
            self.record_audit(
                AuditCategory::Peer,
                AuditSeverity::Warning,
                "media_command_blocked",
                Some(peer_str.clone()),
                format!(
                    "Blocked media command from {peer}: sender profile not eligible under policy"
                ),
                false,
            );
            return;
        }

        // 3. Command specific guards (seek disallowed, volume clamping)
        match &mut frame.command {
            MediaCommand::Seek { .. } => {
                if !policy.allow_seek {
                    debug!(peer = %peer, "inbound media seek command blocked: seek is disabled by policy");
                    self.record_audit(
                        AuditCategory::Peer,
                        AuditSeverity::Notice,
                        "media_command_blocked",
                        Some(peer_str.clone()),
                        format!("Blocked media seek command from {peer}: seek disabled by policy"),
                        false,
                    );
                    return;
                }
            }
            MediaCommand::SetVolume {
                ref mut volume_percent,
            } => {
                let clamped = policy.clamp_volume(*volume_percent);
                *volume_percent = clamped;
            }
            _ => {}
        }

        let cmd_label = frame.command.as_label().to_string();

        // 4. Update local player state machine
        let updated_state = {
            let mut state = self
                .shared
                .local_media_state
                .lock()
                .expect("local_media_state");
            match &frame.command {
                MediaCommand::Play => {
                    state.state = PlaybackState::Playing;
                    state.can_play = false;
                    state.can_pause = true;
                }
                MediaCommand::Pause => {
                    state.state = PlaybackState::Paused;
                    state.can_play = true;
                    state.can_pause = false;
                }
                MediaCommand::PlayPause => {
                    if state.state == PlaybackState::Playing {
                        state.state = PlaybackState::Paused;
                        state.can_play = true;
                        state.can_pause = false;
                    } else {
                        state.state = PlaybackState::Playing;
                        state.can_play = false;
                        state.can_pause = true;
                    }
                }
                MediaCommand::Stop => {
                    state.state = PlaybackState::Stopped;
                    state.position_ms = 0;
                    state.can_play = true;
                    state.can_pause = false;
                }
                MediaCommand::Next => {
                    state.position_ms = 0;
                }
                MediaCommand::Previous => {
                    state.position_ms = 0;
                }
                MediaCommand::Seek { offset_ms } => {
                    let new_pos = (state.position_ms as i64 + offset_ms).max(0) as u64;
                    state.position_ms = if state.duration_ms > 0 {
                        new_pos.min(state.duration_ms)
                    } else {
                        new_pos
                    };
                }
                MediaCommand::SetVolume { volume_percent } => {
                    state.volume_percent = *volume_percent;
                    if *volume_percent > 0 && state.muted {
                        state.muted = false;
                    }
                }
                MediaCommand::ToggleMute => {
                    state.muted = !state.muted;
                }
            }
            state.timestamp_ms = now_ms;
            state.clone()
        };

        // 5. Zero-Plaintext observability events
        self.emit(UiEvent::media_command_received(
            &frame.id,
            &peer_str,
            &alias,
            &cmd_label,
            frame.target_player.as_deref(),
            now_ms,
        ));

        let local_id = self.shared.identity.device_id().to_string();
        let local_name = self.name();

        self.emit(UiEvent::media_state_changed(
            &local_id,
            &local_name,
            &updated_state.player_id,
            updated_state.state.as_str(),
            &updated_state.title,
            &updated_state.artist,
            updated_state.volume_percent,
            updated_state.muted,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Peer,
            AuditSeverity::Info,
            "media_command_executed",
            Some(peer_str),
            format!(
                "Executed media command '{}' from {} (state: {})",
                cmd_label,
                alias,
                updated_state.state.as_str()
            ),
            true,
        );
    }

    /// Ingest and cache an inbound media playback state frame from a connected peer.
    pub fn receive_media_state(&self, peer: &DeviceId, mut frame: MediaStateFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.title =
            MediaValidator::sanitize_text(&frame.title, tetherly_core::MAX_MEDIA_TITLE_LEN);
        frame.artist =
            MediaValidator::sanitize_text(&frame.artist, tetherly_core::MAX_MEDIA_ARTIST_LEN);
        frame.album =
            MediaValidator::sanitize_text(&frame.album, tetherly_core::MAX_MEDIA_ALBUM_LEN);
        frame.player_id =
            MediaValidator::sanitize_text(&frame.player_id, tetherly_core::MAX_MEDIA_PLAYER_ID_LEN);
        if frame.player_id.is_empty() {
            frame.player_id = "default".to_string();
        }
        frame.volume_percent = MediaValidator::clamp_volume(frame.volume_percent);
        frame.timestamp_ms = now_ms;

        {
            let mut states = self
                .shared
                .peer_media_states
                .lock()
                .expect("peer_media_states");
            states.insert(peer_str.clone(), frame.clone());
        }

        self.emit(UiEvent::media_state_changed(
            &peer_str,
            &alias,
            &frame.player_id,
            frame.state.as_str(),
            &frame.title,
            &frame.artist,
            frame.volume_percent,
            frame.muted,
            now_ms,
        ));
    }

    /// Read the currently active battery and power management policy.
    pub fn battery_policy(&self) -> BatteryPolicy {
        self.shared
            .battery_policy
            .lock()
            .expect("battery_policy")
            .clone()
    }

    /// Status and configuration report for battery policy.
    pub fn battery_policy_report(&self) -> BatteryPolicyReport {
        BatteryPolicyReport {
            ok: true,
            policy: self.battery_policy(),
            message: "current battery policy".to_string(),
        }
    }

    /// Update and persist battery management policy with POSIX 0o600 permissions and atomic consistency.
    pub fn set_battery_policy(
        &self,
        mut policy: BatteryPolicy,
    ) -> Result<BatteryPolicyReport, CoreError> {
        policy.sanitize();
        save_battery_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save battery_policy.json: {e}")))?;
        {
            let mut p = self.shared.battery_policy.lock().expect("battery_policy");
            *p = policy.clone();
        }

        let now = self.shared.clock.unix_ms();
        self.emit(UiEvent::battery_policy_changed(
            policy.low_battery_threshold,
            policy.critical_battery_threshold,
            policy.alert_on_low_battery,
            policy.auto_throttle_on_battery_saver,
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
            "battery_policy_changed",
            None::<String>,
            format!(
                "Updated battery policy: low_threshold={}, critical_threshold={}, alert={}, throttle={}, groups={:?}, tags={:?}, types={:?}",
                policy.low_battery_threshold,
                policy.critical_battery_threshold,
                policy.alert_on_low_battery,
                policy.auto_throttle_on_battery_saver,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
            ),
            true,
        );

        Ok(BatteryPolicyReport {
            ok: true,
            policy,
            message: "battery policy updated successfully".to_string(),
        })
    }

    /// Query current battery and power state for local device or a remote peer.
    pub fn battery_get_state(
        &self,
        peer_target: Option<&str>,
    ) -> Result<BatteryStatusReport, CoreError> {
        let resolved_peer = if let Some(target) = peer_target {
            let t = target.trim();
            if t.is_empty() || t == "local" || t == self.shared.identity.device_id().as_str() {
                None
            } else {
                Some(self.resolve_peer(t)?.to_string())
            }
        } else {
            None
        };

        let state = match resolved_peer.as_deref() {
            Some(peer_id) => {
                let states = self
                    .shared
                    .peer_battery_states
                    .lock()
                    .expect("peer_battery_states");
                states.get(peer_id).cloned()
            }
            None => {
                let st = self
                    .shared
                    .local_battery_state
                    .lock()
                    .expect("local_battery_state");
                Some(st.clone())
            }
        };

        Ok(BatteryStatusReport {
            ok: true,
            peer: resolved_peer,
            state,
        })
    }

    /// Update local battery state, clamp bounds, and broadcast TYPE_BATTERY_STATE to all online mesh peers.
    pub async fn battery_update_local_state(
        &self,
        mut state: BatteryStateFrame,
    ) -> Result<BatteryBroadcastReport, CoreError> {
        let now_ms = self.shared.clock.unix_ms();
        state.battery_level = BatteryValidator::clamp_level(state.battery_level);
        state.health_percent = BatteryValidator::clamp_health(state.health_percent);
        state.timestamp_ms = now_ms;

        {
            let mut local = self
                .shared
                .local_battery_state
                .lock()
                .expect("local_battery_state");
            *local = state.clone();
        }

        let local_id = self.shared.identity.device_id().to_string();
        let name = self.name();

        self.emit(UiEvent::battery_state_changed(
            &local_id,
            &name,
            state.battery_level,
            state.is_charging,
            state.power_source.as_str(),
            state.battery_saver,
            state.health_percent,
            now_ms,
        ));

        let frame = state.to_inner_frame()?;
        let live = self.live_peers();
        let mut sent_count = 0;
        for peer in live {
            if self.send_to(&peer.device_id, frame.clone()).await.is_ok() {
                sent_count += 1;
            }
        }

        Ok(BatteryBroadcastReport {
            ok: true,
            state,
            broadcast_count: sent_count,
            message: format!("battery state updated and broadcasted to {sent_count} peers"),
        })
    }

    /// Ingest and cache an inbound battery state frame from a connected peer, triggering low/critical battery alerts.
    pub fn receive_battery_state(&self, peer: &DeviceId, mut frame: BatteryStateFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        frame.battery_level = BatteryValidator::clamp_level(frame.battery_level);
        frame.health_percent = BatteryValidator::clamp_health(frame.health_percent);
        frame.timestamp_ms = now_ms;

        let policy = self.battery_policy();
        let profile = self.peer_profile(peer);

        if !policy.is_peer_eligible(&profile) {
            debug!(peer = %peer, "inbound battery update ignored: sender profile not eligible under battery policy");
            return;
        }

        {
            let mut states = self
                .shared
                .peer_battery_states
                .lock()
                .expect("peer_battery_states");
            states.insert(peer_str.clone(), frame.clone());
        }

        self.emit(UiEvent::battery_state_changed(
            &peer_str,
            &alias,
            frame.battery_level,
            frame.is_charging,
            frame.power_source.as_str(),
            frame.battery_saver,
            frame.health_percent,
            now_ms,
        ));

        // Low / Critical battery alert checking when discharging
        if policy.alert_on_low_battery && !frame.is_charging {
            if frame.battery_level <= policy.critical_battery_threshold {
                self.emit(UiEvent::battery_low_alert(
                    &peer_str,
                    &alias,
                    frame.battery_level,
                    true,
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Peer,
                    AuditSeverity::Alert,
                    "battery_critical_alert",
                    Some(peer_str.clone()),
                    format!(
                        "CRITICAL battery alert for {}: level at {}% (threshold: {}%)",
                        alias, frame.battery_level, policy.critical_battery_threshold
                    ),
                    true,
                );
            } else if frame.battery_level <= policy.low_battery_threshold {
                self.emit(UiEvent::battery_low_alert(
                    &peer_str,
                    &alias,
                    frame.battery_level,
                    false,
                    now_ms,
                ));
                self.record_audit(
                    AuditCategory::Peer,
                    AuditSeverity::Warning,
                    "battery_low_alert",
                    Some(peer_str.clone()),
                    format!(
                        "Low battery alert for {}: level at {}% (threshold: {}%)",
                        alias, frame.battery_level, policy.low_battery_threshold
                    ),
                    true,
                );
            }
        }
    }
}
