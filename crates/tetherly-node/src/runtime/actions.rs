// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Local action definitions and remote action triggers.

use super::*;
use crate::events::UiEvent;
use crate::store::{
    save_action_policy, save_local_actions, ActionAccessMode, ActionPolicy, LocalActionDefinition,
};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tetherly_core::{
    ActionItem, ActionListReqFrame, ActionListRespFrame, ActionResultFrame, ActionTriggerFrame,
    ActionValidator, CoreError, DeviceId,
};
use tracing::warn;

/// Status and configuration report for action execution policy.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActionPolicyReport {
    pub ok: bool,
    pub policy: ActionPolicy,
    pub message: String,
}

/// Report for local registered actions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocalActionReport {
    pub ok: bool,
    pub count: usize,
    pub actions: Vec<LocalActionDefinition>,
    pub message: String,
}

/// Report for cached or discovered remote peer actions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteActionReport {
    pub ok: bool,
    pub peer: String,
    pub count: usize,
    pub actions: Vec<ActionItem>,
    pub message: String,
}

/// Report returned when triggering a remote action.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActionTriggerReport {
    pub ok: bool,
    pub trigger_id: String,
    pub action_id: String,
    pub target_peer: String,
    pub status: String,
    pub message: String,
}

/// Report for recent action execution results.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ActionHistoryReport {
    pub ok: bool,
    pub count: usize,
    pub results: Vec<ActionResultFrame>,
    pub message: String,
}

impl Node {
    /// Returns the currently active action policy.
    pub fn action_policy(&self) -> ActionPolicy {
        self.shared
            .action_policy
            .lock()
            .expect("action_policy")
            .clone()
    }

    /// Status and configuration report for action execution policy.
    pub fn action_policy_report(&self) -> ActionPolicyReport {
        ActionPolicyReport {
            ok: true,
            policy: self.action_policy(),
            message: "action policy report".to_string(),
        }
    }

    /// Update the action policy and persist to disk atomically with 0o600 permissions.
    pub fn set_action_policy(
        &self,
        mut policy: ActionPolicy,
    ) -> Result<ActionPolicyReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        policy.sanitize();
        save_action_policy(&self.shared.cfg.data_dir, &policy)
            .map_err(|e| CoreError::File(format!("failed to save action_policy.json: {e}")))?;
        *self.shared.action_policy.lock().expect("action_policy") = policy.clone();

