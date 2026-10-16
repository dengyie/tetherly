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
pub use filechan::{
    read_header as read_file_header, recv_chunks_after_header, recv_chunks_into,
    recv_chunks_to_sink, send_chunks_from, send_stream_from, CHUNK as FILE_CHUNK_NET,
    FILE_MAGIC_V2,
};
pub use mdns::SERVICE_TYPE;
pub use session::{
    accept_session, dial_session, serve_pong_once, ActiveSession, SessionConfig, CONTROL_PORT,
    FILE_PORT, HANDSHAKE_TIMEOUT, INPUT_PORT, SCREEN_PORT, SESSION_WRITE_TIMEOUT,
};
