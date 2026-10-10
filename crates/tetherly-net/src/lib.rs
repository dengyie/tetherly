// SPDX-License-Identifier: Apache-2.0 OR MIT
//! TCP Hello / SPAKE2 / Noise. Optional LAN mDNS. No EasyTier crate.

#![forbid(unsafe_code)]

pub mod codec;
pub mod dispatch;
pub mod error;
pub mod filechan;
pub mod mdns;
pub mod session;

pub use dispatch::SessionEvent;
pub use error::NetError;
pub use filechan::{recv_bytes as recv_file_bytes, send_bytes as send_file_bytes, token_for};
pub use mdns::{LanPeer, SERVICE_TYPE};
pub use session::{
    accept_session, dial_session, serve_pong_once, ActiveSession, SessionConfig, CONTROL_PORT,
    FILE_PORT, HANDSHAKE_TIMEOUT, INPUT_PORT, SCREEN_PORT,
};
