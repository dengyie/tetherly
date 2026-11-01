// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Request bodies and response DTOs for the loopback UI JSON API.
//!
//! Every struct here is a wire shape: `Serialize` for responses, `Deserialize`
//! (with `#[serde(default)]`) for request bodies. Field names are the public
//! contract of `ui/index.html` and the `tetherly` CLI, so they are not renamed.

use super::*;

#[derive(Serialize)]
pub(super) struct Snapshot {
    pub(super) device_id: String,
    pub(super) name: String,
    pub(super) platform: String,
    pub(super) pin_active: bool,
    pub(super) lan_only: bool,
    pub(super) overlay_present: bool,
    pub(super) overlay_source: String,
    pub(super) overlay_peers: usize,
    pub(super) candidates: Vec<crate::events::CandidateViewDto>,
    /// Inbound quick replies awaiting hand-off. No text is included.
    pub(super) replies: Vec<crate::events::ReplyViewDto>,
    pub(super) peers: Vec<PeerView>,
    pub(super) trusted: Vec<TrustedPeerView>,
    pub(super) files: Vec<FileSnap>,
    pub(super) transfers: Vec<crate::runtime::ActiveTransfer>,
    pub(super) transfer_stats: crate::runtime::TransferStats,
    /// "off" when no BLE transport is attached, else the ingress state.
    pub(super) ancs_state: String,
    /// Allowlisted app ids, so the wizard can show what may be opened.
    pub(super) open_apps: Vec<String>,
    /// Remote-screen channel state (idle/requested/allowed/streaming).
    pub(super) screen_state: String,
    /// Screen counters for the local UI. Never carries pixels.
    pub(super) screen: ScreenSnap,
    /// Input channel state: whether 45719 is dialled and where the seat is.
    pub(super) input_channel: bool,
    pub(super) input_seat: String,
    /// Every local address and why it is or is not in the LAN listen set. Lets
    /// the UI explain "nothing found" without the user reading logs.
    pub(super) lan: Vec<crate::lan::IfaceReport>,
    /// Aggregate network traffic and protocol telemetry report.
    pub(super) traffic: crate::telemetry::TrafficReport,
    /// Total audit entries recorded in ring buffer.
    pub(super) audit_count: usize,
    /// Total unread direct messages in ring buffer.
    pub(super) unread_messages_count: usize,
    /// Total resource handoff entries in inbox.
    pub(super) handoff_inbox_count: usize,
    /// Current handoff action policy string.
    pub(super) handoff_action: String,
    /// Local media playback state if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) media_state: Option<tetherly_core::MediaStateFrame>,
    /// Media ingress control mode string.
    pub(super) media_mode: String,
    /// Local battery state if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) battery_state: Option<tetherly_core::BatteryStateFrame>,
    /// Low battery threshold percentage.
    pub(super) battery_low_threshold: u8,
    /// Focus status report if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) focus_status: Option<crate::runtime::FocusStatusReport>,
    /// Spatial topology layout report if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) topology_layout: Option<crate::runtime::TopologyLayoutReport>,
    /// Spatial audio routing status report if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) audio_routing_status: Option<crate::runtime::AudioRoutingStatusReport>,
    /// Virtual camera status report if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) camera_status: Option<crate::runtime::CameraStatusReport>,
}

#[derive(Serialize)]
pub(super) struct ScreenSnap {
    pub(super) state: String,
    pub(super) sent: u64,
    pub(super) received: u64,
    pub(super) dropped: u64,
    pub(super) bytes: u64,
    pub(super) refused: u64,
    /// p95 host-side frame interval in microseconds. Null until measurable.
    pub(super) p95_frame_interval_us: Option<u64>,
    pub(super) acked: u64,
    pub(super) in_flight: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) rtt_latest_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) rtt_p95_us: Option<u64>,
    pub(super) is_congested: bool,
}

#[derive(Serialize)]
pub(super) struct PeerView {
    pub(super) device_id: String,
    pub(super) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) alias: Option<String>,
    pub(super) platform: String,
    pub(super) device_type: String,
    #[serde(default)]
    pub(super) tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) group: Option<String>,
    pub(super) path: String,
    pub(super) caps: Vec<String>,
    pub(super) features: Vec<String>,
    pub(super) file_port: Option<u16>,
    pub(super) input_port: Option<u16>,
    pub(super) screen_port: Option<u16>,
    pub(super) rtt_ms: Option<u64>,
    pub(super) last_seen_ms: u64,
}

