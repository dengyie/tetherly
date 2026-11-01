// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 0 CLI: listen / dial / pair / ping. No GUI, no BLE, no EasyTier.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tetherly_core::pin::parse_pin;
use tetherly_core::session::LockoutTable;
use tetherly_core::{Clock, SystemClock, TcpLimiter};
use tetherly_crypto::Identity;
use tetherly_net::session::{
    accept_session, serve_pong_once, SessionConfig, CONTROL_PORT, HANDSHAKE_TIMEOUT,
};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{info, warn};

// ---- CLI surface (v0.1.65 / ADR-129) ----
mod cli;
mod cli_device;
mod cli_ops;
mod cli_screen;
mod cli_tools;
pub(crate) use cli::*;
pub(crate) use cli_device::*;
pub(crate) use cli_ops::*;
pub(crate) use cli_screen::*;
pub(crate) use cli_tools::*;

// ---- dispatch + command handlers (v0.1.65 / ADR-129) ----
mod cmd_action;
mod cmd_ancs;
mod cmd_audio;
mod cmd_audit;
mod cmd_bench;
mod cmd_camera;
mod cmd_clip;
mod cmd_core;
mod cmd_display;
mod cmd_files;
mod cmd_focus;
mod cmd_handoff;
mod cmd_input;
mod cmd_locate;
mod cmd_media;
mod cmd_messages;
mod cmd_mic;
mod cmd_notify;
mod cmd_peers;
mod cmd_power;
mod cmd_profile;
mod cmd_screen;
mod cmd_screen_encoding;
mod cmd_screen_overlay;
mod cmd_screen_quality;
mod cmd_screen_replay;
mod cmd_screen_session;
mod cmd_screen_tile;
mod cmd_topology;
mod dispatch;
mod util;
use dispatch::dispatch;

// ---- SSE stream shared by the loopback event helpers (util module) ----

struct SseStream {
    lines: tokio::io::Lines<tokio::io::BufReader<TcpStream>>,
}

impl SseStream {
    async fn next_event(&mut self) -> std::io::Result<Option<String>> {
        while let Some(line) = self.lines.next_line().await? {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with(':') {
                continue;
            }
            if let Some(payload) = trimmed.strip_prefix("data:") {
                return Ok(Some(payload.trim().to_string()));
            }
        }
        Ok(None)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // process quietly like any classic tool, not panic in println!.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("tetherly=info".parse().unwrap())
                // mdns-sd spams ERROR "Cannot find valid addrs for TYPE_SRV"
                // on IPv6-only and TUN interfaces (sandbox-VPS soak logged it
                // continuously); it is interface-enumeration noise, not a
                // discovery failure — default it off so real errors surface.
                // RUST_LOG still overrides for debugging.
                .add_directive("mdns_sd=off".parse().unwrap()),
        )
        .init();
    if std::env::var("TETHERLY_INSECURE_LOG").ok().as_deref() == Some("1") {
        warn!("TETHERLY_INSECURE_LOG=1 is set; debug profile only");
    }

    let cli = Cli::parse();

    dispatch(cli).await
}