        self.emit(UiEvent::action_policy_changed(
            policy.mode.as_str(),
            policy.max_timeout_secs,
            policy.allowed_groups.clone(),
            policy.allowed_tags.clone(),
            policy
                .allowed_types
                .iter()
                .map(|t| t.as_str().to_string())
                .collect(),
            policy.expose_action_list,
            now,
        ));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "action_policy_changed",
            None::<String>,
            format!(
                "Updated action policy: mode={}, max_timeout_secs={}, expose_action_list={}, groups={:?}, tags={:?}, types={:?}",
                policy.mode.as_str(),
                policy.max_timeout_secs,
                policy.expose_action_list,
                policy.allowed_groups,
                policy.allowed_tags,
                policy.allowed_types,
            ),
            true,
        );

        Ok(ActionPolicyReport {
            ok: true,
            policy,
            message: "action policy updated successfully".to_string(),
        })
    }

    /// Returns all registered local action definitions.
    pub fn local_actions(&self) -> Vec<LocalActionDefinition> {
        self.shared
            .local_actions
            .lock()
            .expect("local_actions")
            .clone()
    }

    /// Report for local registered actions.
    pub fn local_action_report(&self) -> LocalActionReport {
        let actions = self.local_actions();
        LocalActionReport {
            ok: true,
            count: actions.len(),
            actions,
            message: "local actions report".to_string(),
        }
    }

    /// Set all local actions atomically, persisting to actions.json with 0o600 permissions.
    pub fn set_local_actions(
        &self,
        mut actions: Vec<LocalActionDefinition>,
    ) -> Result<LocalActionReport, CoreError> {
        let now = self.shared.clock.unix_ms();
        let mut seen = std::collections::HashSet::new();
        for action in &mut actions {
            action.id = ActionValidator::validate_action_id(&action.id)?;
            if !seen.insert(action.id.clone()) {
                return Err(CoreError::InvalidPayload(format!(
                    "duplicate action id: {}",
                    action.id
                )));
            }
            action.name = ActionValidator::sanitize_text(&action.name, 64);
            if action.name.is_empty() {
                action.name = action.id.clone();
            }
            action.description = action
                .description
                .as_deref()
                .map(|d| ActionValidator::sanitize_text(d, 256));
            action.icon = action
                .icon
                .as_deref()
                .map(|i| ActionValidator::sanitize_text(i, 64));
            action.timeout_secs = ActionValidator::clamp_timeout(action.timeout_secs);
            if action.command.trim().is_empty() {
                return Err(CoreError::InvalidPayload(
                    "action command cannot be empty".to_string(),
                ));
            }
        }

        save_local_actions(&self.shared.cfg.data_dir, &actions)
            .map_err(|e| CoreError::File(format!("failed to save actions.json: {e}")))?;
        *self.shared.local_actions.lock().expect("local_actions") = actions.clone();

        self.emit(UiEvent::local_actions_changed(actions.len(), now));

        self.record_audit(
            AuditCategory::Config,
            AuditSeverity::Info,
            "local_actions_updated",
            None::<String>,
            format!(
                "Updated local action table ({} actions registered)",
                actions.len()
            ),
            true,
        );

        Ok(LocalActionReport {
            ok: true,
            count: actions.len(),
            actions,
            message: "local actions updated successfully".to_string(),
        })
    }

    /// Register or update a single local action definition.
    pub fn add_local_action(
        &self,
        mut action: LocalActionDefinition,
    ) -> Result<LocalActionReport, CoreError> {
        action.id = ActionValidator::validate_action_id(&action.id)?;
        action.name = ActionValidator::sanitize_text(&action.name, 64);
        if action.name.is_empty() {
            action.name = action.id.clone();
        }
        action.description = action
            .description
            .as_deref()
            .map(|d| ActionValidator::sanitize_text(d, 256));
        action.icon = action
            .icon
            .as_deref()
            .map(|i| ActionValidator::sanitize_text(i, 64));
        action.timeout_secs = ActionValidator::clamp_timeout(action.timeout_secs);
        if action.command.trim().is_empty() {
            return Err(CoreError::InvalidPayload(
                "action command cannot be empty".to_string(),
            ));
        }

        let mut current = self.local_actions();
        if let Some(pos) = current.iter().position(|a| a.id == action.id) {
            current[pos] = action;
        } else {
            current.push(action);
        }
        self.set_local_actions(current)
    }

    /// Remove a registered local action by id.
    pub fn remove_local_action(&self, action_id: &str) -> Result<LocalActionReport, CoreError> {
        let clean_id = ActionValidator::validate_action_id(action_id)?;
        let mut current = self.local_actions();
        let initial_len = current.len();
        current.retain(|a| a.id != clean_id);
        if current.len() == initial_len {
            return Err(CoreError::InvalidPayload(format!(
                "action '{}' not found",
                clean_id
            )));
        }
        self.set_local_actions(current)
    }

    /// Get cached action items from a remote peer.
    pub fn remote_actions(&self, peer_target: &str) -> Result<RemoteActionReport, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let peer_str = peer_id.to_string();
        let cache = self
            .shared
            .peer_remote_actions
            .lock()
            .expect("peer_remote_actions");
        let actions = cache.get(&peer_str).cloned().unwrap_or_default();
        Ok(RemoteActionReport {
            ok: true,
            peer: peer_str,
            count: actions.len(),
            actions,
            message: "remote actions report".to_string(),
        })
    }

    /// Query actions from a remote peer over wire protocol.
    pub async fn query_remote_actions(
        &self,
        peer_target: &str,
    ) -> Result<RemoteActionReport, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let now_ms = self.shared.clock.unix_ms();
        let seq = self
            .shared
            .action_trigger_seq
            .fetch_add(1, Ordering::Relaxed);
        let request_id = format!("act_req_{}_{}", now_ms, seq);

        let frame = ActionListReqFrame {
            request_id,
            timestamp_ms: now_ms,
        };

        self.send_to(&peer_id, frame.to_inner_frame()?).await?;

        let peer_str = peer_id.to_string();
        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());
        self.emit(UiEvent::action_list_requested(&peer_str, &alias, now_ms));

        self.remote_actions(&peer_str)
    }

    /// Trigger execution of a predefined quick action on a remote peer.
    pub async fn trigger_remote_action(
        &self,
        peer_target: &str,
        action_id: &str,
    ) -> Result<ActionTriggerReport, CoreError> {
        let peer_id = self.resolve_peer(peer_target)?;
        let clean_action_id = ActionValidator::validate_action_id(action_id)?;
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer_id.to_string();

        // 5-second anti-harassment rate limiting per peer
        {
            let mut last_map = self
                .shared
                .peer_last_action_trigger_ms
                .lock()
                .expect("peer_last_action_trigger_ms");
            if let Some(&last_ts) = last_map.get(&peer_str) {
                if now_ms.saturating_sub(last_ts) < 5_000 {
                    let wait_secs = (5_000 - now_ms.saturating_sub(last_ts)).div_ceil(1000);
                    return Err(CoreError::InvalidPayload(format!(
                        "rate limit: please wait at least {}s before triggering action on peer '{}' again",
                        wait_secs, peer_str
                    )));
                }
            }
            last_map.insert(peer_str.clone(), now_ms);
        }

        let seq = self
            .shared
            .action_trigger_seq
            .fetch_add(1, Ordering::Relaxed);
        let trigger_id = format!("trig_{}_{}", now_ms, seq);

        let frame = ActionTriggerFrame {
            trigger_id: trigger_id.clone(),
            action_id: clean_action_id.clone(),
            timestamp_ms: now_ms,
        };

        self.send_to(&peer_id, frame.to_inner_frame()?).await?;

        let alias = self
            .peer_alias(&peer_id)
            .unwrap_or_else(|| peer_str.clone());
        self.emit(UiEvent::action_triggered(
            &trigger_id,
            &clean_action_id,
            &peer_str,
            &alias,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "action_trigger_sent",
            Some(peer_str.clone()),
            format!(
                "Sent action trigger request {} (action_id: '{}') to peer '{}'",
                trigger_id, clean_action_id, peer_str
            ),
            true,
        );

        Ok(ActionTriggerReport {
            ok: true,
            trigger_id,
            action_id: clean_action_id,
            target_peer: peer_str,
            status: "triggered".to_string(),
            message: "action trigger sent to peer".to_string(),
        })
    }

    /// Returns bounded history of recent action execution results.
    pub fn action_history(&self) -> ActionHistoryReport {
        let cache = self
            .shared
            .action_results_cache
            .lock()
            .expect("action_results_cache");
        let results = cache.iter().cloned().collect::<Vec<_>>();
        ActionHistoryReport {
            ok: true,
            count: results.len(),
            results,
            message: "action results history report".to_string(),
        }
    }

    /// Ingest and process an inbound ActionListReqFrame from a connected peer.
    pub fn receive_action_list_req(&self, peer: &DeviceId, frame: ActionListReqFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        let policy = self.action_policy();
        let profile = self.peer_profile(peer);

        let actions = if policy.mode == ActionAccessMode::Disabled
            || !policy.expose_action_list
            || !policy.is_peer_eligible(&profile)
        {
            Vec::new()
        } else {
            let local = self.local_actions();
            local
                .into_iter()
                .filter(|a| a.enabled)
                .map(|a| a.to_item())
                .collect()
        };

        self.emit(UiEvent::action_list_requested(&peer_str, &alias, now_ms));
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "action_list_requested",
            Some(peer_str.clone()),
            format!(
                "Peer {} requested action list (advertised {} actions)",
                peer_str,
                actions.len()
            ),
            true,
        );

        let resp = ActionListRespFrame {
            request_id: frame.request_id,
            actions,
            timestamp_ms: now_ms,
        };

        let node = self.clone_handle();
        let target = peer.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if let Ok(inner) = resp.to_inner_frame() {
                    let _ = node.send_to(&target, inner).await;
                }
            });
        }
    }

    /// Ingest and process an inbound ActionListRespFrame from a connected peer.
    pub fn receive_action_list_resp(&self, peer: &DeviceId, mut frame: ActionListRespFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        // Sanitize incoming action items
        for item in &mut frame.actions {
            item.id = ActionValidator::clean_identifier(&item.id, 64);
            item.name = ActionValidator::sanitize_text(&item.name, 64);
            item.description = item
                .description
                .as_deref()
                .map(|d| ActionValidator::sanitize_text(d, 256));
            item.icon = item
                .icon
                .as_deref()
                .map(|i| ActionValidator::sanitize_text(i, 64));
        }

        let count = frame.actions.len();
        {
            let mut cache = self
                .shared
                .peer_remote_actions
                .lock()
                .expect("peer_remote_actions");
            cache.insert(peer_str.clone(), frame.actions);
        }

        self.emit(UiEvent::action_list_received(
            &peer_str, &alias, count, now_ms,
        ));
        self.record_audit(
            AuditCategory::Security,
            AuditSeverity::Info,
            "action_list_received",
            Some(peer_str),
            format!("Received {} advertised actions from peer {}", count, alias),
            true,
        );
    }

    /// Ingest and process an inbound ActionTriggerFrame from a connected peer.
    pub fn receive_action_trigger(&self, peer: &DeviceId, frame: ActionTriggerFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        let policy = self.action_policy();
        let profile = self.peer_profile(peer);

        // Anti-harassment rate limit check (5s cooldown per peer)
        {
            let mut last_map = self
                .shared
                .peer_last_action_trigger_ms
                .lock()
                .expect("peer_last_action_trigger_ms");
            if let Some(&last_ts) = last_map.get(&peer_str) {
                if now_ms.saturating_sub(last_ts) < 5_000 {
                    warn!(peer = %peer, "inbound action trigger rate-limited (<5s cooldown)");
                    self.record_audit(
                        AuditCategory::Security,
                        AuditSeverity::Warning,
                        "action_trigger_rate_limited",
                        Some(peer_str.clone()),
                        format!(
                            "Rate-limited action trigger from {} (<5s cooldown)",
                            peer_str
                        ),
                        false,
                    );
                    let result_frame = ActionResultFrame {
                        trigger_id: frame.trigger_id,
                        action_id: frame.action_id,
                        success: false,
                        exit_code: None,
                        output_summary: None,
                        error_message: Some("rate limit exceeded (5s cooldown)".to_string()),
                        duration_ms: 0,
                        timestamp_ms: now_ms,
                    };
                    let node = self.clone_handle();
                    let target = peer.clone();
                    if let Ok(handle) = tokio::runtime::Handle::try_current() {
                        handle.spawn(async move {
                            if let Ok(inner) = result_frame.to_inner_frame() {
                                let _ = node.send_to(&target, inner).await;
                            }
                        });
                    }
                    return;
                }
            }
            last_map.insert(peer_str.clone(), now_ms);
        }

        // Policy admission check
        if policy.mode == ActionAccessMode::Disabled {
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "action_trigger_disabled",
                Some(peer_str.clone()),
                format!(
                    "Ignored action trigger from {} because action policy is disabled",
                    peer_str
                ),
                false,
            );
            let result_frame = ActionResultFrame {
                trigger_id: frame.trigger_id,
                action_id: frame.action_id,
                success: false,
                exit_code: None,
                output_summary: None,
                error_message: Some("action policy is disabled".to_string()),
                duration_ms: 0,
                timestamp_ms: now_ms,
            };
            let node = self.clone_handle();
            let target = peer.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Ok(inner) = result_frame.to_inner_frame() {
                        let _ = node.send_to(&target, inner).await;
                    }
                });
            }
            return;
        }

        if !policy.is_peer_eligible(&profile) {
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "action_trigger_ineligible",
                Some(peer_str.clone()),
                format!(
                    "Rejected action trigger from {} due to profile scoping rules",
                    peer_str
                ),
                false,
            );
            let result_frame = ActionResultFrame {
                trigger_id: frame.trigger_id,
                action_id: frame.action_id,
                success: false,
                exit_code: None,
                output_summary: None,
                error_message: Some("peer not eligible under action policy".to_string()),
                duration_ms: 0,
                timestamp_ms: now_ms,
            };
            let node = self.clone_handle();
            let target = peer.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Ok(inner) = result_frame.to_inner_frame() {
                        let _ = node.send_to(&target, inner).await;
                    }
                });
            }
            return;
        }

        if policy.mode == ActionAccessMode::PromptOnly {
            self.record_audit(
                AuditCategory::Security,
                AuditSeverity::Warning,
                "action_trigger_prompt_required",
                Some(peer_str.clone()),
                format!(
                    "Action trigger from {} rejected: prompt confirmation required",
                    peer_str
                ),
                false,
            );
            let result_frame = ActionResultFrame {
                trigger_id: frame.trigger_id,
                action_id: frame.action_id,
                success: false,
                exit_code: None,
                output_summary: None,
                error_message: Some(
                    "action execution requires interactive prompt confirmation".to_string(),
                ),
                duration_ms: 0,
                timestamp_ms: now_ms,
            };
            let node = self.clone_handle();
            let target = peer.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if let Ok(inner) = result_frame.to_inner_frame() {
                        let _ = node.send_to(&target, inner).await;
                    }
                });
            }
            return;
        }

        // Local action lookup
        let local_actions = self.local_actions();
        let target_action = match local_actions
            .iter()
            .find(|a| a.id == frame.action_id && a.enabled)
        {
            Some(a) => a.clone(),
            None => {
                self.record_audit(
                    AuditCategory::Security,
                    AuditSeverity::Warning,
                    "action_trigger_not_found",
                    Some(peer_str.clone()),
                    format!(
                        "Action '{}' requested by {} not found in local allowlist",
                        frame.action_id, peer_str
                    ),
                    false,
                );
                let result_frame = ActionResultFrame {
                    trigger_id: frame.trigger_id,
                    action_id: frame.action_id,
                    success: false,
                    exit_code: None,
                    output_summary: None,
                    error_message: Some("action not found in local allowlist".to_string()),
                    duration_ms: 0,
                    timestamp_ms: now_ms,
                };
                let node = self.clone_handle();
                let target = peer.clone();
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    handle.spawn(async move {
                        if let Ok(inner) = result_frame.to_inner_frame() {
                            let _ = node.send_to(&target, inner).await;
                        }
                    });
                }
                return;
            }
        };

        self.emit(UiEvent::action_triggered(
            &frame.trigger_id,
            &frame.action_id,
            &peer_str,
            &alias,
            now_ms,
        ));

        // ADR-008: Execute locally predefined command and args under timeout sandbox
        let node = self.clone_handle();
        let target = peer.clone();
        let timeout_secs = target_action
            .timeout_secs
            .min(policy.max_timeout_secs)
            .clamp(1, 30);
        let trigger_id = frame.trigger_id;
        let action_id = frame.action_id;

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let start_time = std::time::Instant::now();
                let (success, exit_code, output_summary, error_message) =
                    match run_action_command(
                        &target_action.command,
                        &target_action.args,
                        Duration::from_secs(timeout_secs as u64),
                    )
                    .await
                    {
                        Ok(Ok(output)) => {
                            let code = output.status.code();
                            let is_success = output.status.success();
                            let mut out_bytes = output.stdout;
                            out_bytes.extend_from_slice(&output.stderr);
                            let summary = String::from_utf8_lossy(&out_bytes);
                            let clean_summary = ActionValidator::sanitize_output(&summary);
                            (
                                is_success,
                                code,
                                if clean_summary.is_empty() {
                                    None
                                } else {
                                    Some(clean_summary)
                                },
                                if is_success {
                                    None
                                } else {
                                    Some(format!("exit code: {:?}", code))
                                },
                            )
                        }
                        Ok(Err(e)) => (false, None, None, Some(format!("execution failed: {}", e))),
                        Err(_) => (
                            false,
                            None,
                            None,
                            Some(format!("execution timed out after {}s", timeout_secs)),
                        ),
                    };

                let duration_ms = start_time.elapsed().as_millis() as u64;
                let exec_now = node.shared.clock.unix_ms();

                let result_frame = ActionResultFrame {
                    trigger_id: trigger_id.clone(),
                    action_id: action_id.clone(),
                    success,
                    exit_code,
                    output_summary,
                    error_message: error_message.clone(),
                    duration_ms,
                    timestamp_ms: exec_now,
                };

                node.record_audit(
                    AuditCategory::Security,
                    if success {
                        AuditSeverity::Info
                    } else {
                        AuditSeverity::Warning
                    },
                    "action_executed_locally",
                    Some(peer_str.clone()),
                    format!(
                        "Locally executed action '{}' for peer {} (success={}, duration={}ms, error={:?})",
                        action_id, peer_str, success, duration_ms, error_message
                    ),
                    success,
                );

                // Save to local history cache
                {
                    let mut cache = node
                        .shared
                        .action_results_cache
                        .lock()
                        .expect("action_results_cache");
                    if cache.len() >= 100 {
                        cache.pop_front();
                    }
                    cache.push_back(result_frame.clone());
                }

                if let Ok(inner) = result_frame.to_inner_frame() {
                    let _ = node.send_to(&target, inner).await;
                }
            });
        }
    }

    /// Ingest and process an inbound ActionResultFrame from a connected peer.
    pub fn receive_action_result(&self, peer: &DeviceId, frame: ActionResultFrame) {
        let now_ms = self.shared.clock.unix_ms();
        let peer_str = peer.to_string();
        let alias = self.peer_alias(peer).unwrap_or_else(|| peer_str.clone());

        self.emit(UiEvent::action_result_received(
            &frame.trigger_id,
            &frame.action_id,
            &peer_str,
            &alias,
            frame.success,
            frame.exit_code,
            frame.duration_ms,
            now_ms,
        ));

        self.record_audit(
            AuditCategory::Security,
            if frame.success {
                AuditSeverity::Info
            } else {
                AuditSeverity::Warning
            },
            "action_result_received",
            Some(peer_str),
            format!(
                "Action result from {} for '{}': success={}, duration={}ms, error={:?}",
                alias, frame.action_id, frame.success, frame.duration_ms, frame.error_message
            ),
            frame.success,
        );

        {
            let mut cache = self
                .shared
                .action_results_cache
                .lock()
                .expect("action_results_cache");
            if cache.len() >= 100 {
                cache.pop_front();
            }
            cache.push_back(frame);
        }
    }
}