#[derive(Serialize)]
pub struct TrustedPeerView {
    pub device_id: String,
    pub alias: String,
    pub device_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub paired_at: u64,
    pub revoked: bool,
    pub online: bool,
}

#[derive(Serialize)]
pub(super) struct FileSnap {
    pub(super) transfer_id: String,
    pub(super) source: String,
    pub(super) name: String,
    pub(super) size: u64,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct ProfileUpdateBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) device_type: Option<String>,
    #[serde(default)]
    pub(super) tags: Option<Vec<String>>,
    #[serde(default)]
    pub(super) group: Option<Option<String>>,
    #[serde(default)]
    pub(super) notes: Option<Option<String>>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct PeerTagBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) tag: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct PeerGroupBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) group: Option<String>,
}

#[derive(serde::Deserialize)]
pub(super) struct DialBody {
    #[serde(default)]
    pub(super) addr: Option<String>,
    #[serde(default)]
    pub(super) peer: Option<String>,
    pub(super) pin: Option<String>,
}

#[derive(serde::Deserialize)]
pub(super) struct NotifyBody {
    pub(super) title: String,
    pub(super) body: String,
    #[serde(default = "default_notify_app")]
    pub(super) app_id: String,
    pub(super) peer: Option<String>,
}

pub(super) fn default_notify_app() -> String {
    "com.example.sms".into()
}

