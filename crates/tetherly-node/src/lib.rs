// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 1 LAN node: persist identity/trust, notify/clip/file hubs, reconnect.

// Win32 insert in platform.rs is the only unsafe. Core/net stay safe.

pub mod events;
pub mod lan;
pub mod platform;
pub mod runtime;
pub mod sidecar;
pub mod store;
pub mod uihttp;

pub use events::{CandidateViewDto, UiEvent};
pub use platform::{MemoryInsertor, MemoryOpener, SystemOpener};
pub use runtime::{LivePeer, Node, NodeConfig, UserCommand};
pub use sidecar::{parse_overlay_cidrs, OverlayConfig, OverlaySource, OverlayStatus};
pub use store::default_data_dir;
