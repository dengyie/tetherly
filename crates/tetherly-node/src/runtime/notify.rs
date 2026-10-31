// SPDX-License-Identifier: Apache-2.0 OR MIT
//! System probe, notification banners, open allowlist and ANCS relay.

use super::*;
use crate::ancs::AncsInbound;
use crate::events::{CandidateViewDto, UiEvent};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tetherly_core::frame::{NotifyPush, NotifyReply};
use tetherly_core::{
    ancs_actions, AncsEvent, AncsIngress, AncsState, ConnectOutcome, CoreError, DeviceId,
    IngestOutcome, NotificationPayload, NotificationUrgency, Notifier, OpenAllowlist, ReplyView,
};
use tracing::{info, warn};

/// Do-Not-Disturb modes for native desktop notification banners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DndMode {
    #[default]
    Off,
    Silent,
    All,
}

/// Node configuration for desktop notification banners.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NotifyBannerConfig {
    pub banners_enabled: bool,
    pub dnd_mode: DndMode,
    pub sound_enabled: bool,
    pub mask_otp: bool,
    pub allow_otp_in_dnd: bool,
}

impl Default for NotifyBannerConfig {
    fn default() -> Self {
        Self {
            banners_enabled: true,
            dnd_mode: DndMode::Off,
            sound_enabled: true,
            mask_otp: true,
            allow_otp_in_dnd: true,
        }
    }
}

/// Status report for desktop notification banners and telemetry.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NotifyConfigReport {
    pub ok: bool,
    pub config: NotifyBannerConfig,
    pub total_dispatched: u64,
    pub total_suppressed: u64,
}

/// Result report for testing desktop notification banners.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NotifyTestReport {
    pub ok: bool,
    pub sent: bool,
    pub suppressed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppression_reason: Option<String>,
    pub message: String,
    pub title: String,
}

/// Optional payload parameters for triggering a test notification.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct NotifyTestRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<bool>,
}

/// Detailed process resource consumption metrics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ProcessMetrics {
    pub pid: u32,
    pub uptime_secs: u64,
    pub rss_bytes: u64,
    pub virt_bytes: u64,
    pub thread_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_fds: Option<u32>,
    pub cpu_time_ms: u64,
    pub memory_rss_formatted: String,
    pub memory_virt_formatted: String,
}

/// Host machine hardware and operating system profile.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HostMetrics {
    pub os: String,
    pub arch: String,
    pub num_cpus: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_memory_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_memory_formatted: Option<String>,
}

/// Comprehensive system health and resource telemetry report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SystemReport {
    pub ok: bool,
    pub node_name: String,
    pub device_id: String,
    pub process: ProcessMetrics,
    pub host: HostMetrics,
    pub timestamp_ms: u64,
}

/// Structured DTO representing one allowlist rule for ANCS notification action open.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct OpenRuleDto {
    pub app_id: String,
    pub url: String,
}

/// Lightweight status report for the ANCS (Apple Notification Center Service) ingress.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AncsReport {
    pub ok: bool,
    pub attached: bool,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peripheral: Option<String>,
    pub retry_at_ms: Option<u64>,
    pub rules: Vec<OpenRuleDto>,
    pub allowlist: Vec<String>,
}

/// Report returned by an ANCS connect attempt.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AncsConnectReport {
    pub ok: bool,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at_ms: Option<u64>,
    pub message: String,
}

/// Report returned by an ANCS disconnect or reset action.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AncsDisconnectReport {
    pub ok: bool,
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_at_ms: Option<u64>,
    pub message: String,
}

/// Report returned when an allowlist rule is added or removed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AncsRuleReport {
    pub ok: bool,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<OpenRuleDto>,
    pub total_rules: usize,
    pub timestamp_ms: u64,
}

/// Report returned when all ANCS allowlist rules are cleared.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AncsClearRulesReport {
    pub ok: bool,
    pub cleared_count: usize,
    pub total_rules: usize,
    pub timestamp_ms: u64,
}