/// Wall clock for a locally originated push. The node clock is the handshake
/// clock; a notify timestamp only has to be monotone enough for the peer's TTL.
pub(super) fn node_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(serde::Deserialize)]
pub(super) struct IdBody {
    pub(super) id: String,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(super) struct FileDecisionBody {
    pub(super) id: Option<String>,
    pub(super) peer: Option<String>,
    pub(super) all: Option<bool>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(super) struct AutoAcceptBody {
    pub(super) enabled: Option<bool>,
    pub(super) auto_accept_trusted: Option<bool>,
    #[serde(default)]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) max_file_size: Option<u64>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub(super) struct NotifyDismissBody {
    pub(super) id: Option<String>,
    pub(super) peer: Option<String>,
    pub(super) app_id: Option<String>,
    pub(super) all: Option<bool>,
}

#[derive(serde::Deserialize)]
pub(super) struct UnpairBody {
    pub(super) device_id: String,
}

#[derive(serde::Deserialize)]
pub(super) struct ReplyBody {
    pub(super) id: String,
    pub(super) text: String,
}

#[derive(serde::Deserialize)]
pub(super) struct OpenRuleBody {
    pub(super) app_id: String,
    #[serde(default = "default_open_url")]
    pub(super) url: String,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct OpenRuleRemoveBody {
    #[serde(default)]
    pub(super) app_id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct StorageCleanBody {
    #[serde(default)]
    pub(super) orphaned_only: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct InboxDeleteBody {
    #[serde(default)]
    pub(super) name: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct InboxClearBody {
    #[serde(default)]
    pub(super) before_secs: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct InboxOpenBody {
    #[serde(default)]
    pub(super) name: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct NotifyConfigUpdateBody {
    pub(super) banners_enabled: Option<bool>,
    pub(super) dnd_mode: Option<crate::runtime::DndMode>,
    pub(super) sound_enabled: Option<bool>,
    pub(super) mask_otp: Option<bool>,
    pub(super) allow_otp_in_dnd: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct NotifyTestBody {
    pub(super) title: Option<String>,
    pub(super) body: Option<String>,
}

pub(super) fn default_open_url() -> String {
    "tetherly://open".to_string()
}

#[derive(serde::Deserialize)]
pub(super) struct SendFileBody {
    pub(super) path: Option<String>,
    #[serde(default, alias = "data")]
    pub(super) data_base64: Option<String>,
    pub(super) peer: Option<String>,
    #[serde(default, alias = "filename")]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) tag: Option<String>,
    #[serde(default)]
    pub(super) group: Option<String>,
    #[serde(default)]
    pub(super) device_type: Option<String>,
    #[serde(default)]
    pub(super) r#type: Option<String>,
    #[serde(default)]
    pub(super) all: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct ClipSyncBody {
    pub(super) peer: Option<String>,
    pub(super) text: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct ClipPolicyBody {
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default)]
    pub(super) sensitive_guard: Option<bool>,
    #[serde(default)]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) max_payload_bytes: Option<usize>,
    #[serde(default)]
    pub(super) auto_clear_timeout_secs: Option<u64>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct HandoffSendBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) url: String,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default, alias = "auto_open_requested")]
    pub(super) auto_open: Option<bool>,
    #[serde(default)]
    pub(super) tag: Option<String>,
    #[serde(default)]
    pub(super) group: Option<String>,
    #[serde(default)]
    pub(super) device_type: Option<String>,
    #[serde(default)]
    pub(super) r#type: Option<String>,
    #[serde(default)]
    pub(super) all: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct HandoffInboxActionBody {
    #[serde(default)]
    pub(super) id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct HandoffPolicyBody {
    #[serde(default)]
    pub(super) action: Option<String>,
    #[serde(default)]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) max_inbox_entries: Option<usize>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct MediaCommandBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) target: Option<String>,
    #[serde(default)]
    pub(super) command: Option<String>,
    #[serde(default)]
    pub(super) action: Option<String>,
    #[serde(default)]
    pub(super) volume: Option<u8>,
    #[serde(default, alias = "volume_percent")]
    pub(super) volume_percent: Option<u8>,
    #[serde(default)]
    pub(super) offset_ms: Option<i64>,
    #[serde(default)]
    pub(super) target_player: Option<String>,
    #[serde(default)]
    pub(super) tag: Option<String>,
    #[serde(default)]
    pub(super) group: Option<String>,
    #[serde(default)]
    pub(super) device_type: Option<String>,
    #[serde(default)]
    pub(super) r#type: Option<String>,
    #[serde(default)]
    pub(super) all: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct MediaStateUpdateBody {
    #[serde(default)]
    pub(super) player_id: Option<String>,
    #[serde(default)]
    pub(super) state: Option<String>,
    #[serde(default)]
    pub(super) title: Option<String>,
    #[serde(default)]
    pub(super) artist: Option<String>,
    #[serde(default)]
    pub(super) album: Option<String>,
    #[serde(default, alias = "position", alias = "pos")]
    pub(super) position_ms: Option<u64>,
    #[serde(default, alias = "duration", alias = "dur")]
    pub(super) duration_ms: Option<u64>,
    #[serde(default, alias = "volume", alias = "vol")]
    pub(super) volume_percent: Option<u8>,
    #[serde(default)]
    pub(super) muted: Option<bool>,
    #[serde(default)]
    pub(super) can_play: Option<bool>,
    #[serde(default)]
    pub(super) can_pause: Option<bool>,
    #[serde(default)]
    pub(super) can_go_next: Option<bool>,
    #[serde(default)]
    pub(super) can_go_previous: Option<bool>,
    #[serde(default)]
    pub(super) can_seek: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct MediaPolicyBody {
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default)]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) max_volume_limit: Option<u8>,
    #[serde(default)]
    pub(super) allow_seek: Option<bool>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct BatteryStateUpdateBody {
    #[serde(default, alias = "level", alias = "battery_percent", alias = "percent")]
    pub(super) battery_level: Option<u8>,
    #[serde(default, alias = "charging")]
    pub(super) is_charging: Option<bool>,
    #[serde(default, alias = "source", alias = "source_type")]
    pub(super) power_source: Option<String>,
    #[serde(default, alias = "saver", alias = "low_power_mode")]
    pub(super) battery_saver: Option<bool>,
    #[serde(default, alias = "health")]
    pub(super) health_percent: Option<u8>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct BatteryPolicyBody {
    #[serde(default, alias = "low_threshold", alias = "low")]
    pub(super) low_battery_threshold: Option<u8>,
    #[serde(default, alias = "critical_threshold", alias = "critical")]
    pub(super) critical_battery_threshold: Option<u8>,
    #[serde(default, alias = "alert")]
    pub(super) alert_on_low_battery: Option<bool>,
    #[serde(default, alias = "throttle", alias = "auto_throttle")]
    pub(super) auto_throttle_on_battery_saver: Option<bool>,
    #[serde(default)]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct LocateRingBody {
    #[serde(default, alias = "target")]
    pub(super) peer: Option<String>,
    #[serde(default, alias = "duration")]
    pub(super) duration_secs: Option<u32>,
    #[serde(default)]
    pub(super) pattern: Option<String>,
    #[serde(default)]
    pub(super) override_silent: Option<bool>,
    #[serde(default)]
    pub(super) message: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct LocateStopBody {
    #[serde(default, alias = "target")]
    pub(super) peer: Option<String>,
    #[serde(default, alias = "id")]
    pub(super) request_id: Option<String>,
    #[serde(default)]
    pub(super) reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct LocatePolicyBody {
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default)]
    pub(super) allow_override_silent: Option<bool>,
    #[serde(default, alias = "max_duration")]
    pub(super) max_duration_secs: Option<u32>,
    #[serde(default)]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct ActionLocalBody {
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) icon: Option<String>,
    #[serde(default)]
    pub(super) command: Option<String>,
    #[serde(default)]
    pub(super) args: Option<Vec<String>>,
    #[serde(default, alias = "timeout")]
    pub(super) timeout_secs: Option<u32>,
    #[serde(default)]
    pub(super) enabled: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct ActionTriggerBody {
    #[serde(default, alias = "target")]
    pub(super) peer: Option<String>,
    #[serde(default, alias = "id")]
    pub(super) action_id: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct ActionPolicyBody {
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default, alias = "timeout", alias = "max_timeout")]
    pub(super) max_timeout_secs: Option<u32>,
    #[serde(default)]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default, alias = "expose")]
    pub(super) expose_action_list: Option<bool>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct FocusSetBody {
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default, alias = "duration", alias = "duration_minutes")]
    pub(super) duration_mins: Option<u32>,
    #[serde(default)]
    pub(super) reason: Option<String>,
    #[serde(default)]
    pub(super) broadcast: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct FocusPolicyBody {
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default)]
    pub(super) default_mode: Option<String>,
    #[serde(default, alias = "mirror")]
    pub(super) mirror_inbound_focus: Option<bool>,
    #[serde(default, alias = "breakthrough")]
    pub(super) allow_emergency_breakthrough: Option<bool>,
    #[serde(default, alias = "quiet_hours")]
    pub(super) quiet_hours_enabled: Option<bool>,
    #[serde(default, alias = "start_minute")]
    pub(super) quiet_hours_start_minute: Option<u16>,
    #[serde(default, alias = "end_minute")]
    pub(super) quiet_hours_end_minute: Option<u16>,
    #[serde(default, alias = "start")]
    pub(super) quiet_hours_start: Option<String>,
    #[serde(default, alias = "end")]
    pub(super) quiet_hours_end: Option<String>,
    #[serde(default)]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(default)]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct TopologyLinkBody {
    #[serde(default)]
    pub(super) edge: Option<String>,
    #[serde(default, alias = "peer_id", alias = "device_id", alias = "target")]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) peer_edge: Option<String>,
    #[serde(default)]
    pub(super) offset: Option<i32>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct TopologyUnlinkBody {
    #[serde(default)]
    pub(super) edge: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct TopologyConfigBody {
    #[serde(default, alias = "resistance")]
    pub(super) resistance_px: Option<u32>,
    #[serde(default, alias = "delay")]
    pub(super) delay_ms: Option<u32>,
    #[serde(default, alias = "corner_lock", alias = "corner_lock_size")]
    pub(super) corner_lock_size_px: Option<u32>,
    #[serde(default, alias = "wrap")]
    pub(super) wrap_around: Option<bool>,
    #[serde(default)]
    pub(super) width: Option<u32>,
    #[serde(default)]
    pub(super) height: Option<u32>,
    #[serde(default, alias = "scale")]
    pub(super) scale_factor: Option<f32>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct TopologyPolicyBody {
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default, alias = "allow_inbound")]
    pub(super) allow_inbound_control: Option<bool>,
    #[serde(default, alias = "auto_link")]
    pub(super) auto_link_paired_peers: Option<bool>,
    #[serde(default, alias = "group", alias = "groups")]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default, alias = "tag", alias = "tags")]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(
        default,
        alias = "type",
        alias = "types",
        alias = "device_type",
        alias = "device_types"
    )]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct InputTunnelAttachBody {
    #[serde(default, alias = "peer_id", alias = "device_id", alias = "target")]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) edge: Option<String>,
    #[serde(default)]
    pub(super) x: Option<i32>,
    #[serde(default)]
    pub(super) y: Option<i32>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct InputTunnelReleaseBody {
    #[serde(default)]
    pub(super) reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct InputTunnelPolicyBody {
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default, alias = "allow_inbound")]
    pub(super) allow_inbound_control: Option<bool>,
    #[serde(default, alias = "keyboard")]
    pub(super) allow_keyboard: Option<bool>,
    #[serde(default, alias = "mouse")]
    pub(super) allow_mouse: Option<bool>,
    #[serde(default, alias = "hotkey", alias = "hotkey_release")]
    pub(super) hotkey_release_enabled: Option<bool>,
    #[serde(default, alias = "timeout", alias = "idle_timeout")]
    pub(super) idle_timeout_secs: Option<u32>,
    #[serde(default, alias = "group", alias = "groups")]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default, alias = "tag", alias = "tags")]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(
        default,
        alias = "type",
        alias = "types",
        alias = "device_type",
        alias = "device_types"
    )]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct VirtualDisplayAttachBody {
    #[serde(default, alias = "peer_id", alias = "device_id", alias = "target")]
    pub(super) peer: Option<String>,
    #[serde(default, alias = "assigned_edge")]
    pub(super) edge: Option<String>,
    #[serde(default)]
    pub(super) width: Option<u32>,
    #[serde(default)]
    pub(super) height: Option<u32>,
    #[serde(default, alias = "target_fps", alias = "refresh_rate")]
    pub(super) fps: Option<u32>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct VirtualDisplayDetachBody {
    #[serde(default)]
    pub(super) reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct VirtualDisplayPolicyBody {
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default, alias = "host", alias = "allow_host")]
    pub(super) allow_hosting: Option<bool>,
    #[serde(default, alias = "extend", alias = "allow_extend")]
    pub(super) allow_remote_extend: Option<bool>,
    #[serde(default, alias = "max_width")]
    pub(super) max_resolution_width: Option<u32>,
    #[serde(default, alias = "max_height")]
    pub(super) max_resolution_height: Option<u32>,
    #[serde(default)]
    pub(super) max_fps: Option<u32>,
    #[serde(default, alias = "timeout", alias = "idle_timeout")]
    pub(super) idle_timeout_secs: Option<u32>,
    #[serde(default, alias = "group", alias = "groups")]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default, alias = "tag", alias = "tags")]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(
        default,
        alias = "type",
        alias = "types",
        alias = "device_type",
        alias = "device_types"
    )]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default, Clone, Copy)]
pub(super) struct AudioSpatialPositionReq {
    #[serde(default)]
    pub(super) x: Option<f32>,
    #[serde(default)]
    pub(super) y: Option<f32>,
    #[serde(default)]
    pub(super) z: Option<f32>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct AudioRoutingAttachBody {
    #[serde(default, alias = "peer_id", alias = "device_id", alias = "target")]
    pub(super) peer: Option<String>,
    #[serde(default, alias = "channel_layout")]
    pub(super) layout: Option<String>,
    #[serde(default)]
    pub(super) sample_rate: Option<u32>,
    #[serde(default)]
    pub(super) channels: Option<u8>,
    #[serde(default, alias = "x")]
    pub(super) spatial_x: Option<f32>,
    #[serde(default, alias = "y")]
    pub(super) spatial_y: Option<f32>,
    #[serde(default, alias = "z")]
    pub(super) spatial_z: Option<f32>,
    #[serde(default)]
    pub(super) spatial_position: Option<AudioSpatialPositionReq>,
    #[serde(default, alias = "gain", alias = "volume")]
    pub(super) volume_gain: Option<f32>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct AudioRoutingDetachBody {
    #[serde(default)]
    pub(super) reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct AudioRoutingPolicyBody {
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default, alias = "sink", alias = "allow_sink")]
    pub(super) allow_inbound_sink: Option<bool>,
    #[serde(default, alias = "routing", alias = "allow_outbound")]
    pub(super) allow_outbound_routing: Option<bool>,
    #[serde(default, alias = "max_volume", alias = "volume_limit")]
    pub(super) max_volume_limit: Option<u8>,
    #[serde(
        default,
        alias = "spatial",
        alias = "allow_spatial",
        alias = "spatial_rendering_enabled"
    )]
    pub(super) allow_spatial_rendering: Option<bool>,
    #[serde(default, alias = "timeout", alias = "idle_timeout")]
    pub(super) idle_timeout_secs: Option<u32>,
    #[serde(default, alias = "group", alias = "groups")]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default, alias = "tag", alias = "tags")]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(
        default,
        alias = "type",
        alias = "types",
        alias = "device_type",
        alias = "device_types"
    )]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct CameraAttachBody {
    #[serde(default, alias = "peer_id", alias = "device_id", alias = "target")]
    pub(super) peer: Option<String>,
    #[serde(default, alias = "lens")]
    pub(super) lens_id: Option<String>,
    #[serde(default)]
    pub(super) width: Option<u32>,
    #[serde(default)]
    pub(super) height: Option<u32>,
    #[serde(default, alias = "target_fps")]
    pub(super) fps: Option<u32>,
    #[serde(default)]
    pub(super) codec: Option<String>,
    #[serde(default, alias = "torch")]
    pub(super) flashlight: Option<bool>,
    #[serde(default, alias = "zoom_ratio")]
    pub(super) zoom: Option<f32>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct CameraDetachBody {
    #[serde(default)]
    pub(super) reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct CameraPolicyBody {
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default, alias = "inbound", alias = "allow_inbound")]
    pub(super) allow_inbound_stream: Option<bool>,
    #[serde(default, alias = "capture", alias = "allow_capture")]
    pub(super) allow_remote_capture: Option<bool>,
    #[serde(default, alias = "max_width")]
    pub(super) max_resolution_width: Option<u32>,
    #[serde(default, alias = "max_height")]
    pub(super) max_resolution_height: Option<u32>,
    #[serde(default, alias = "fps")]
    pub(super) max_fps: Option<u32>,
    #[serde(default, alias = "privacy_shutter", alias = "enforce_privacy_shutter")]
    pub(super) enforce_privacy_shutter_indicator: Option<bool>,
    #[serde(default, alias = "timeout", alias = "idle_timeout")]
    pub(super) idle_timeout_secs: Option<u32>,
    #[serde(default, alias = "group", alias = "groups")]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default, alias = "tag", alias = "tags")]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(
        default,
        alias = "type",
        alias = "types",
        alias = "device_type",
        alias = "device_types"
    )]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct MicAttachBody {
    #[serde(default, alias = "peer_id", alias = "device_id", alias = "target")]
    pub(super) peer: Option<String>,
    #[serde(default, alias = "mic")]
    pub(super) mic_id: Option<String>,
    #[serde(default, alias = "session_mode")]
    pub(super) mode: Option<String>,
    #[serde(default)]
    pub(super) codec: Option<String>,
    #[serde(default, alias = "rate")]
    pub(super) sample_rate: Option<u32>,
    #[serde(default)]
    pub(super) channels: Option<u32>,
    #[serde(default)]
    pub(super) gain: Option<f32>,
    #[serde(default)]
    pub(super) muted: Option<bool>,
    #[serde(default, alias = "ns")]
    pub(super) noise_suppression: Option<bool>,
    #[serde(default, alias = "aec")]
    pub(super) echo_cancellation: Option<bool>,
    #[serde(default, alias = "agc")]
    pub(super) auto_gain: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct MicDetachBody {
    #[serde(default)]
    pub(super) reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct MicPolicyBody {
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default, alias = "inbound", alias = "allow_inbound")]
    pub(super) allow_inbound_stream: Option<bool>,
    #[serde(default, alias = "capture", alias = "allow_capture")]
    pub(super) allow_remote_capture: Option<bool>,
    #[serde(default, alias = "intercom", alias = "ptt")]
    pub(super) allow_intercom: Option<bool>,
    #[serde(default, alias = "max_rate")]
    pub(super) max_sample_rate: Option<u32>,
    #[serde(default, alias = "max_duration")]
    pub(super) max_session_duration_secs: Option<u32>,
    #[serde(default, alias = "privacy_indicator", alias = "enforce_privacy")]
    pub(super) enforce_privacy_indicator: Option<bool>,
    #[serde(default, alias = "start_muted")]
    pub(super) default_muted: Option<bool>,
    #[serde(default, alias = "timeout", alias = "idle_timeout")]
    pub(super) idle_timeout_secs: Option<u32>,
    #[serde(default, alias = "group", alias = "groups")]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default, alias = "tag", alias = "tags")]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(
        default,
        alias = "type",
        alias = "types",
        alias = "device_type",
        alias = "device_types"
    )]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct BenchStartBody {
    #[serde(default, alias = "peer_id", alias = "device_id", alias = "target")]
    pub(super) peer: Option<String>,
    #[serde(default, alias = "dir")]
    pub(super) direction: Option<String>,
    #[serde(default, alias = "duration", alias = "secs")]
    pub(super) duration_secs: Option<u32>,
    #[serde(default, alias = "chunk")]
    pub(super) chunk_size: Option<u32>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct BenchPolicyBody {
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default, alias = "inbound", alias = "allow_inbound")]
    pub(super) allow_inbound_bench: Option<bool>,
    #[serde(default, alias = "duration", alias = "max_duration")]
    pub(super) max_duration_secs: Option<u32>,
    #[serde(default, alias = "bandwidth", alias = "max_bandwidth")]
    pub(super) max_bandwidth_mbps: Option<u32>,
    #[serde(default, alias = "group", alias = "groups")]
    pub(super) allowed_groups: Option<Vec<String>>,
    #[serde(default, alias = "tag", alias = "tags")]
    pub(super) allowed_tags: Option<Vec<String>>,
    #[serde(
        default,
        alias = "type",
        alias = "types",
        alias = "device_type",
        alias = "device_types"
    )]
    pub(super) allowed_types: Option<Vec<String>>,
    #[serde(default)]
    pub(super) clear_filters: Option<bool>,
}

