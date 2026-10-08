// SPDX-License-Identifier: Apache-2.0 OR MIT
//! TCP Hello / SPAKE2 / Noise. Optional LAN mDNS. No EasyTier crate.

#![forbid(unsafe_code)]

pub mod codec;
pub mod error;
pub mod mdns;
pub mod session;

pub use error::NetError;
pub use session::{
    accept_session, dial_session, serve_pong_once, ActiveSession, SessionConfig, CONTROL_PORT,
    FILE_PORT, HANDSHAKE_TIMEOUT, INPUT_PORT,
};