/// Port assignments bound by the running node.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NodePorts {
    pub control: u16,
    pub file: u16,
    pub input: u16,
    pub screen: u16,
    pub ui: u16,
}

/// Lightweight health report for liveness probes, monitoring, and diagnostics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct HealthReport {
    pub ok: bool,
    pub status: String,
    pub version: String,
    pub device_id: String,
    pub name: String,
    pub platform: String,
    pub pid: u32,
    pub uptime_secs: u64,
    pub ports: NodePorts,
    pub live_peers: usize,
    pub trusted_peers: usize,
}

impl Node {
    pub fn set_sys_probe(&self, probe: Arc<dyn crate::platform::SysProbe>) {
        *self.shared.sys_probe.lock().expect("set_sys_probe lock") = probe;
    }

    pub fn sys_probe(&self) -> Arc<dyn crate::platform::SysProbe> {
        self.shared
            .sys_probe
            .lock()
            .expect("sys_probe lock")
            .clone()
    }

    pub fn system_report(&self) -> Result<SystemReport, CoreError> {
        let pid = std::process::id();
        let uptime_secs = self.shared.started_at.elapsed().as_secs();
        let probe = self.sys_probe();
        let proc_sample = probe.sample_process(pid)?;
        let host_sample = probe.sample_host()?;

        let process = ProcessMetrics {
            pid,
            uptime_secs,
            rss_bytes: proc_sample.rss_bytes,
            virt_bytes: proc_sample.virt_bytes,
            thread_count: proc_sample.thread_count,
            open_fds: proc_sample.open_fds,
            cpu_time_ms: proc_sample.cpu_time_ms,
            memory_rss_formatted: format_memory_bytes(proc_sample.rss_bytes),
            memory_virt_formatted: format_memory_bytes(proc_sample.virt_bytes),
        };

        let num_cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let host = HostMetrics {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            num_cpus,
            total_memory_bytes: host_sample.total_memory_bytes,
            total_memory_formatted: host_sample.total_memory_bytes.map(format_memory_bytes),
        };

        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let node_name = self
            .shared
            .node_name
            .lock()
            .expect("node_name lock")
            .clone();
        let device_id = self.shared.identity.device_id().to_string();

        let report = SystemReport {
            ok: true,
            node_name,
            device_id,
            process: process.clone(),
            host: host.clone(),
            timestamp_ms,
        };

        let _ = self.shared.events.send(UiEvent::system_metrics_refreshed(
            process.pid,
            process.uptime_secs,
            process.rss_bytes,
            process.virt_bytes,
            process.thread_count,
            process.open_fds,
            process.cpu_time_ms,
            host.num_cpus,
            host.total_memory_bytes,
            timestamp_ms,
        ));

        Ok(report)
    }

    pub fn set_opener(&self, opener: Arc<dyn tetherly_core::Opener>) {
        *self.shared.opener.lock().expect("set_opener lock") = opener;
    }

    pub fn opener(&self) -> Arc<dyn tetherly_core::Opener> {
        self.shared.opener.lock().expect("opener lock").clone()
    }

    pub fn set_notifier(&self, notifier: Arc<dyn Notifier>) {
        *self.shared.notifier.lock().expect("set_notifier lock") = notifier;
    }

    pub fn notifier(&self) -> Arc<dyn Notifier> {
        self.shared.notifier.lock().expect("notifier lock").clone()
    }

    pub fn notify_config(&self) -> NotifyBannerConfig {
        self.shared
            .notify_config
            .lock()
            .expect("notify_config lock")
            .clone()
    }

    pub fn notify_config_report(&self) -> NotifyConfigReport {
        let config = self.notify_config();
        let total_dispatched = self.shared.notify_dispatched_count.load(Ordering::Relaxed);
        let total_suppressed = self.shared.notify_suppressed_count.load(Ordering::Relaxed);
        NotifyConfigReport {
            ok: true,
            config,
            total_dispatched,
            total_suppressed,
        }
    }

