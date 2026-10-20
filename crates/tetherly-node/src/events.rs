// SPDX-License-Identifier: Apache-2.0 OR MIT
//! UI-facing events. Bodies, OTP digits, and clipboard text never appear here.

use serde::{Deserialize, Serialize};
use tetherly_core::{CandidateView, DeviceId, IncomingTransfer, ReplyView};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEvent {
    Candidate {
        candidate: CandidateViewDto,
    },
    CandidateExpired {
        id: String,
    },
    CandidateDismissed {
        id: String,
        source: String,
        app_id: String,
        uid: String,
    },
    ReplyReceived {
        reply: ReplyViewDto,
    },
    FileOffered {
        transfer_id: String,
        source: String,
        files: Vec<FileView>,
    },
    FileRejected {
        transfer_id: String,
        source: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    FileAutoAcceptChanged {
        enabled: bool,
    },
    FileDone {
        transfer_id: String,
    },
    PeerUp {
        device_id: String,
        name: String,
        platform: String,
    },
    PeerDown {
        device_id: String,
    },
    PairingNeeded {
        device_id: String,
        fp: String,
    },
    Status {
        message: String,
    },
    Overlay {
        present: bool,
        lan_only: bool,
    },
    PeerLatency {
        device_id: String,
        rtt_ms: u64,
    },
    PinStateChanged {
        active: bool,
    },
    PeerAliasChanged {
        device_id: String,
        alias: String,
    },
    ConfigChanged {
        name: String,
        advertise: bool,
        reconnect: bool,
    },
    RouteChanged {
        device_id: String,
        addr: String,
        action: String,
    },
    MetricsReset {
        timestamp_ms: u64,
    },
    PeerCapsChanged {
        device_id: String,
        caps: Vec<String>,
        features: Vec<String>,
    },
    PeerBlocked {
        device_id: String,
        reason: Option<String>,
    },
    PeerUnblocked {
        device_id: String,
    },
    ClipboardSynced {
        seq: u64,
        source: String,
        direction: String,
        bytes: usize,
        hash: String,
        timestamp_ms: u64,
        sent_peers: usize,
    },
    ClipboardCleared {
        timestamp_ms: u64,
    },
    ScreenStateChanged {
        state: String,
        allowed: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        peer: Option<String>,
    },
    InputSeatChanged {
        seat: String,
        channel_open: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        peer: Option<String>,
    },
    NetworkRefreshed {
        iface_count: usize,
        listen_count: usize,
        overlay_present: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        overlay_source: Option<String>,
    },
    OverlayStateChanged {
        present: bool,
        source: String,
        local_ips: Vec<String>,
        peer_count: usize,
    },
    AncsStateChanged {
        state: String,
        attached: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_at_ms: Option<u64>,
    },
    AncsRuleChanged {
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        app_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        rules_count: usize,
    },
    InboxFileDeleted {
        name: String,
        freed_bytes: u64,
        timestamp_ms: u64,
    },
    InboxCleared {
        deleted_count: usize,
        freed_bytes: u64,
        timestamp_ms: u64,
    },
    InboxFileOpened {
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        timestamp_ms: u64,
    },
    NotificationBannerSent {
        title: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        app_name: Option<String>,
        is_otp: bool,
        timestamp_ms: u64,
    },
    NotifyConfigChanged {
        banners_enabled: bool,
        dnd_mode: String,
        sound_enabled: bool,
        mask_otp: bool,
        allow_otp_in_dnd: bool,
    },
    PeerDiscovered {
        device_id: String,
        name: String,
        addr: String,
        caps: Vec<String>,
    },
    PeerExpired {
        device_id: String,
    },
    SystemMetricsRefreshed {
        pid: u32,
        uptime_secs: u64,
        rss_bytes: u64,
        virt_bytes: u64,
        thread_count: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        open_fds: Option<u32>,
        cpu_time_ms: u64,
        num_cpus: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        total_memory_bytes: Option<u64>,
        timestamp_ms: u64,
    },
    PeerLinkQualityRefreshed {
        device_id: String,
        rtt_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        min_rtt_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_rtt_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        avg_rtt_ms: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        jitter_ms: Option<f64>,
        packet_loss_rate: f64,
        grade: String,
        timestamp_ms: u64,
    },
    HeartbeatConfigChanged {
        interval_secs: u64,
        deadline_secs: u64,
        timestamp_ms: u64,
    },
    AuditEventRecorded {
        id: u64,
        category: String,
        severity: String,
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        summary: String,
        success: bool,
        timestamp_ms: u64,
    },
    MessageReceived {
        id: String,
        peer: String,
        len: usize,
        timestamp_ms: u64,
    },
    MessageSent {
        id: String,
        peer: String,
        len: usize,
        timestamp_ms: u64,
    },
    OutboxUpdated {
        id: String,
        transfer_id: String,
        peer: String,
        filename: String,
        size: u64,
        status: String,
        timestamp_ms: u64,
    },
    OutboxCleared {
        count: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        peer: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        timestamp_ms: u64,
    },
    PeerProfileUpdated {
        device_id: String,
        device_type: String,
        tags: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        group: Option<String>,
        timestamp_ms: u64,
    },
    MultiDispatchCompleted {
        dispatch_kind: String,
        selector: String,
        target_count: usize,
        success_count: usize,
        skipped_offline_count: usize,
        failed_count: usize,
        timestamp_ms: u64,
    },
    AutoAcceptPolicyChanged {
        enabled: bool,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_file_size: Option<u64>,
        timestamp_ms: u64,
    },
    ClipboardPolicyChanged {
        mode: String,
        sensitive_guard: bool,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        max_payload_bytes: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        auto_clear_timeout_secs: Option<u64>,
        timestamp_ms: u64,
    },
    ClipboardSyncBlocked {
        reason: String,
        bytes: usize,
        hash: String,
        timestamp_ms: u64,
    },
}

impl UiEvent {
    pub fn multi_dispatch_completed(
        kind: impl Into<String>,
        selector: impl Into<String>,
        target_count: usize,
        success_count: usize,
        skipped_offline_count: usize,
        failed_count: usize,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::MultiDispatchCompleted {
            dispatch_kind: kind.into(),
            selector: selector.into(),
            target_count,
            success_count,
            skipped_offline_count,
            failed_count,
            timestamp_ms,
        }
    }

    pub fn auto_accept_policy_changed(
        enabled: bool,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        max_file_size: Option<u64>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::AutoAcceptPolicyChanged {
            enabled,
            allowed_groups,
            allowed_tags,
            allowed_types,
            max_file_size,
            timestamp_ms,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn clipboard_policy_changed(
        mode: impl Into<String>,
        sensitive_guard: bool,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        max_payload_bytes: usize,
        auto_clear_timeout_secs: Option<u64>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::ClipboardPolicyChanged {
            mode: mode.into(),
            sensitive_guard,
            allowed_groups,
            allowed_tags,
            allowed_types,
            max_payload_bytes,
            auto_clear_timeout_secs,
            timestamp_ms,
        }
    }

    pub fn clipboard_sync_blocked(
        reason: impl Into<String>,
        bytes: usize,
        hash: impl Into<String>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::ClipboardSyncBlocked {
            reason: reason.into(),
            bytes,
            hash: hash.into(),
            timestamp_ms,
        }
    }
    pub fn peer_profile_updated(
        device_id: impl Into<String>,
        device_type: impl Into<String>,
        tags: Vec<String>,
        group: Option<impl Into<String>>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::PeerProfileUpdated {
            device_id: device_id.into(),
            device_type: device_type.into(),
            tags,
            group: group.map(Into::into),
            timestamp_ms,
        }
    }
    pub fn outbox_updated(
        id: impl Into<String>,
        transfer_id: impl Into<String>,
        peer: impl Into<String>,
        filename: impl Into<String>,
        size: u64,
        status: impl Into<String>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::OutboxUpdated {
            id: id.into(),
            transfer_id: transfer_id.into(),
            peer: peer.into(),
            filename: filename.into(),
            size,
            status: status.into(),
            timestamp_ms,
        }
    }

    pub fn outbox_cleared(
        count: usize,
        peer: Option<impl Into<String>>,
        status: Option<impl Into<String>>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::OutboxCleared {
            count,
            peer: peer.map(Into::into),
            status: status.map(Into::into),
            timestamp_ms,
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn audit_event_recorded(
        id: u64,
        category: impl Into<String>,
        severity: impl Into<String>,
        action: impl Into<String>,
        target: Option<impl Into<String>>,
        summary: impl Into<String>,
        success: bool,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::AuditEventRecorded {
            id,
            category: category.into(),
            severity: severity.into(),
            action: action.into(),
            target: target.map(Into::into),
            summary: summary.into(),
            success,
            timestamp_ms,
        }
    }
    pub fn message_received(
        id: impl Into<String>,
        peer: impl Into<String>,
        len: usize,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::MessageReceived {
            id: id.into(),
            peer: peer.into(),
            len,
            timestamp_ms,
        }
    }

    pub fn message_sent(
        id: impl Into<String>,
        peer: impl Into<String>,
        len: usize,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::MessageSent {
            id: id.into(),
            peer: peer.into(),
            len,
            timestamp_ms,
        }
    }

    pub fn inbox_file_deleted(
        name: impl Into<String>,
        freed_bytes: u64,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::InboxFileDeleted {
            name: name.into(),
            freed_bytes,
            timestamp_ms,
        }
    }

    pub fn inbox_cleared(deleted_count: usize, freed_bytes: u64, timestamp_ms: u64) -> Self {
        UiEvent::InboxCleared {
            deleted_count,
            freed_bytes,
            timestamp_ms,
        }
    }

    pub fn inbox_file_opened(name: Option<impl Into<String>>, timestamp_ms: u64) -> Self {
        UiEvent::InboxFileOpened {
            name: name.map(Into::into),
            timestamp_ms,
        }
    }

    pub fn notification_banner_sent(
        title: impl Into<String>,
        app_name: Option<impl Into<String>>,
        is_otp: bool,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::NotificationBannerSent {
            title: title.into(),
            app_name: app_name.map(Into::into),
            is_otp,
            timestamp_ms,
        }
    }

    pub fn notify_config_changed(
        banners_enabled: bool,
        dnd_mode: impl Into<String>,
        sound_enabled: bool,
        mask_otp: bool,
        allow_otp_in_dnd: bool,
    ) -> Self {
        UiEvent::NotifyConfigChanged {
            banners_enabled,
            dnd_mode: dnd_mode.into(),
            sound_enabled,
            mask_otp,
            allow_otp_in_dnd,
        }
    }

    pub fn screen_state_changed(
        state: impl Into<String>,
        allowed: bool,
        peer: Option<String>,
    ) -> Self {
        screen_state_changed(state, allowed, peer)
    }

    pub fn input_seat_changed(
        seat: impl Into<String>,
        channel_open: bool,
        peer: Option<String>,
    ) -> Self {
        input_seat_changed(seat, channel_open, peer)
    }

    pub fn network_refreshed(
        iface_count: usize,
        listen_count: usize,
        overlay_present: bool,
        overlay_source: Option<String>,
    ) -> Self {
        network_refreshed(iface_count, listen_count, overlay_present, overlay_source)
    }

    pub fn overlay_state_changed(
        present: bool,
        source: impl Into<String>,
        local_ips: Vec<String>,
        peer_count: usize,
    ) -> Self {
        overlay_state_changed(present, source, local_ips, peer_count)
    }

    pub fn ancs_state_changed(
        state: impl Into<String>,
        attached: bool,
        retry_at_ms: Option<u64>,
    ) -> Self {
        ancs_state_changed(state, attached, retry_at_ms)
    }

    pub fn ancs_rule_changed(
        action: impl Into<String>,
        app_id: Option<String>,
        url: Option<String>,
        rules_count: usize,
    ) -> Self {
        ancs_rule_changed(action, app_id, url, rules_count)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn system_metrics_refreshed(
        pid: u32,
        uptime_secs: u64,
        rss_bytes: u64,
        virt_bytes: u64,
        thread_count: u32,
        open_fds: Option<u32>,
        cpu_time_ms: u64,
        num_cpus: usize,
        total_memory_bytes: Option<u64>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::SystemMetricsRefreshed {
            pid,
            uptime_secs,
            rss_bytes,
            virt_bytes,
            thread_count,
            open_fds,
            cpu_time_ms,
            num_cpus,
            total_memory_bytes,
            timestamp_ms,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateViewDto {
    pub id: String,
    pub source: String,
    pub app_id: String,
    pub app_name: String,
    pub uid: String,
    pub title: String,
    pub has_otp: bool,
    pub created_ms: u64,
    pub expires_ms: u64,
    pub actions: Vec<String>,
}

impl From<CandidateView> for CandidateViewDto {
    fn from(v: CandidateView) -> Self {
        Self {
            id: v.id,
            source: v.source.to_string(),
            app_id: v.app_id,
            app_name: v.app_name,
            uid: v.uid,
            title: v.title,
            has_otp: v.has_otp,
            created_ms: v.created_ms,
            expires_ms: v.expires_ms,
            actions: v.actions,
        }
    }
}

/// A queued `notify.reply`, shown so the user knows a reply is waiting. The
/// text is not carried here (see `ReplyView`), matching the no-content rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplyViewDto {
    pub id: String,
    pub source: String,
    pub app_id: String,
    pub uid: String,
}

impl From<ReplyView> for ReplyViewDto {
    fn from(v: ReplyView) -> Self {
        Self {
            id: v.id,
            source: v.source.to_string(),
            app_id: v.app_id,
            uid: v.uid,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileView {
    pub name: String,
    pub size: u64,
}

pub fn file_offered(t: &IncomingTransfer) -> UiEvent {
    UiEvent::FileOffered {
        transfer_id: t.transfer_id.clone(),
        source: t.source.to_string(),
        files: t
            .files
            .iter()
            .map(|f| FileView {
                name: f.name.clone(),
                size: f.size,
            })
            .collect(),
    }
}

pub fn peer_up(id: &DeviceId, name: &str, platform: &str) -> UiEvent {
    UiEvent::PeerUp {
        device_id: id.to_string(),
        name: name.to_string(),
        platform: platform.to_string(),
    }
}

pub fn screen_state_changed(
    state: impl Into<String>,
    allowed: bool,
    peer: Option<String>,
) -> UiEvent {
    UiEvent::ScreenStateChanged {
        state: state.into(),
        allowed,
        peer,
    }
}

pub fn input_seat_changed(
    seat: impl Into<String>,
    channel_open: bool,
    peer: Option<String>,
) -> UiEvent {
    UiEvent::InputSeatChanged {
        seat: seat.into(),
        channel_open,
        peer,
    }
}

pub fn network_refreshed(
    iface_count: usize,
    listen_count: usize,
    overlay_present: bool,
    overlay_source: Option<String>,
) -> UiEvent {
    UiEvent::NetworkRefreshed {
        iface_count,
        listen_count,
        overlay_present,
        overlay_source,
    }
}

pub fn overlay_state_changed(
    present: bool,
    source: impl Into<String>,
    local_ips: Vec<String>,
    peer_count: usize,
) -> UiEvent {
    UiEvent::OverlayStateChanged {
        present,
        source: source.into(),
        local_ips,
        peer_count,
    }
}

pub fn ancs_state_changed(
    state: impl Into<String>,
    attached: bool,
    retry_at_ms: Option<u64>,
) -> UiEvent {
    UiEvent::AncsStateChanged {
        state: state.into(),
        attached,
        retry_at_ms,
    }
}

pub fn ancs_rule_changed(
    action: impl Into<String>,
    app_id: Option<String>,
    url: Option<String>,
    rules_count: usize,
) -> UiEvent {
    UiEvent::AncsRuleChanged {
        action: action.into(),
        app_id,
        url,
        rules_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_events_serialization_and_zero_plaintext() {
        let ev1 = UiEvent::clipboard_policy_changed(
            "bidirectional_auto",
            true,
            vec!["engineering".into()],
            vec!["secure".into()],
            vec!["laptop".into()],
            65536,
            Some(60),
            123456789,
        );
        let json1 = serde_json::to_string(&ev1).unwrap();
        assert!(json1.contains("\"kind\":\"clipboard_policy_changed\""));
        assert!(json1.contains("\"mode\":\"bidirectional_auto\""));
        assert!(json1.contains("\"sensitive_guard\":true"));
        assert!(json1.contains("\"allowed_groups\":[\"engineering\"]"));

        let ev2 = UiEvent::clipboard_sync_blocked("private_key", 1024, "abcdef123456", 123456790);
        let json2 = serde_json::to_string(&ev2).unwrap();
        assert!(json2.contains("\"kind\":\"clipboard_sync_blocked\""));
        assert!(json2.contains("\"reason\":\"private_key\""));
        assert!(json2.contains("\"bytes\":1024"));
        assert!(json2.contains("\"hash\":\"abcdef123456\""));
        // Confirm no plaintext leaked
        assert!(!json2.contains("BEGIN"));
        assert!(!json2.contains("PRIVATE"));
    }
}
