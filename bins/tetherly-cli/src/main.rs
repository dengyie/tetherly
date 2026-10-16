// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 0 CLI: listen / dial / pair / ping. No GUI, no BLE, no EasyTier.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tetherly_core::pin::{format_pin, generate_pin, parse_pin};
use tetherly_core::ports::Rng;
use tetherly_core::session::{backoff_delay, LockoutTable};
use tetherly_core::{Clock, CoreError, SystemClock, TcpLimiter};
use tetherly_crypto::Identity;
use tetherly_net::session::{
    accept_session, dial_session, serve_pong_once, SessionConfig, CONTROL_PORT, HANDSHAKE_TIMEOUT,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "tetherly-cli", about = "Tetherly Phase 0 pairing and ping")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print a new identity device_id (ephemeral; not persisted in Phase 0 CLI).
    Identity,
    /// Display an 8-digit PIN grouped as `2517 0394`.
    Pin,
    /// Listen on 127.0.0.1:port, pair with the given PIN, then answer one ping.
    Listen {
        #[arg(long, default_value_t = CONTROL_PORT)]
        port: u16,
        #[arg(long)]
        pin: String,
        #[arg(long, default_value = "host")]
        name: String,
    },
    /// Dial 127.0.0.1:port, pair, ping, print pong timestamp.
    Dial {
        #[arg(long)]
        port: u16,
        #[arg(long)]
        pin: String,
        #[arg(long, default_value = "peer")]
        name: String,
        #[arg(long, default_value_t = 1)]
        reconnect: u32,
    },
    /// Send a file to a live peer via the running daemon's loopback UI API.
    Send {
        /// Path to the file to send.
        path: std::path::PathBuf,
        /// Optional filename override to declare to the peer.
        #[arg(long)]
        name: Option<String>,
        /// `tdev_…` of the peer. Omit only when exactly one peer is live.
        #[arg(long)]
        peer: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

struct OsRng;

impl Rng for OsRng {
    fn fill_bytes(&self, dest: &mut [u8]) -> Result<(), CoreError> {
        getrandom::getrandom(dest).map_err(|_| CoreError::Rng)
    }
}

fn now_ms() -> u64 {
    SystemClock.unix_ms()
}

fn cfg(identity: Identity, name: &str, pin: [u8; 8]) -> SessionConfig {
    SessionConfig {
        identity,
        name: name.to_string(),
        platform: std::env::consts::OS.to_string(),
        trust: Arc::new(Mutex::new(Default::default())),
        lockout: Arc::new(Mutex::new(LockoutTable::default())),
        tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
        pin: Some(pin),
        pin_created_ms: now_ms(),
        clock: Arc::new(SystemClock),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: false,
        screen_only: false,
        caps: None,
    }
}

async fn loopback_post(port: u16, path: &str, body: &[u8]) -> std::io::Result<(u16, Vec<u8>)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await?;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    let text = String::from_utf8_lossy(&buf);
    let code = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let payload = text
        .split("\r\n\r\n")
        .nth(1)
        .map(|b| b.as_bytes().to_vec())
        .unwrap_or_default();
    Ok((code, payload))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("tetherly=info".parse().unwrap()),
        )
        .init();
    if std::env::var("TETHERLY_INSECURE_LOG").ok().as_deref() == Some("1") {
        warn!("TETHERLY_INSECURE_LOG=1 is set; debug profile only");
    }

    match Cli::parse().cmd {
        Cmd::Identity => {
            let id = Identity::generate().context("identity")?;
            println!("{}", id.device_id());
        }
        Cmd::Pin => {
            let pin = generate_pin(&OsRng)?;
            println!("{}", format_pin(&pin));
        }
        Cmd::Listen { port, pin, name } => {
            let pin = parse_pin(&pin)?;
            let identity = Identity::generate()?;
            info!(device_id = %identity.device_id(), port, "listen");
            let listener = TcpListener::bind(("127.0.0.1", port)).await?;
            let (stream, addr) = listener.accept().await?;
            let cfg = cfg(identity, &name, pin);
            let mut sess = accept_session(stream, addr, &cfg).await?;
            info!(peer = %sess.peer_id, "paired; waiting for ping");
            serve_pong_once(&mut sess).await?;
        }
        Cmd::Dial {
            port,
            pin,
            name,
            reconnect,
        } => {
            let pin = parse_pin(&pin)?;
            let identity = Identity::generate()?;
            let addr: SocketAddr = format!("127.0.0.1:{port}").parse()?;
            let mut attempt = 0u32;
            loop {
                match TcpStream::connect(addr).await {
                    Ok(stream) => {
                        let cfg = cfg(identity, &name, pin);
                        let mut sess = dial_session(stream, addr, &cfg).await?;
                        let ts = now_ms();
                        let echoed = sess.ping_pong(ts).await?;
                        println!("pong {echoed}");
                        break;
                    }
                    Err(e) => {
                        attempt += 1;
                        if attempt > reconnect {
                            bail!("dial failed: {e}");
                        }
                        let delay = backoff_delay(attempt - 1);
                        error!(%e, attempt, "connect failed; backoff");
                        tokio::time::sleep(delay.min(Duration::from_secs(5))).await;
                    }
                }
            }
        }
        Cmd::Send {
            path,
            name,
            peer,
            ui_port,
        } => {
            let path_str = path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("path contains invalid UTF-8"))?;
            let payload = serde_json::json!({
                "path": path_str,
                "name": name,
                "peer": peer,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/file/send", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "node refused file send (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let transfer_id = v.get("transfer_id").and_then(|p| p.as_str()).unwrap_or("?");
            let target_peer = v.get("peer").and_then(|p| p.as_str()).unwrap_or("?");
            let file_name = v.get("name").and_then(|p| p.as_str()).unwrap_or("?");
            let size = v.get("size").and_then(|p| p.as_u64()).unwrap_or(0);
            println!("offered {file_name} ({size} bytes) to {target_peer} [{transfer_id}]");
        }
    }
    Ok(())
}