    pub fn update_notify_config(&self, new_cfg: NotifyBannerConfig) -> NotifyConfigReport {
        {
            let mut c = self
                .shared
                .notify_config
                .lock()
                .expect("notify_config lock");
            *c = new_cfg.clone();
        }
        let dnd_str = match new_cfg.dnd_mode {
            DndMode::Off => "off",
            DndMode::Silent => "silent",
            DndMode::All => "all",
        };
        let _ = self.shared.events.send(UiEvent::notify_config_changed(
            new_cfg.banners_enabled,
            dnd_str,
            new_cfg.sound_enabled,
            new_cfg.mask_otp,
            new_cfg.allow_otp_in_dnd,
        ));
        self.notify_config_report()
    }

    pub fn dispatch_notification_banner(
        &self,
        title: &str,
        body: &str,
        app_name: Option<&str>,
        is_otp: bool,
    ) -> Result<bool, CoreError> {
        let cfg = self.notify_config();
        if !cfg.banners_enabled {
            self.shared
                .notify_suppressed_count
                .fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        }

        let should_suppress = match cfg.dnd_mode {
            DndMode::All => !(is_otp && cfg.allow_otp_in_dnd),
            DndMode::Silent | DndMode::Off => false,
        };

        if should_suppress {
            self.shared
                .notify_suppressed_count
                .fetch_add(1, Ordering::Relaxed);
            return Ok(false);
        }

        // ADR-070: Cross-device focus mode and quiet hours suppression
        let focus_status = self.focus_status();
        if focus_status.active {
            let breakthrough = is_otp && focus_status.policy.allow_emergency_breakthrough;
            if !breakthrough {
                self.shared
                    .notify_suppressed_count
                    .fetch_add(1, Ordering::Relaxed);
                self.record_audit(
                    AuditCategory::Focus,
                    AuditSeverity::Info,
                    "focus_notification_suppressed",
                    None::<String>,
                    format!(
                        "Notification '{}' suppressed due to active focus mode ({:?})",
                        title, focus_status.mode
                    ),
                    false,
                );
                return Ok(false);
            }
        }

        let play_sound = cfg.sound_enabled && cfg.dnd_mode != DndMode::Silent;

        let display_body = if is_otp && cfg.mask_otp {
            "收到短信验证码 (已在通知中心就绪)".to_string()
        } else {
            body.to_string()
        };

        let urgency = if is_otp {
            NotificationUrgency::Critical
        } else {
            NotificationUrgency::Normal
        };

        let mut payload = NotificationPayload::new(title, display_body)
            .with_sound(play_sound)
            .with_urgency(urgency);
        if let Some(app) = app_name {
            payload = payload.with_app_name(app);
        }

        let notifier = self.notifier();
        notifier.notify(&payload)?;
        self.shared
            .notify_dispatched_count
            .fetch_add(1, Ordering::Relaxed);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let _ = self.shared.events.send(UiEvent::notification_banner_sent(
            title, app_name, is_otp, now_ms,
        ));

        Ok(true)
    }

