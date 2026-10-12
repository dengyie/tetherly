// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 1 LAN node: persist identity/trust, notify/clip/file hubs, reconnect.

// Win32 insert in platform.rs is the only unsafe. Core/net stay safe.

pub mod cred;
pub mod events;
pub mod lan;
pub mod platform;
pub mod runtime;
pub mod screen;
pub mod sidecar;
pub mod store;
pub mod uihttp;

pub use events::{CandidateViewDto, UiEvent};

#[cfg(windows)]
pub use platform::{cursor_pos, screen_size, start_input_hooks, HookControl, RawInput};
pub use platform::{MemoryInsertor, MemoryOpener, SystemInputSink, SystemOpener};
pub use runtime::{LivePeer, Node, NodeConfig, UserCommand};
pub use screen::{AdbSource, DxgiSource, EncodedSource, SystemSink, SystemSource, ZstdCodec};
pub use sidecar::{parse_overlay_cidrs, OverlayConfig, OverlaySource, OverlayStatus};
pub use store::default_data_dir;