/// Run an action command under its timeout. Without `kill_on_drop` a
/// timed-out child kept running (as SYSTEM on Windows); dropping the
/// timed-out `output()` future now kills it.
async fn run_action_command(
    command: &str,
    args: &[String],
    timeout: Duration,
) -> Result<std::io::Result<std::process::Output>, tokio::time::error::Elapsed> {
    let fut = tokio::process::Command::new(command)
        .args(args)
        .kill_on_drop(true)
        .output();
    tokio::time::timeout(timeout, fut).await
}

#[cfg(all(test, unix))]
mod tests {
    /// v0.2.49 review: the action timeout reported "timed out" but left the
    /// child running (as SYSTEM on Windows) forever. Dropping the timed-out
    /// future must kill the process.
    #[tokio::test]
    async fn timed_out_action_command_is_killed() {
        let marker = std::env::temp_dir().join(format!(
            "tetherly-action-kill-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let script = format!("sleep 2; touch '{}'", marker.display());
        let res = super::run_action_command(
            "sh",
            &["-c".to_string(), script],
            std::time::Duration::from_millis(300),
        )
        .await;
        assert!(res.is_err(), "must time out");
        tokio::time::sleep(std::time::Duration::from_millis(2800)).await;
        let survived = marker.exists();
        let _ = std::fs::remove_file(&marker);
        assert!(!survived, "timed-out child kept running");
    }
}
