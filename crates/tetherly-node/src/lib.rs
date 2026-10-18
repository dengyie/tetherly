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
pub use platform::{MemoryInsertor, MemoryOpener, SystemInputSink, SystemOpener};
pub use runtime::{
    extract_features, ActiveTransfer, AncsReport, AutoAcceptReport, BatchDismissReport,
    BlockedPeersReport, CandidateDismissReport, ClipClearReport, ClipHistoryReport,
    ClipMetaEntryDto, ClipStatusReport, DismissFilter, FileBatchDecisionReport,
    FileDecisionReport, FileOfferDto, FileOffersReport, HealthReport, LivePeer, LocalCapsReport,
    Node, NodeConfig, NodeConfigReport, NodePorts, PeerBlockReport, PeerCapsDto,
    PeerConnectReport, PeerDisconnectReport, PeerPingReport, PeerUnblockReport, PeersCapsReport,
    PinReport, RouteEntryDto, RouteMutateReport, RoutesReport, StorageCleanReport, StorageReport,
    TransferDirection, TransferPhase, TransferStats, TransfersReport, UserCommand,
};
pub use screen::{AdbSource, EncodedSource, SystemSink, SystemSource, ZstdCodec};
pub use telemetry::{ChannelTrafficDto, ProtocolMessagesDto, TrafficReport, TrafficTelemetry};

#[cfg(windows)]
pub use screen::DxgiSource;
#[cfg(target_os = "macos")]
pub use screen::MacSource;
#[cfg(all(unix, not(target_os = "macos")))]
pub use screen::X11Source;
pub use sidecar::{parse_overlay_cidrs, OverlayConfig, OverlaySource, OverlayStatus};
pub use store::{default_data_dir, BlockedPeer, BlockedTable, BLOCKED_FILE};
pub use uihttp::{
    loopback_delete, loopback_events_connect, loopback_get, loopback_post, loopback_put, SseStream,
};
