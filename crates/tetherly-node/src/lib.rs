// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 1 LAN node: persist identity/trust, notify/clip/file hubs, reconnect.

// Win32 insert in platform.rs is the only unsafe. Core/net stay safe.

pub mod ancs;
pub mod cred;
pub mod events;
pub mod lan;
pub mod platform;
pub mod runtime;
pub mod screen;
pub mod sidecar;
pub mod store;
pub mod telemetry;
pub mod uihttp;

pub use events::{CandidateViewDto, UiEvent};

#[cfg(windows)]
pub use platform::{cursor_pos, screen_size, start_input_hooks, HookControl, RawInput};
pub use platform::{
    HostSample, MemoryInsertor, MemoryNotifier, MemoryOpener, MemorySysProbe, PlatformSysProbe,
    ProcessSample, SysProbe, SystemInputSink, SystemNotifier, SystemOpener,
};
pub use runtime::{
    extract_features, format_memory_bytes, mime_type_from_name, ActiveTransfer,
    AncsClearRulesReport, AncsConnectReport, AncsDisconnectReport, AncsReport, AncsRuleReport,
    AuditCategory, AuditClearReport, AuditEngine, AuditEntry, AuditLogReport, AuditSeverity,
    AutoAcceptPolicyReport, AutoAcceptReport, BatchDismissReport, BatchSendItem, BatchSendReport,
    BatchSendRequest, BatchTransferItemReport, BlockedPeersReport, CandidateDismissReport,
    ClipClearReport, ClipHistoryReport, ClipMetaEntryDto, ClipStatusReport, ClipboardPolicyReport,
    DiscoveredPeerDto, DiscoveryReport, DiscoveryRescanReport, DismissFilter, DndMode,
    FileBatchDecisionReport, FileDecisionReport, FileOfferDto, FileOffersReport, HealthReport,
    HeartbeatConfig, HostMetrics, IfaceReportDto, InboxClearReport, InboxDeleteReport,
    InboxFileDto, InboxOpenReport, InboxReport, InputControlReport, InputStatusReport, LinkGrade,
    LinkProbeReport, LinkQualityReport, LivePeer, LocalCapsReport, MessageDirection, MessageEngine,
    MultiFileSendReport, MultiMessageSendReport, NetworkRefreshReport, NetworkReport, Node,
    NodeConfig, NodeConfigReport, NodePorts, NotifyBannerConfig, NotifyConfigReport,
    NotifyTestReport, NotifyTestRequest, OpenRuleDto, OutboxClearReport, OutboxEngine, OutboxEntry,
    OutboxReport, OutboxResendReport, OutboxStatus, OverlayStatusReport, PeerBlockReport,
    PeerCapsDto, PeerConnectReport, PeerDisconnectReport, PeerFileSendItem, PeerGroupReport,
    PeerLinkQualityDto, PeerLinkTracker, PeerMessage, PeerMessageClearReport,
    PeerMessageHistoryReport, PeerMessageSendItem, PeerMessageSendReport, PeerPingReport,
    PeerProfileUpdateReport, PeerProfilesReport, PeerTagsReport, PeerUnblockReport,
    PeersCapsReport, PinReport, ProcessMetrics, ResumeMarkerDto, ResumeMarkersReport,
    RouteEntryDto, RouteMutateReport, RoutesReport, ScreenBitrateReport, ScreenControlReport,
    ScreenFpsReport, ScreenStatusReport, StorageCleanReport, StorageReport, SystemReport,
    TransferDirection, TransferPhase, TransferStats, TransfersReport, UserCommand,
    VirtualDisplayDecisionReport, VirtualDisplayPolicyReport, VirtualDisplayPolicyUpdate,
    VirtualDisplayStatusReport,
};
pub use screen::{
    screen_bitrate_from_profile, screen_fps_from_profile, AdbSource, EncodedSource, H264Codec,
    NegotiatedCodec, SystemSink, SystemSource, ZstdCodec,
};
pub use telemetry::{ChannelTrafficDto, ProtocolMessagesDto, TrafficReport, TrafficTelemetry};

#[cfg(windows)]
pub use screen::DxgiSource;
#[cfg(target_os = "macos")]
pub use screen::MacSource;
#[cfg(all(unix, not(target_os = "macos")))]
pub use screen::X11Source;
pub use sidecar::{parse_overlay_cidrs, OverlayConfig, OverlaySource, OverlayStatus};
pub use store::{
    default_data_dir, load_audio_routing_policy, load_auto_accept_policy, load_battery_policy,
    load_clipboard_policy, load_locator_policy, load_profiles, load_virtual_display_policy,
    save_audio_routing_policy, save_auto_accept_policy, save_battery_policy, save_clipboard_policy,
    save_locator_policy, save_profiles, save_virtual_display_policy, AudioRoutingPolicy,
    AutoAcceptPolicy, BatteryPolicy, BlockedPeer, BlockedTable, ClipboardPolicy, DeviceType,
    LocatorPolicy, PeerProfile, ProfileTable, TargetSelector, VirtualDisplayPolicy,
    AUDIO_ROUTING_POLICY_FILE, AUTO_ACCEPT_FILE, BATTERY_POLICY_FILE, BLOCKED_FILE,
    CLIPBOARD_POLICY_FILE, DEFAULT_MAX_CLIP_AUTO_SYNC_BYTES, LOCATOR_POLICY_FILE, PROFILES_FILE,
    VIRTUAL_DISPLAY_POLICY_FILE,
};
pub use uihttp::{
    loopback_delete, loopback_events_connect, loopback_get, loopback_post, loopback_put, SseStream,
    UiResponse,
};
