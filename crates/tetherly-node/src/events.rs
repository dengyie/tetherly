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
    ResourceHandoffReceived {
        id: String,
        sender_id: String,
        sender_alias: String,
        host: String,
        title: String,
        status: String,
        auto_opened: bool,
        timestamp_ms: u64,
    },
    ResourceHandoffDispatched {
        id: String,
        target_count: usize,
        success_count: usize,
        host: String,
        timestamp_ms: u64,
    },
    HandoffPolicyChanged {
        action: String,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        max_inbox_entries: usize,
        timestamp_ms: u64,
    },
    HandoffInboxUpdated {
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        count: usize,
        timestamp_ms: u64,
    },
    MediaCommandReceived {
        id: String,
        sender_id: String,
        sender_alias: String,
        command: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        target_player: Option<String>,
        timestamp_ms: u64,
    },
    MediaCommandDispatched {
        id: String,
        target_peer: String,
        command: String,
        success: bool,
        timestamp_ms: u64,
    },
    MediaStateChanged {
        peer_id: String,
        peer_alias: String,
        player_id: String,
        state: String,
        title: String,
        artist: String,
        volume_percent: u8,
        muted: bool,
        timestamp_ms: u64,
    },
    MediaPolicyChanged {
        mode: String,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        max_volume_limit: u8,
        allow_seek: bool,
        timestamp_ms: u64,
    },
    BatteryStateChanged {
        peer_id: String,
        peer_alias: String,
        battery_level: u8,
        is_charging: bool,
        power_source: String,
        battery_saver: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        health_percent: Option<u8>,
        timestamp_ms: u64,
    },
    BatteryLowAlert {
        peer_id: String,
        peer_alias: String,
        battery_level: u8,
        is_critical: bool,
        timestamp_ms: u64,
    },
    BatteryPolicyChanged {
        low_battery_threshold: u8,
        critical_battery_threshold: u8,
        alert_on_low_battery: bool,
        auto_throttle_on_battery_saver: bool,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        timestamp_ms: u64,
    },
    LocateRingTriggered {
        request_id: String,
        peer_id: String,
        peer_alias: String,
        duration_secs: u32,
        pattern: String,
        override_silent: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        timestamp_ms: u64,
    },
    LocateRingStopped {
        #[serde(skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        peer_id: String,
        peer_alias: String,
        reason: String,
        timestamp_ms: u64,
    },
    LocatePolicyChanged {
        enabled: bool,
        allow_override_silent: bool,
        max_duration_secs: u32,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        timestamp_ms: u64,
    },
    ActionListRequested {
        peer_id: String,
        peer_alias: String,
        timestamp_ms: u64,
    },
    ActionListReceived {
        peer_id: String,
        peer_alias: String,
        count: usize,
        timestamp_ms: u64,
    },
    ActionTriggered {
        trigger_id: String,
        action_id: String,
        peer_id: String,
        peer_alias: String,
        timestamp_ms: u64,
    },
    ActionResultReceived {
        trigger_id: String,
        action_id: String,
        peer_id: String,
        peer_alias: String,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        duration_ms: u64,
        timestamp_ms: u64,
    },
    ActionPolicyChanged {
        mode: String,
        max_timeout_secs: u32,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        expose_action_list: bool,
        timestamp_ms: u64,
    },
    LocalActionsChanged {
        count: usize,
        timestamp_ms: u64,
    },
    FocusStateChanged {
        enabled: bool,
        mode: String,
        active: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        expires_ms: Option<u64>,
        timestamp_ms: u64,
    },
    PeerFocusStateChanged {
        peer_id: String,
        peer_alias: String,
        enabled: bool,
        mode: String,
        active: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        expires_ms: Option<u64>,
        timestamp_ms: u64,
    },
    FocusPolicyChanged {
        enabled: bool,
        default_mode: String,
        mirror_inbound_focus: bool,
        allow_emergency_breakthrough: bool,
        quiet_hours_enabled: bool,
        quiet_hours_start_minute: u16,
        quiet_hours_end_minute: u16,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
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

    #[allow(clippy::too_many_arguments)]
    pub fn resource_handoff_received(
        id: impl Into<String>,
        sender_id: impl Into<String>,
        sender_alias: impl Into<String>,
        host: impl Into<String>,
        title: impl Into<String>,
        status: impl Into<String>,
        auto_opened: bool,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::ResourceHandoffReceived {
            id: id.into(),
            sender_id: sender_id.into(),
            sender_alias: sender_alias.into(),
            host: host.into(),
            title: title.into(),
            status: status.into(),
            auto_opened,
            timestamp_ms,
        }
    }

    pub fn resource_handoff_dispatched(
        id: impl Into<String>,
        target_count: usize,
        success_count: usize,
        host: impl Into<String>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::ResourceHandoffDispatched {
            id: id.into(),
            target_count,
            success_count,
            host: host.into(),
            timestamp_ms,
        }
    }

    pub fn handoff_policy_changed(
        action: impl Into<String>,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        max_inbox_entries: usize,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::HandoffPolicyChanged {
            action: action.into(),
            allowed_groups,
            allowed_tags,
            allowed_types,
            max_inbox_entries,
            timestamp_ms,
        }
    }

    pub fn handoff_inbox_updated(
        action: impl Into<String>,
        id: Option<impl Into<String>>,
        count: usize,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::HandoffInboxUpdated {
            action: action.into(),
            id: id.map(Into::into),
            count,
            timestamp_ms,
        }
    }

    pub fn media_command_received(
        id: impl Into<String>,
        sender_id: impl Into<String>,
        sender_alias: impl Into<String>,
        command: impl Into<String>,
        target_player: Option<impl Into<String>>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::MediaCommandReceived {
            id: id.into(),
            sender_id: sender_id.into(),
            sender_alias: sender_alias.into(),
            command: command.into(),
            target_player: target_player.map(Into::into),
            timestamp_ms,
        }
    }

    pub fn media_command_dispatched(
        id: impl Into<String>,
        target_peer: impl Into<String>,
        command: impl Into<String>,
        success: bool,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::MediaCommandDispatched {
            id: id.into(),
            target_peer: target_peer.into(),
            command: command.into(),
            success,
            timestamp_ms,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn media_state_changed(
        peer_id: impl Into<String>,
        peer_alias: impl Into<String>,
        player_id: impl Into<String>,
        state: impl Into<String>,
        title: impl Into<String>,
        artist: impl Into<String>,
        volume_percent: u8,
        muted: bool,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::MediaStateChanged {
            peer_id: peer_id.into(),
            peer_alias: peer_alias.into(),
            player_id: player_id.into(),
            state: state.into(),
            title: title.into(),
            artist: artist.into(),
            volume_percent,
            muted,
            timestamp_ms,
        }
    }

    pub fn media_policy_changed(
        mode: impl Into<String>,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        max_volume_limit: u8,
        allow_seek: bool,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::MediaPolicyChanged {
            mode: mode.into(),
            allowed_groups,
            allowed_tags,
            allowed_types,
            max_volume_limit,
            allow_seek,
            timestamp_ms,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn battery_state_changed(
        peer_id: impl Into<String>,
        peer_alias: impl Into<String>,
        battery_level: u8,
        is_charging: bool,
        power_source: impl Into<String>,
        battery_saver: bool,
        health_percent: Option<u8>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::BatteryStateChanged {
            peer_id: peer_id.into(),
            peer_alias: peer_alias.into(),
            battery_level,
            is_charging,
            power_source: power_source.into(),
            battery_saver,
            health_percent,
            timestamp_ms,
        }
    }

    pub fn battery_low_alert(
        peer_id: impl Into<String>,
        peer_alias: impl Into<String>,
        battery_level: u8,
        is_critical: bool,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::BatteryLowAlert {
            peer_id: peer_id.into(),
            peer_alias: peer_alias.into(),
            battery_level,
            is_critical,
            timestamp_ms,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn battery_policy_changed(
        low_battery_threshold: u8,
        critical_battery_threshold: u8,
        alert_on_low_battery: bool,
        auto_throttle_on_battery_saver: bool,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::BatteryPolicyChanged {
            low_battery_threshold,
            critical_battery_threshold,
            alert_on_low_battery,
            auto_throttle_on_battery_saver,
            allowed_groups,
            allowed_tags,
            allowed_types,
            timestamp_ms,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn locate_ring_triggered(
        request_id: impl Into<String>,
        peer_id: impl Into<String>,
        peer_alias: impl Into<String>,
        duration_secs: u32,
        pattern: impl Into<String>,
        override_silent: bool,
        message: Option<impl Into<String>>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::LocateRingTriggered {
            request_id: request_id.into(),
            peer_id: peer_id.into(),
            peer_alias: peer_alias.into(),
            duration_secs,
            pattern: pattern.into(),
            override_silent,
            message: message.map(Into::into),
            timestamp_ms,
        }
    }

    pub fn locate_ring_stopped(
        request_id: Option<impl Into<String>>,
        peer_id: impl Into<String>,
        peer_alias: impl Into<String>,
        reason: impl Into<String>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::LocateRingStopped {
            request_id: request_id.map(Into::into),
            peer_id: peer_id.into(),
            peer_alias: peer_alias.into(),
            reason: reason.into(),
            timestamp_ms,
        }
    }

    pub fn locate_policy_changed(
        enabled: bool,
        allow_override_silent: bool,
        max_duration_secs: u32,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::LocatePolicyChanged {
            enabled,
            allow_override_silent,
            max_duration_secs,
            allowed_groups,
            allowed_tags,
            allowed_types,
            timestamp_ms,
        }
    }

    pub fn action_list_requested(
        peer_id: impl Into<String>,
        peer_alias: impl Into<String>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::ActionListRequested {
            peer_id: peer_id.into(),
            peer_alias: peer_alias.into(),
            timestamp_ms,
        }
    }

    pub fn action_list_received(
        peer_id: impl Into<String>,
        peer_alias: impl Into<String>,
        count: usize,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::ActionListReceived {
            peer_id: peer_id.into(),
            peer_alias: peer_alias.into(),
            count,
            timestamp_ms,
        }
    }

    pub fn action_triggered(
        trigger_id: impl Into<String>,
        action_id: impl Into<String>,
        peer_id: impl Into<String>,
        peer_alias: impl Into<String>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::ActionTriggered {
            trigger_id: trigger_id.into(),
            action_id: action_id.into(),
            peer_id: peer_id.into(),
            peer_alias: peer_alias.into(),
            timestamp_ms,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn action_result_received(
        trigger_id: impl Into<String>,
        action_id: impl Into<String>,
        peer_id: impl Into<String>,
        peer_alias: impl Into<String>,
        success: bool,
        exit_code: Option<i32>,
        duration_ms: u64,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::ActionResultReceived {
            trigger_id: trigger_id.into(),
            action_id: action_id.into(),
            peer_id: peer_id.into(),
            peer_alias: peer_alias.into(),
            success,
            exit_code,
            duration_ms,
            timestamp_ms,
        }
    }

    pub fn action_policy_changed(
        mode: impl Into<String>,
        max_timeout_secs: u32,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        expose_action_list: bool,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::ActionPolicyChanged {
            mode: mode.into(),
            max_timeout_secs,
            allowed_groups,
            allowed_tags,
            allowed_types,
            expose_action_list,
            timestamp_ms,
        }
    }

    pub fn local_actions_changed(count: usize, timestamp_ms: u64) -> Self {
        UiEvent::LocalActionsChanged {
            count,
            timestamp_ms,
        }
    }

    pub fn focus_state_changed(
        enabled: bool,
        mode: impl Into<String>,
        active: bool,
        reason: Option<impl Into<String>>,
        expires_ms: Option<u64>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::FocusStateChanged {
            enabled,
            mode: mode.into(),
            active,
            reason: reason.map(Into::into),
            expires_ms,
            timestamp_ms,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn peer_focus_state_changed(
        peer_id: impl Into<String>,
        peer_alias: impl Into<String>,
        enabled: bool,
        mode: impl Into<String>,
        active: bool,
        reason: Option<impl Into<String>>,
        expires_ms: Option<u64>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::PeerFocusStateChanged {
            peer_id: peer_id.into(),
            peer_alias: peer_alias.into(),
            enabled,
            mode: mode.into(),
            active,
            reason: reason.map(Into::into),
            expires_ms,
            timestamp_ms,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn focus_policy_changed(
        enabled: bool,
        default_mode: impl Into<String>,
        mirror_inbound_focus: bool,
        allow_emergency_breakthrough: bool,
        quiet_hours_enabled: bool,
        quiet_hours_start_minute: u16,
        quiet_hours_end_minute: u16,
        allowed_groups: Vec<String>,
        allowed_tags: Vec<String>,
        allowed_types: Vec<String>,
        timestamp_ms: u64,
    ) -> Self {
        UiEvent::FocusPolicyChanged {
            enabled,
            default_mode: default_mode.into(),
            mirror_inbound_focus,
            allow_emergency_breakthrough,
            quiet_hours_enabled,
            quiet_hours_start_minute,
            quiet_hours_end_minute,
            allowed_groups,
            allowed_tags,
            allowed_types,
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

    #[test]
    fn test_resource_handoff_events_serialization_and_zero_plaintext() {
        let ev1 = UiEvent::resource_handoff_received(
            "ho_123",
            "peer_dev_1",
            "MacBook",
            "github.com",
            "Rust PR #1234",
            "pending",
            false,
            123456791,
        );
        let json1 = serde_json::to_string(&ev1).unwrap();
        assert!(json1.contains("\"kind\":\"resource_handoff_received\""));
        assert!(json1.contains("\"id\":\"ho_123\""));
        assert!(json1.contains("\"host\":\"github.com\""));
        assert!(json1.contains("\"auto_opened\":false"));
        // Assert no URL path or parameters leaked in event
        assert!(!json1.contains("token="));

        let ev2 = UiEvent::resource_handoff_dispatched("ho_123", 3, 2, "example.com", 123456792);
        let json2 = serde_json::to_string(&ev2).unwrap();
        assert!(json2.contains("\"kind\":\"resource_handoff_dispatched\""));
        assert!(json2.contains("\"target_count\":3"));
        assert!(json2.contains("\"success_count\":2"));

        let ev3 = UiEvent::handoff_policy_changed(
            "auto_open",
            vec!["office".into()],
            vec!["trusted".into()],
            vec!["phone".into()],
            100,
            123456793,
        );
        let json3 = serde_json::to_string(&ev3).unwrap();
        assert!(json3.contains("\"kind\":\"handoff_policy_changed\""));
        assert!(json3.contains("\"action\":\"auto_open\""));
        assert!(json3.contains("\"allowed_groups\":[\"office\"]"));
    }

    #[test]
    fn test_battery_events_serialization() {
        let ev1 = UiEvent::battery_state_changed(
            "peer_123",
            "iPhone 15",
            85,
            true,
            "ac",
            false,
            Some(95),
            123456794,
        );
        let json1 = serde_json::to_string(&ev1).unwrap();
        assert!(json1.contains("\"kind\":\"battery_state_changed\""));
        assert!(json1.contains("\"peer_id\":\"peer_123\""));
        assert!(json1.contains("\"battery_level\":85"));
        assert!(json1.contains("\"is_charging\":true"));
        assert!(json1.contains("\"power_source\":\"ac\""));
        assert!(json1.contains("\"health_percent\":95"));

        let ev2 = UiEvent::battery_low_alert("peer_123", "iPhone 15", 15, false, 123456795);
        let json2 = serde_json::to_string(&ev2).unwrap();
        assert!(json2.contains("\"kind\":\"battery_low_alert\""));
        assert!(json2.contains("\"battery_level\":15"));
        assert!(json2.contains("\"is_critical\":false"));

        let ev3 = UiEvent::battery_policy_changed(
            20,
            10,
            true,
            false,
            vec!["mobile".into()],
            vec!["work".into()],
            vec!["phone".into()],
            123456796,
        );
        let json3 = serde_json::to_string(&ev3).unwrap();
        assert!(json3.contains("\"kind\":\"battery_policy_changed\""));
        assert!(json3.contains("\"low_battery_threshold\":20"));
        assert!(json3.contains("\"critical_battery_threshold\":10"));
        assert!(json3.contains("\"alert_on_low_battery\":true"));
    }

    #[test]
    fn test_locate_events_serialization() {
        let ev1 = UiEvent::locate_ring_triggered(
            "req_loc_1",
            "peer_phone",
            "My Pixel",
            20,
            "siren",
            true,
            Some("Where is my phone?"),
            123456797,
        );
        let json1 = serde_json::to_string(&ev1).unwrap();
        assert!(json1.contains("\"kind\":\"locate_ring_triggered\""));
        assert!(json1.contains("\"request_id\":\"req_loc_1\""));
        assert!(json1.contains("\"peer_id\":\"peer_phone\""));
        assert!(json1.contains("\"peer_alias\":\"My Pixel\""));
        assert!(json1.contains("\"duration_secs\":20"));
        assert!(json1.contains("\"pattern\":\"siren\""));
        assert!(json1.contains("\"override_silent\":true"));
        assert!(json1.contains("\"message\":\"Where is my phone?\""));

        let ev2 = UiEvent::locate_ring_stopped(
            Some("req_loc_1"),
            "peer_phone",
            "My Pixel",
            "found_device",
            123456798,
        );
        let json2 = serde_json::to_string(&ev2).unwrap();
        assert!(json2.contains("\"kind\":\"locate_ring_stopped\""));
        assert!(json2.contains("\"request_id\":\"req_loc_1\""));
        assert!(json2.contains("\"reason\":\"found_device\""));

        let ev3 = UiEvent::locate_policy_changed(
            true,
            false,
            45,
            vec!["family".into()],
            vec!["trusted".into()],
            vec!["phone".into()],
            123456799,
        );
        let json3 = serde_json::to_string(&ev3).unwrap();
        assert!(json3.contains("\"kind\":\"locate_policy_changed\""));
        assert!(json3.contains("\"enabled\":true"));
        assert!(json3.contains("\"allow_override_silent\":false"));
        assert!(json3.contains("\"max_duration_secs\":45"));
        assert!(json3.contains("\"allowed_groups\":[\"family\"]"));
    }

    #[test]
    fn test_action_events_serialization_and_zero_plaintext() {
        let ev1 = UiEvent::action_list_requested("peer_laptop", "MacBook Pro", 123456800);
        let json1 = serde_json::to_string(&ev1).unwrap();
        assert!(json1.contains("\"kind\":\"action_list_requested\""));
        assert!(json1.contains("\"peer_id\":\"peer_laptop\""));
        assert!(json1.contains("\"peer_alias\":\"MacBook Pro\""));

        let ev2 = UiEvent::action_list_received("peer_laptop", "MacBook Pro", 5, 123456801);
        let json2 = serde_json::to_string(&ev2).unwrap();
        assert!(json2.contains("\"kind\":\"action_list_received\""));
        assert!(json2.contains("\"count\":5"));

        let ev3 =
            UiEvent::action_triggered("trig_1", "lock_screen", "peer_phone", "iPhone", 123456802);
        let json3 = serde_json::to_string(&ev3).unwrap();
        assert!(json3.contains("\"kind\":\"action_triggered\""));
        assert!(json3.contains("\"trigger_id\":\"trig_1\""));
        assert!(json3.contains("\"action_id\":\"lock_screen\""));
        // Zero-Plaintext: command string or args must never appear in trigger event
        assert!(!json3.contains("loginctl"));
        assert!(!json3.contains("sh"));

        let ev4 = UiEvent::action_result_received(
            "trig_1",
            "lock_screen",
            "peer_phone",
            "iPhone",
            true,
            Some(0),
            120,
            123456803,
        );
        let json4 = serde_json::to_string(&ev4).unwrap();
        assert!(json4.contains("\"kind\":\"action_result_received\""));
        assert!(json4.contains("\"success\":true"));
        assert!(json4.contains("\"exit_code\":0"));
        assert!(json4.contains("\"duration_ms\":120"));

        let ev5 = UiEvent::action_policy_changed(
            "allow",
            15,
            vec!["work".into()],
            vec!["laptop".into()],
            vec!["desktop".into()],
            true,
            123456804,
        );
        let json5 = serde_json::to_string(&ev5).unwrap();
        assert!(json5.contains("\"kind\":\"action_policy_changed\""));
        assert!(json5.contains("\"mode\":\"allow\""));
        assert!(json5.contains("\"max_timeout_secs\":15"));
        assert!(json5.contains("\"expose_action_list\":true"));

        let ev6 = UiEvent::local_actions_changed(3, 123456805);
        let json6 = serde_json::to_string(&ev6).unwrap();
        assert!(json6.contains("\"kind\":\"local_actions_changed\""));
        assert!(json6.contains("\"count\":3"));
    }

    #[test]
    fn test_focus_events_serialization() {
        let ev1 = UiEvent::focus_state_changed(
            true,
            "meeting",
            true,
            Some("Quarterly review"),
            Some(1700000000000),
            123456806,
        );
        let json1 = serde_json::to_string(&ev1).unwrap();
        assert!(json1.contains("\"kind\":\"focus_state_changed\""));
        assert!(json1.contains("\"enabled\":true"));
        assert!(json1.contains("\"mode\":\"meeting\""));
        assert!(json1.contains("\"active\":true"));
        assert!(json1.contains("\"reason\":\"Quarterly review\""));
        assert!(json1.contains("\"expires_ms\":1700000000000"));

        let ev2 = UiEvent::peer_focus_state_changed(
            "peer_phone",
            "iPhone",
            true,
            "dnd",
            true,
            None::<String>,
            None,
            123456807,
        );
        let json2 = serde_json::to_string(&ev2).unwrap();
        assert!(json2.contains("\"kind\":\"peer_focus_state_changed\""));
        assert!(json2.contains("\"peer_id\":\"peer_phone\""));
        assert!(json2.contains("\"peer_alias\":\"iPhone\""));
        assert!(json2.contains("\"enabled\":true"));
        assert!(json2.contains("\"mode\":\"dnd\""));
        assert!(json2.contains("\"active\":true"));

        let ev3 = UiEvent::focus_policy_changed(
            true,
            "work",
            true,
            true,
            true,
            1380,
            420,
            vec!["exec".into()],
            vec!["vip".into()],
            vec!["phone".into()],
            123456808,
        );
        let json3 = serde_json::to_string(&ev3).unwrap();
        assert!(json3.contains("\"kind\":\"focus_policy_changed\""));
        assert!(json3.contains("\"enabled\":true"));
        assert!(json3.contains("\"default_mode\":\"work\""));
        assert!(json3.contains("\"mirror_inbound_focus\":true"));
        assert!(json3.contains("\"allow_emergency_breakthrough\":true"));
        assert!(json3.contains("\"quiet_hours_enabled\":true"));
        assert!(json3.contains("\"quiet_hours_start_minute\":1380"));
        assert!(json3.contains("\"quiet_hours_end_minute\":420"));
        assert!(json3.contains("\"allowed_groups\":[\"exec\"]"));
    }
}