    pub fn test_notification_banner(
        &self,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Result<NotifyTestReport, CoreError> {
        let t = title.unwrap_or("Tetherly 桌面通知测试");
        let b = body.unwrap_or("这是一条来自 Tetherly 的测试通知横幅。跨设备互联已正常工作！");
        let cfg = self.notify_config();
        if !cfg.banners_enabled {
            self.shared
                .notify_suppressed_count
                .fetch_add(1, Ordering::Relaxed);
            return Ok(NotifyTestReport {
                ok: true,
                sent: false,
                suppressed: true,
                suppression_reason: Some("banners_disabled".into()),
                message: "notification suppressed: banners are disabled in config".into(),
                title: t.into(),
            });
        }

        let should_suppress = match cfg.dnd_mode {
            DndMode::All => true,
            DndMode::Silent | DndMode::Off => false,
        };

        if should_suppress {
            self.shared
                .notify_suppressed_count
                .fetch_add(1, Ordering::Relaxed);
            return Ok(NotifyTestReport {
                ok: true,
                sent: false,
                suppressed: true,
                suppression_reason: Some(format!("dnd_mode_{:?}", cfg.dnd_mode).to_lowercase()),
                message: format!(
                    "notification suppressed by Do-Not-Disturb mode: {:?}",
                    cfg.dnd_mode
                ),
                title: t.into(),
            });
        }

        // ADR-070: Cross-device focus mode and quiet hours suppression for test notifications
        let focus_status = self.focus_status();
        if focus_status.active {
            self.shared
                .notify_suppressed_count
                .fetch_add(1, Ordering::Relaxed);
            return Ok(NotifyTestReport {
                ok: true,
                sent: false,
                suppressed: true,
                suppression_reason: Some(
                    format!("focus_mode_{:?}", focus_status.mode).to_lowercase(),
                ),
                message: format!(
                    "notification suppressed by active focus mode: {:?} (quiet_hours={})",
                    focus_status.mode, focus_status.quiet_hours_active
                ),
                title: t.into(),
            });
        }

        let play_sound = cfg.sound_enabled && cfg.dnd_mode != DndMode::Silent;
        let payload = NotificationPayload::new(t, b)
            .with_app_name("Tetherly")
            .with_sound(play_sound)
            .with_urgency(NotificationUrgency::Normal);

        let notifier = self.notifier();
        notifier.notify(&payload)?;
        self.shared
            .notify_dispatched_count
            .fetch_add(1, Ordering::Relaxed);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        let _ = self.shared.events.send(UiEvent::notification_banner_sent(
            t,
            Some("Tetherly"),
            false,
            now_ms,
        ));

        Ok(NotifyTestReport {
            ok: true,
            sent: true,
            suppressed: false,
            suppression_reason: None,
            message: "test desktop notification banner dispatched successfully".into(),
            title: t.into(),
        })
    }

    pub async fn send_notify(&self, peer: &DeviceId, push: NotifyPush) -> Result<(), CoreError> {
        self.send_to(peer, push.to_frame()?).await
    }

    pub async fn ingest_local_push(&self, source: DeviceId, push: NotifyPush) -> IngestOutcome {
        let title = push.title.clone();
        let body = push.body.clone();
        let app_name = push.app_name.clone();
        let outcome = {
            let mut hubs = self.shared.hubs.lock().expect("hubs");
            hubs.notify.ingest_push(source, push, &*self.shared.clock)
        };
        if let IngestOutcome::Candidate(v) = &outcome {
            self.emit(UiEvent::Candidate {
                candidate: CandidateViewDto::from(v.clone()),
            });
            let app_opt = if app_name.is_empty() {
                None
            } else {
                Some(app_name.as_str())
            };
            let _ = self.dispatch_notification_banner(&title, &body, app_opt, v.has_otp);
        }
        outcome
    }

    /// Reply to the notification behind a candidate, addressed to the phone it
    /// came from. Empty text is refused; the text is trimmed and capped by
    /// `NotifyReply::truncate` on the wire. Never executed, only delivered.
    pub async fn reply_to_candidate(&self, id: &str, text: &str) -> Result<(), CoreError> {
        let (source, app_id, uid) = {
            let hubs = self.shared.hubs.lock().expect("hubs");
            let c = hubs.notify.get(id).ok_or(CoreError::UnknownCandidate)?;
            (c.source.clone(), c.app_id.clone(), c.uid.clone())
        };
        if text.trim().is_empty() {
            return Err(CoreError::EmptyReply);
        }
        let reply = NotifyReply {
            uid,
            app_id,
            text: text.to_string(),
        }
        .truncate();
        self.send_to(&source, reply.to_frame()?).await
    }

    /// Take a queued inbound reply exactly once by its stable id. Returns the
    /// addressing view and the reply text; the caller that owns the phone path
    /// acts on it (fires the notification's reply action). Single-use so a
    /// quick-reply is never fired twice.
    pub fn take_reply(&self, id: &str) -> Option<(ReplyView, String)> {
        self.shared
            .hubs
            .lock()
            .expect("hubs")
            .notify
            .take_reply_id(id)
    }

    /// Queued `notify.reply` entries awaiting delivery to their phone.
    pub fn reply_views(&self) -> Vec<ReplyView> {
        self.shared.hubs.lock().expect("hubs").notify.reply_views()
    }

    /// Install the app allowlist that gates the ANCS "open" action (M4.2).
    pub fn set_open_allowlist(&self, list: OpenAllowlist) {
        *self.shared.allowlist.lock().expect("allowlist") = list;
    }

    /// Add one allowlist rule (validated). Used by the pairing wizard.
    pub fn add_open_rule(&self, rule: tetherly_core::OpenRule) -> Result<(), CoreError> {
        self.shared.allowlist.lock().expect("allowlist").push(rule)
    }

    pub fn open_allowlist(&self) -> OpenAllowlist {
        self.shared.allowlist.lock().expect("allowlist").clone()
    }

    pub fn ancs_state(&self) -> Option<AncsState> {
        self.shared
            .ancs
            .lock()
            .expect("ancs")
            .as_ref()
            .map(|i| i.state())
    }

    pub fn ancs_report(&self) -> AncsReport {
        let (attached, state, retry_at_ms) = {
            let guard = self.shared.ancs.lock().expect("ancs");
            match guard.as_ref() {
                None => (false, "off".to_string(), None),
                Some(ing) => match ing.state() {
                    tetherly_core::AncsState::Idle => (true, "idle".to_string(), None),
                    tetherly_core::AncsState::Ready => (true, "ready".to_string(), None),
                    tetherly_core::AncsState::Backoff { retry_at_ms } => {
                        (true, "backoff".to_string(), Some(retry_at_ms))
                    }
                },
            }
        };
        let rules: Vec<OpenRuleDto> = self
            .open_allowlist()
            .rules()
            .iter()
            .map(|r| OpenRuleDto {
                app_id: r.app_id.clone(),
                url: r.url.clone(),
            })
            .collect();
        let allowlist = rules.iter().map(|r| r.app_id.clone()).collect();
        let peripheral = if self.shared.cfg.ancs_peripheral.is_empty() {
            None
        } else {
            Some(self.shared.cfg.ancs_peripheral.clone())
        };
        AncsReport {
            ok: true,
            attached,
            state,
            peripheral,
            retry_at_ms,
            rules,
            allowlist,
        }
    }

    /// Trigger ANCS connection and return a structured report, broadcasting state changes.
    pub fn ancs_connect_reported(&self) -> Result<AncsConnectReport, CoreError> {
        match self.ancs_connect()? {
            Some(ConnectOutcome::Subscribed) => {
                self.emit(UiEvent::ancs_state_changed("ready", true, None));
                Ok(AncsConnectReport {
                    ok: true,
                    state: "ready".to_string(),
                    retry_at_ms: None,
                    message: "ANCS subscribed".to_string(),
                })
            }
            Some(ConnectOutcome::BackingOff { retry_at_ms }) => {
                self.emit(UiEvent::ancs_state_changed(
                    "backoff",
                    true,
                    Some(retry_at_ms),
                ));
                Ok(AncsConnectReport {
                    ok: true,
                    state: "backoff".to_string(),
                    retry_at_ms: Some(retry_at_ms),
                    message: "ANCS backing off".to_string(),
                })
            }
            None => Err(CoreError::NoAncsTransport),
        }
    }

    /// Trigger ANCS disconnection or backoff reset, broadcasting state changes.
    pub fn ancs_disconnect_reported(&self) -> AncsDisconnectReport {
        let retry_at_ms = self.ancs_disconnected();
        let state = if retry_at_ms.is_some() {
            "backoff".to_string()
        } else {
            "idle".to_string()
        };
        self.emit(UiEvent::ancs_state_changed(&state, true, retry_at_ms));
        AncsDisconnectReport {
            ok: true,
            state,
            retry_at_ms,
            message: "ANCS disconnected".to_string(),
        }
    }

    /// Add one allowlist rule (validated), returning a structured report and broadcasting to event bus.
    pub fn add_open_rule_reported(
        &self,
        rule: tetherly_core::OpenRule,
    ) -> Result<AncsRuleReport, CoreError> {
        let app_id = rule.app_id.clone();
        let url = rule.url.clone();
        self.add_open_rule(rule)?;
        let total_rules = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .rules()
            .len();
        self.emit(UiEvent::ancs_rule_changed(
            "added",
            Some(app_id.clone()),
            Some(url.clone()),
            total_rules,
        ));
        Ok(AncsRuleReport {
            ok: true,
            action: "added".to_string(),
            rule: Some(OpenRuleDto { app_id, url }),
            total_rules,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        })
    }

    /// Remove one allowlist rule by `app_id`, returning a structured report and broadcasting to event bus.
    pub fn remove_open_rule_reported(&self, app_id: &str) -> AncsRuleReport {
        let removed = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .remove(app_id);
        let total_rules = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .rules()
            .len();
        let action = if removed.is_some() {
            "removed".to_string()
        } else {
            "not_found".to_string()
        };
        let rule_dto = removed.as_ref().map(|r| OpenRuleDto {
            app_id: r.app_id.clone(),
            url: r.url.clone(),
        });
        if removed.is_some() {
            self.emit(UiEvent::ancs_rule_changed(
                "removed",
                Some(app_id.to_string()),
                removed.as_ref().map(|r| r.url.clone()),
                total_rules,
            ));
        }
        AncsRuleReport {
            ok: true,
            action,
            rule: rule_dto,
            total_rules,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Clear all allowlist rules, returning a structured report and broadcasting to event bus.
    pub fn clear_open_rules_reported(&self) -> AncsClearRulesReport {
        let cleared_count = self.shared.allowlist.lock().expect("allowlist").clear();
        let total_rules = self
            .shared
            .allowlist
            .lock()
            .expect("allowlist")
            .rules()
            .len();
        self.emit(UiEvent::ancs_rule_changed(
            "cleared",
            None,
            None,
            total_rules,
        ));
        AncsClearRulesReport {
            ok: true,
            cleared_count,
            total_rules,
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Lightweight health report for liveness probes, monitoring, and diagnostics.
    pub fn health_report(&self) -> HealthReport {
        let ports = NodePorts {
            control: self.control_port(),
            file: self.file_port(),
            input: self.input_port(),
            screen: self.screen_port(),
            ui: self.ui_port(),
        };
        HealthReport {
            ok: true,
            status: "healthy".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            device_id: self.shared.identity.device_id().to_string(),
            name: self.name(),
            platform: self.platform().to_string(),
            pid: std::process::id(),
            uptime_secs: self.shared.started_at.elapsed().as_secs(),
            ports,
            live_peers: self.live_peers().len(),
            trusted_peers: self.trusted().len(),
        }
    }

    /// Bring the ANCS link up: Data Source first, then Notification Source.
    pub fn ancs_connect(&self) -> Result<Option<ConnectOutcome>, CoreError> {
        let mut guard = self.shared.ancs.lock().expect("ancs");
        match guard.as_mut() {
            Some(ing) => ing.connect().map(Some),
            None => Ok(None),
        }
    }

    /// Phone disconnected: schedule the backoff reconnect.
    pub fn ancs_disconnected(&self) -> Option<u64> {
        self.shared
            .ancs
            .lock()
            .expect("ancs")
            .as_mut()
            .map(|i| i.on_disconnected())
    }

    /// Notification Source value-changed. Never writes the Control Point.
    pub async fn ancs_notification_source(&self, value: &[u8]) -> Result<(), CoreError> {
        self.ancs_feed(|i| i.on_notification_source(value)).await
    }

    /// Data Source value-changed (fragments included).
    pub async fn ancs_data_source(&self, value: &[u8]) -> Result<(), CoreError> {
        self.ancs_feed(|i| i.on_data_source(value)).await
    }

    /// Drive the serial Control Point queue. Call from the BLE task.
    pub async fn ancs_tick(&self) -> Result<(), CoreError> {
        self.ancs_feed(|i| i.tick()).await
    }

    pub(super) async fn ancs_feed<F>(&self, f: F) -> Result<(), CoreError>
    where
        F: FnOnce(&mut AncsIngress) -> Result<(), CoreError>,
    {
        let events = {
            let mut guard = self.shared.ancs.lock().expect("ancs");
            let Some(ing) = guard.as_mut() else {
                return Ok(());
            };
            f(ing)?;
            ing.drain_events()
        };
        for ev in events {
            self.apply_ancs_event(ev).await;
        }
        Ok(())
    }

    pub(super) async fn apply_ancs_event(&self, ev: AncsEvent) {
        let source = self.shared.ancs_source.clone();
        match ev {
            AncsEvent::Added(n) | AncsEvent::Modified(n) => {
                let actions = {
                    let list = self.shared.allowlist.lock().expect("allowlist");
                    ancs_actions(&n.app_id, &list)
                };
                let push = NotifyPush {
                    uid: n.uid.to_string(),
                    app_id: n.app_id,
                    app_name: n.app_name,
                    title: n.title,
                    body: n.body,
                    ts: self.shared.clock.unix_ms(),
                    actions,
                };
                let _ = self.ingest_local_push(source, push).await;
            }
            AncsEvent::Removed { uid } => {
                let ids = {
                    let mut hubs = self.shared.hubs.lock().expect("hubs");
                    hubs.notify.dismiss_uid(&source, &uid.to_string())
                };
                for id in ids {
                    self.emit(UiEvent::CandidateDismissed {
                        id,
                        source: source.to_string(),
                        app_id: String::new(),
                        uid: uid.to_string(),
                    });
                }
            }
        }
    }

    /// Own the ANCS link end to end: drive the ingress from the stack's inbound
    /// channel and tick it so the serial Control Point queue drains.
    ///
    /// Runs on its own thread with a current-thread runtime. The transport's
    /// subscribe/write calls block until the BLE stack answers (bounded by the
    /// stack's own ack timeout), and an ANCS link is far too low-rate to justify
    /// letting that land on the node's async workers.
    pub fn spawn_ancs_pump(&self, inbound: tokio::sync::mpsc::UnboundedReceiver<AncsInbound>) {
        let node = self.clone_handle();
        let spawned = std::thread::Builder::new()
            .name("tetherly-ancs-pump".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        warn!(%e, "ancs: pump runtime unavailable");
                        return;
                    }
                };
                runtime.block_on(node.ancs_pump(inbound));
            });
        if let Err(e) = spawned {
            warn!(%e, "ancs: pump thread unavailable");
        }
    }

    pub(super) async fn ancs_pump(
        &self,
        mut inbound: tokio::sync::mpsc::UnboundedReceiver<AncsInbound>,
    ) {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                maybe = inbound.recv() => match maybe {
                    None => break,
                    // The session is up: subscribe Data Source then Notification
                    // Source (the ingress fixes that order; §9.4).
                    Some(AncsInbound::Ready) => {
                        if let Err(e) = self.ancs_connect() {
                            warn!(%e, "ancs: subscribing to the phone failed");
                        }
                    }
                    Some(AncsInbound::NotificationSource(value)) => {
                        if let Err(e) = self.ancs_notification_source(&value).await {
                            warn!(%e, "ancs: notification source rejected");
                        }
                    }
                    Some(AncsInbound::DataSource(value)) => {
                        if let Err(e) = self.ancs_data_source(&value).await {
                            warn!(%e, "ancs: data source rejected");
                        }
                    }
                    Some(AncsInbound::Disconnected) => {
                        if let Some(retry_at) = self.ancs_disconnected() {
                            info!(retry_at, "ancs: phone gone; backing off");
                        }
                    }
                },
                _ = tick.tick() => {
                    if let Err(e) = self.ancs_tick().await {
                        warn!(%e, "ancs: control point tick failed");
                    }
                }
            }
        }
    }
}