pub(super) fn parse_hh_mm(s: &str) -> Option<u16> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let h: u16 = parts[0].trim().parse().ok()?;
    let m: u16 = parts[1].trim().parse().ok()?;
    if h < 24 && m < 60 {
        Some(h * 60 + m)
    } else {
        None
    }
}

#[derive(serde::Deserialize, Default)]
pub(super) struct AuditQueryBody {
    #[serde(default)]
    pub(super) category: Option<String>,
    #[serde(default)]
    pub(super) severity: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    pub(super) since: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct PeerTargetBody {
    pub(super) peer: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct PeerConnectBody {
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) addr: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct PeerDisconnectBody {
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) peer: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct PeerBlockBody {
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) reason: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct PeerUnblockBody {
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) peer: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct TransfersQueryBody {
    pub(super) id: Option<String>,
    pub(super) active: Option<bool>,
    pub(super) terminal: Option<bool>,
    pub(super) direction: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct PeerAliasBody {
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) alias: String,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct ConfigUpdateBody {
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) advertise: Option<bool>,
    #[serde(default)]
    pub(super) reconnect: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct RouteAddBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) addr: String,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct PeerLinkProbeBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) device_id: Option<String>,
    #[serde(default)]
    pub(super) count: Option<usize>,
    #[serde(default)]
    pub(super) interval_ms: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct HeartbeatConfigBody {
    #[serde(default)]
    pub(super) interval_secs: Option<u64>,
    #[serde(default)]
    pub(super) deadline_secs: Option<u64>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct MessageSendBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) tag: Option<String>,
    #[serde(default)]
    pub(super) group: Option<String>,
    #[serde(default)]
    pub(super) device_type: Option<String>,
    #[serde(default)]
    pub(super) r#type: Option<String>,
    #[serde(default)]
    pub(super) all: Option<bool>,
    #[serde(default)]
    pub(super) text: String,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct MessageQueryBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
    #[serde(default)]
    pub(super) unread_only: Option<bool>,
    #[serde(default)]
    pub(super) mark_read: Option<bool>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct MessageClearBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct OutboxQueryBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) status: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<usize>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct OutboxClearBody {
    #[serde(default)]
    pub(super) peer: Option<String>,
    #[serde(default)]
    pub(super) status: Option<String>,
}

#[derive(serde::Deserialize, Default)]
pub(super) struct OutboxResendBody {
    #[serde(default)]
    pub(super) id: Option<String>,
    #[serde(default)]
    pub(super) transfer_id: Option<String>,
}
