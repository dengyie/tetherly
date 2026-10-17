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
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
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
    /// Send a user-facing notification to a live peer.
    Notify {
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
        #[arg(long, default_value = "com.example.sms")]
        app_id: String,
        /// `tdev_…` of the peer. Omit only when exactly one peer is live.
        #[arg(long)]
        peer: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
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
        /// Watch transfer progress until completion.
        #[arg(long)]
        watch: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Cancel an in-flight file transfer by its transfer ID.
    Cancel {
        /// Transfer ID to cancel (e.g. tfl_...).
        id: String,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clear completed and failed file transfers from history.
    ClearTransfers {
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Unpair and revoke trust for a paired peer device by device_id.
    Unpair {
        /// Device ID of the peer to unpair (tdev_...).
        device_id: String,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage and synchronize clipboard with paired peers.
    Clip {
        #[command(subcommand)]
        action: ClipCmd,
    },
    /// List active notification candidates.
    Candidates {
        /// Output raw JSON instead of table.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Get or copy the OTP verification code from an active candidate.
    Otp {
        /// Candidate ID (e.g. ntc_...).
        id: String,
        /// Also copy the OTP to the local system clipboard.
        #[arg(long)]
        copy: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Dismiss an active notification candidate.
    Dismiss {
        /// Candidate ID (e.g. ntc_...).
        id: String,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Send a quick reply to the mobile device for a notification candidate.
    Reply {
        /// Candidate ID (e.g. ntc_...).
        id: String,
        /// Reply message text to send.
        text: String,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// List queued inbound replies awaiting OS delivery.
    Replies {
        /// Output raw JSON instead of table.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Take and consume an inbound quick reply by ID (single-use).
    TakeReply {
        /// Reply ID (e.g. ntr_...).
        id: String,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage remote screen viewing (allow, revoke, start, stop, status).
    Screen {
        #[command(subcommand)]
        action: ScreenCmd,
    },
    /// Manage remote keyboard and mouse control (start, stop, status).
    Input {
        #[command(subcommand)]
        action: InputCmd,
    },
    /// List connected and trusted peer devices, or probe peer latency.
    Peers {
        /// Include offline trusted devices.
        #[arg(long, short)]
        trusted: bool,
        /// Probe peer round-trip latency (RTT) over the encrypted Noise session.
        #[arg(long)]
        ping: bool,
        /// Specific peer to target for ping probe (defaults to single connected peer).
        #[arg(long)]
        peer: Option<String>,
        /// Number of ping probes to send when probing latency.
        #[arg(long, default_value_t = 1)]
        count: u32,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// List cached physical addressing routes.
    Routes {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Diagnose network interfaces and EasyTier overlay sidecar.
    Net {
        /// Show full interface classification details (including skipped interfaces and reasons).
        #[arg(long, short)]
        interfaces: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage Apple Notification Center Service (status, connect, allow).
    Ancs {
        #[command(subcommand)]
        action: AncsCmd,
    },
    /// Manage local storage, inbox directory, and partial transfer files.
    Storage {
        #[command(subcommand)]
        action: StorageCmd,
    },
    /// Query status of the *running* daemon (peers, transfers, and listen interfaces).
    Status {
        /// Watch mode: refresh status periodically until interrupted.
        #[arg(long, short)]
        watch: bool,
        /// Refresh interval in seconds for --watch (default: 1).
        #[arg(long, default_value_t = 1)]
        interval: u64,
        /// Output raw JSON snapshot instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Check health, uptime, and bound ports of the running node daemon.
    Health {
        /// Output raw JSON health report instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Fast ping to verify node daemon responsiveness or probe a connected peer's Noise session RTT.
    Ping {
        /// Optional peer ID to probe (e.g. tdev_...). When omitted, probes the local daemon loopback.
        #[arg(long)]
        peer: Option<String>,
        /// Number of ping probes to send (default: 1).
        #[arg(long, default_value_t = 1)]
        count: u32,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Gracefully stop the running node daemon.
    Stop {
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stream real-time node events via loopback Server-Sent Events (SSE).
    Events {
        /// Maximum number of events to receive before exiting (0 for unlimited).
        #[arg(long, default_value_t = 0)]
        count: usize,
        /// Timeout in seconds to wait for events (0 for no timeout).
        #[arg(long, default_value_t = 0)]
        timeout_secs: u64,
        /// Output raw JSON per event line instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum ClipCmd {
    /// Sync the current local clipboard to connected peers.
    Sync {
        /// Target peer device_id (tdev_...). Omit to broadcast to all live peers.
        #[arg(long)]
        peer: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Send explicit text to connected peers' clipboards.
    Send {
        /// Text content to send. If omitted, read from stdin.
        text: Option<String>,
        /// Target peer device_id (tdev_...). Omit to broadcast to all live peers.
        #[arg(long)]
        peer: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Get the current local clipboard content from the running daemon.
    Get {
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum ScreenCmd {
    /// Allow the local screen to be viewed by a connected visitor (local consent).
    Allow {
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Revoke local screen viewing consent.
    Revoke {
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Start viewing a remote peer's screen (as visitor).
    Start {
        /// Target peer device_id (tdev_...). Required if multiple peers are connected.
        #[arg(long)]
        peer: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stop screen streaming.
    Stop {
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Show remote screen channel status, counters and frame intervals.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum InputCmd {
    /// Connect and start remote keyboard/mouse channel to a peer.
    Start {
        /// Target peer device_id (tdev_...). Required if multiple peers are connected.
        #[arg(long)]
        peer: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stop remote keyboard/mouse channel and return cursor locally.
    Stop {
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Show remote keyboard/mouse channel status and cursor seat.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum AncsCmd {
    /// Show ANCS ingress status, connection state, and active open allowlist.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Trigger connection or re-subscription to iPhone ANCS GATT characteristics.
    Connect {
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Add an application ID and optional URL template to the notification open allowlist.
    Allow {
        /// Target application bundle ID (e.g. com.tencent.xin).
        app_id: String,
        /// Optional URL scheme or template (e.g. weixin://).
        #[arg(long)]
        url: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum StorageCmd {
    /// Query local storage and inbox space consumption.
    Status {
        /// Output raw JSON snapshot instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clean orphaned or incomplete partial transfer files in the inbox.
    Clean {
        /// Clean all incomplete partials, not just orphaned ones.
        #[arg(long)]
        all: bool,
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

async fn loopback_get(port: u16, path: &str) -> std::io::Result<(u16, Vec<u8>)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await?;
    let head = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(head.as_bytes()).await?;
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

async fn loopback_events_connect(port: u16) -> std::io::Result<SseStream> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await?;
    let req = "GET /api/events HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\n\r\n";
    stream.write_all(req.as_bytes()).await?;
    stream.flush().await?;

    let mut lines = tokio::io::BufReader::new(stream).lines();
    let status_line = lines.next_line().await?.unwrap_or_default();
    if !status_line.contains("200") {
        return Err(std::io::Error::other(format!(
            "SSE HTTP handshake failed: {status_line}"
        )));
    }
    while let Some(header) = lines.next_line().await? {
        if header.trim().is_empty() {
            break;
        }
    }
    Ok(SseStream { lines })
}

fn print_event(v: &serde_json::Value) {
    let kind = v["kind"].as_str().unwrap_or("unknown");
    match kind {
        "status" => {
            let msg = v["message"].as_str().unwrap_or("");
            println!("[status] {msg}");
        }
        "peer_up" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let name = v["name"].as_str().unwrap_or("?");
            let platform = v["platform"].as_str().unwrap_or("?");
            println!("[peer_up] {name} ({dev}) [{platform}]");
        }
        "peer_down" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            println!("[peer_down] {dev}");
        }
        "file_offered" => {
            let tid = v["transfer_id"].as_str().unwrap_or("?");
            let src = v["source"].as_str().unwrap_or("?");
            let files_count = v["files"].as_array().map(|a| a.len()).unwrap_or(0);
            println!("[file_offered] transfer={tid} from={src} ({files_count} files)");
        }
        "file_done" => {
            let tid = v["transfer_id"].as_str().unwrap_or("?");
            println!("[file_done] transfer={tid}");
        }
        "pairing_needed" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let fp = v["fp"].as_str().unwrap_or("?");
            println!("[pairing_needed] peer={dev} fp={fp}");
        }
        "candidate" => {
            let c = &v["candidate"];
            let id = c["id"].as_str().unwrap_or("?");
            let app = c["app_name"]
                .as_str()
                .unwrap_or(c["app_id"].as_str().unwrap_or("?"));
            let title = c["title"].as_str().unwrap_or("");
            println!("[candidate] id={id} app={app} title={title}");
        }
        "candidate_expired" => {
            let id = v["id"].as_str().unwrap_or("?");
            println!("[candidate_expired] id={id}");
        }
        "reply_received" => {
            let r = &v["reply"];
            let id = r["id"].as_str().unwrap_or("?");
            let app = r["app_id"].as_str().unwrap_or("?");
            println!("[reply_received] id={id} app={app}");
        }
        "overlay" => {
            let present = v["present"].as_bool().unwrap_or(false);
            let lan_only = v["lan_only"].as_bool().unwrap_or(false);
            println!("[overlay] present={present} lan_only={lan_only}");
        }
        other => {
            println!("[{other}] {v}");
        }
    }
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
        Cmd::Notify {
            title,
            body,
            app_id,
            peer,
            ui_port,
        } => {
            let payload = serde_json::json!({
                "title": title,
                "body": body,
                "app_id": app_id,
                "peer": peer,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/notify", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "node refused notify (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let peer = v.get("peer").and_then(|p| p.as_str()).unwrap_or("?");
            println!("notified {peer}");
        }
        Cmd::Send {
            path,
            name,
            peer,
            watch,
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

            if watch {
                println!("watching transfer progress for [{transfer_id}]...");
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    let (status_code, state_resp) = loopback_get(ui_port, "/api/state").await?;
                    if status_code != 200 {
                        continue;
                    }
                    let state: serde_json::Value = match serde_json::from_slice(&state_resp) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let transfers = state.get("transfers").and_then(|t| t.as_array());
                    let current = transfers.and_then(|list| {
                        list.iter().find(|t| {
                            t.get("transfer_id").and_then(|id| id.as_str()) == Some(transfer_id)
                        })
                    });
                    if let Some(t) = current {
                        let transferred =
                            t.get("transferred").and_then(|v| v.as_u64()).unwrap_or(0);
                        let total = t.get("total").and_then(|v| v.as_u64()).unwrap_or(size);
                        let speed_bps = t.get("speed_bps").and_then(|v| v.as_u64()).unwrap_or(0);
                        let phase = t
                            .get("phase")
                            .and_then(|v| v.as_str())
                            .unwrap_or("transferring");
                        let pct = if total > 0 {
                            (transferred as f64 / total as f64) * 100.0
                        } else {
                            0.0
                        };
                        print!(
                            "\r[{:>5.1}%] {} / {} ({}) [{}]      ",
                            pct,
                            format_bytes(transferred),
                            format_bytes(total),
                            format_speed(speed_bps),
                            phase
                        );
                        use std::io::Write;
                        let _ = std::io::stdout().flush();

                        if phase == "completed" {
                            println!(
                                "\n✓ Transfer completed: {file_name} ({})",
                                format_bytes(total)
                            );
                            break;
                        } else if phase == "failed" {
                            let err_msg = t
                                .get("error")
                                .and_then(|e| e.as_str())
                                .unwrap_or("transfer failed");
                            bail!("\n✗ Transfer failed: {err_msg}");
                        }
                    }
                }
            }
        }
        Cmd::Cancel { id, ui_port } => {
            let body = serde_json::to_vec(&serde_json::json!({ "id": id }))?;
            let (code, resp) = loopback_post(ui_port, "/api/file/cancel", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "cancel failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            println!("cancelled transfer [{id}]");
        }
        Cmd::ClearTransfers { ui_port } => {
            let (code, resp) = loopback_post(ui_port, "/api/file/clear", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "clear failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let cleared = v.get("cleared").and_then(|c| c.as_u64()).unwrap_or(0);
            println!("cleared {cleared} finished transfer(s) from history");
        }
        Cmd::Unpair { device_id, ui_port } => {
            let payload = serde_json::json!({ "device_id": device_id });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/peers/unpair", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "node refused unpair (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let unpaired = v.get("unpaired").and_then(|u| u.as_bool()).unwrap_or(false);
            if unpaired {
                println!("unpaired and severed connection with {device_id}");
            } else {
                println!("peer {device_id} was not in trusted peers list");
            }
        }
        Cmd::Clip { action } => match action {
            ClipCmd::Sync { peer, ui_port } => {
                let payload = serde_json::json!({ "peer": peer });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = loopback_post(ui_port, "/api/clip/sync", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "clipboard sync failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let sent_peers = v.get("sent_peers").and_then(|c| c.as_u64()).unwrap_or(0);
                let bytes = v.get("bytes").and_then(|c| c.as_u64()).unwrap_or(0);
                let hash = v.get("hash").and_then(|h| h.as_str()).unwrap_or("");
                let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
                println!(
                    "synced clipboard ({bytes} bytes, sha256:{short_hash}...) to {sent_peers} peer(s)"
                );
            }
            ClipCmd::Send {
                text,
                peer,
                ui_port,
            } => {
                let content = match text {
                    Some(t) => t,
                    None => {
                        use std::io::Read;
                        let mut buf = String::new();
                        std::io::stdin().read_to_string(&mut buf)?;
                        buf
                    }
                };
                let payload = serde_json::json!({
                    "text": content,
                    "peer": peer,
                });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = loopback_post(ui_port, "/api/clip/sync", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "clipboard send failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let sent_peers = v.get("sent_peers").and_then(|c| c.as_u64()).unwrap_or(0);
                let bytes = v.get("bytes").and_then(|c| c.as_u64()).unwrap_or(0);
                let hash = v.get("hash").and_then(|h| h.as_str()).unwrap_or("");
                let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
                println!("sent {bytes} bytes to {sent_peers} peer(s) (sha256:{short_hash}...)");
            }
            ClipCmd::Get { ui_port } => {
                let (code, resp) = loopback_get(ui_port, "/api/clip").await.map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
                if code != 200 {
                    bail!(
                        "clipboard get failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
                    print!("{text}");
                } else {
                    eprintln!("(clipboard is empty)");
                }
            }
        },
        Cmd::Candidates { json, ui_port } => {
            let (code, resp) = loopback_get(ui_port, "/api/notify/candidates")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "candidates query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let list = v.get("candidates").and_then(|c| c.as_array());
                match list {
                    Some(items) if !items.is_empty() => {
                        println!(
                            "{:<32} {:<24} {:<6} {:<10} TITLE",
                            "ID", "APP", "OTP", "EXPIRES"
                        );
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis() as u64)
                            .unwrap_or(0);
                        for item in items {
                            let id = item.get("id").and_then(|s| s.as_str()).unwrap_or("-");
                            let app = item.get("app_id").and_then(|s| s.as_str()).unwrap_or("-");
                            let has_otp = item
                                .get("has_otp")
                                .and_then(|b| b.as_bool())
                                .unwrap_or(false);
                            let exp = item.get("expires_ms").and_then(|n| n.as_u64()).unwrap_or(0);
                            let title = item.get("title").and_then(|s| s.as_str()).unwrap_or("");
                            let rem_sec = exp.saturating_sub(now_ms) / 1000;
                            println!(
                                "{:<32} {:<24} {:<6} {:<10} {}",
                                id,
                                app,
                                if has_otp { "YES" } else { "NO" },
                                format!("{}s", rem_sec),
                                title
                            );
                        }
                    }
                    _ => println!("(no active notification candidates)"),
                }
            }
        }
        Cmd::Otp { id, copy, ui_port } => {
            let payload = serde_json::json!({ "id": id });
            let bytes = serde_json::to_vec(&payload)?;
            if copy {
                let (c_code, c_resp) =
                    loopback_post(ui_port, "/api/copy", &bytes)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if c_code != 200 {
                    bail!(
                        "copy OTP failed (HTTP {c_code}): {}",
                        String::from_utf8_lossy(&c_resp)
                    );
                }
            }
            let (code, resp) = loopback_post(ui_port, "/api/otp/get", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "get OTP failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value = serde_json::from_slice(&resp)
                .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
            let otp = v
                .get("otp")
                .and_then(|o| o.as_str())
                .ok_or_else(|| anyhow::anyhow!("no OTP in response"))?;
            println!("{otp}");
            if copy {
                eprintln!("(copied to system clipboard)");
            }
        }
        Cmd::Dismiss { id, ui_port } => {
            let payload = serde_json::json!({ "id": id });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/dismiss", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "dismiss failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            println!("dismissed candidate {id}");
        }
        Cmd::Reply { id, text, ui_port } => {
            let payload = serde_json::json!({ "id": id, "text": text });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/reply", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "reply failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            println!("reply sent for candidate {id}");
        }
        Cmd::Replies { json, ui_port } => {
            let (code, resp) = loopback_get(ui_port, "/api/notify/replies")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "replies query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let list = v.get("replies").and_then(|r| r.as_array());
                match list {
                    Some(items) if !items.is_empty() => {
                        println!("{:<32} {:<24} UID", "ID", "APP");
                        for item in items {
                            let id = item.get("id").and_then(|s| s.as_str()).unwrap_or("-");
                            let app = item.get("app_id").and_then(|s| s.as_str()).unwrap_or("-");
                            let uid = item.get("uid").and_then(|s| s.as_str()).unwrap_or("-");
                            println!("{:<32} {:<24} {}", id, app, uid);
                        }
                    }
                    _ => println!("(no queued inbound replies)"),
                }
            }
        }
        Cmd::TakeReply { id, ui_port } => {
            let payload = serde_json::json!({ "id": id });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/reply/take", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "take reply failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value = serde_json::from_slice(&resp)
                .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
            let app = v.get("app_id").and_then(|s| s.as_str()).unwrap_or("-");
            let uid = v.get("uid").and_then(|s| s.as_str()).unwrap_or("-");
            let text = v.get("text").and_then(|s| s.as_str()).unwrap_or("");
            println!("app_id: {app}, uid: {uid}, text: {text}");
        }
        Cmd::Screen { action } => match action {
            ScreenCmd::Allow { ui_port } => {
                let (code, resp) = loopback_post(ui_port, "/api/screen/allow", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "screen allow failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                println!("Screen viewing allowed (local consent granted).");
            }
            ScreenCmd::Revoke { ui_port } => {
                let (code, resp) = loopback_post(ui_port, "/api/screen/revoke", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "screen revoke failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                println!("Screen viewing consent revoked.");
            }
            ScreenCmd::Start { peer, ui_port } => {
                let payload = serde_json::json!({ "peer": peer });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = loopback_post(ui_port, "/api/screen/start", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "screen start failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                println!("Screen streaming started.");
            }
            ScreenCmd::Stop { ui_port } => {
                let (code, resp) = loopback_post(ui_port, "/api/screen/stop", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "screen stop failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                println!("Screen streaming stopped.");
            }
            ScreenCmd::Status { json, ui_port } => {
                let (code, resp) = loopback_get(ui_port, "/api/screen").await.map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
                if code != 200 {
                    bail!(
                        "screen status query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
                    let open = v
                        .get("channel_open")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);
                    let sent = v.get("sent").and_then(|n| n.as_u64()).unwrap_or(0);
                    let received = v.get("received").and_then(|n| n.as_u64()).unwrap_or(0);
                    let dropped = v.get("dropped").and_then(|n| n.as_u64()).unwrap_or(0);
                    let refused = v.get("refused").and_then(|n| n.as_u64()).unwrap_or(0);
                    let bytes = v.get("bytes").and_then(|n| n.as_u64()).unwrap_or(0);
                    let p95 = v.get("p95_frame_interval_us").and_then(|n| n.as_u64());

                    println!("Screen Channel Status:");
                    println!("  State:        {state}");
                    println!("  Channel Open: {open}");
                    println!("  Frames Sent:  {sent}");
                    println!("  Frames Recv:  {received}");
                    println!("  Dropped:      {dropped}");
                    println!("  Refused:      {refused}");
                    println!("  Bytes:        {bytes}");
                    match p95 {
                        Some(us) => {
                            println!("  p95 Interval: {:.2} ms ({us} us)", us as f64 / 1000.0);
                        }
                        None => println!("  p95 Interval: (not yet measurable)"),
                    }
                }
            }
        },
        Cmd::Input { action } => match action {
            InputCmd::Start { peer, ui_port } => {
                let payload = serde_json::json!({ "peer": peer });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = loopback_post(ui_port, "/api/input/start", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "input start failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let seat = v.get("seat").and_then(|s| s.as_str()).unwrap_or("unknown");
                println!("Input channel connected. Cursor seat: {seat}");
            }
            InputCmd::Stop { ui_port } => {
                let (code, resp) = loopback_post(ui_port, "/api/input/stop", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "input stop failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let seat = v.get("seat").and_then(|s| s.as_str()).unwrap_or("local");
                println!("Input channel disconnected. Cursor seat: {seat}");
            }
            InputCmd::Status { json, ui_port } => {
                let (code, resp) = loopback_get(ui_port, "/api/input").await.map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
                if code != 200 {
                    bail!(
                        "input status query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let open = v
                        .get("channel_open")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);
                    let seat = v.get("seat").and_then(|s| s.as_str()).unwrap_or("unknown");
                    println!("Input Channel Status:");
                    println!("  Channel Open: {open}");
                    println!("  Cursor Seat:  {seat}");
                }
            }
        },
        Cmd::Peers {
            trusted,
            ping,
            peer,
            count,
            json,
            ui_port,
        } => {
            if ping {
                return do_peer_ping(ui_port, peer, count, json).await;
            }
            let (code, resp) = loopback_get(ui_port, "/api/peers").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                bail!(
                    "node returned HTTP {code}: {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
                return Ok(());
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let self_id = v["self_id"].as_str().unwrap_or("unknown");
            let live_arr = v["live"].as_array();
            let live_count = v["live_count"].as_u64().unwrap_or(0);
            let trusted_count = v["trusted_count"].as_u64().unwrap_or(0);

            println!("Node: {self_id} (live: {live_count}, trusted: {trusted_count})");
            println!("\nActive Live Peers:");
            match live_arr {
                Some(arr) if !arr.is_empty() => {
                    for p in arr {
                        let dev = p["device_id"].as_str().unwrap_or("-");
                        let name = p["name"].as_str().unwrap_or("-");
                        let platform = p["platform"].as_str().unwrap_or("-");
                        let addr = p["addr"].as_str().unwrap_or("-");
                        let path = p["path"].as_str().unwrap_or("-");
                        let rtt_str = match p["rtt_ms"].as_u64() {
                            Some(rtt) => format!("{rtt}ms"),
                            None => "n/a".to_string(),
                        };
                        println!("  * {name} ({platform}) · {addr} [{path}] · rtt: {rtt_str} · {dev}");
                    }
                }
                _ => println!("  (no live peers connected)"),
            }

            if trusted {
                println!("\nTrusted Peers Inventory:");
                if let Some(arr) = v["trusted"].as_array() {
                    if arr.is_empty() {
                        println!("  (no trusted peers)");
                    } else {
                        for tp in arr {
                            let dev = tp["device_id"].as_str().unwrap_or("-");
                            let alias = tp["alias"].as_str().unwrap_or("-");
                            let online = tp["online"].as_bool().unwrap_or(false);
                            let status = if online { "ONLINE" } else { "OFFLINE" };
                            println!("  * [{status}] {alias} · {dev}");
                        }
                    }
                }
            }
        }
        Cmd::Routes { json, ui_port } => {
            let (code, resp) = loopback_get(ui_port, "/api/routes").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                bail!(
                    "node returned HTTP {code}: {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
                return Ok(());
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let routes_arr = v["routes"].as_array();
            let count = v["count"].as_u64().unwrap_or(0);
            println!("Cached Physical Routes ({count}):");
            match routes_arr {
                Some(arr) if !arr.is_empty() => {
                    for r in arr {
                        let dev = r["device_id"].as_str().unwrap_or("-");
                        let addr = r["addr"].as_str().unwrap_or("-");
                        println!("  * {dev} -> {addr}");
                    }
                }
                _ => println!("  (no cached routes)"),
            }
        }
        Cmd::Net {
            interfaces,
            json,
            ui_port,
        } => {
            let (code, resp) = loopback_get(ui_port, "/api/net").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                bail!(
                    "node returned HTTP {code}: {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
                return Ok(());
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;

            let ov = &v["overlay"];
            let ov_enabled = ov["enabled"].as_bool().unwrap_or(false);
            let ov_present = ov["present"].as_bool().unwrap_or(false);
            let ov_source = ov["source"].as_str().unwrap_or("Absent");
            let local_ips = ov["local_ips"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let peer_count = ov["peer_count"].as_u64().unwrap_or(0);
            let peers = ov["peers"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();

            println!("EasyTier Overlay Sidecar:");
            println!("  Enabled:    {ov_enabled}");
            println!("  Present:    {ov_present} ({ov_source})");
            println!(
                "  Local IPs:  {}",
                if local_ips.is_empty() {
                    "(none)"
                } else {
                    &local_ips
                }
            );
            if peer_count == 0 {
                println!("  Mesh Peers: 0");
            } else {
                println!("  Mesh Peers: {peer_count} ({peers})");
            }

            let ifaces_arr = v["interfaces"].as_array();
            if interfaces {
                println!("\nInterface Classification Details:");
                match ifaces_arr {
                    Some(arr) if !arr.is_empty() => {
                        for iface in arr {
                            let name = iface["iface"].as_str().unwrap_or("-");
                            let ip = iface["ip"].as_str().unwrap_or("-");
                            let listen = iface["listen"].as_bool().unwrap_or(false);
                            let reason = iface["reason"].as_str();
                            if listen {
                                println!("  * [LISTEN] {name}: {ip}");
                            } else {
                                let r_str = reason.map(|r| format!(" ({r})")).unwrap_or_default();
                                println!("  * [SKIP]   {name}: {ip}{r_str}");
                            }
                        }
                    }
                    _ => println!("  (no interfaces reported)"),
                }
            } else {
                println!("\nActive Listening Interfaces:");
                let mut listen_count = 0;
                if let Some(arr) = ifaces_arr {
                    for iface in arr {
                        let listen = iface["listen"].as_bool().unwrap_or(false);
                        if listen {
                            let name = iface["iface"].as_str().unwrap_or("-");
                            let ip = iface["ip"].as_str().unwrap_or("-");
                            println!("  * {name}: {ip}");
                            listen_count += 1;
                        }
                    }
                }
                if listen_count == 0 {
                    println!("  (no listening interfaces)");
                }
                println!("\n(Use --interfaces or -i to inspect all skipped interfaces and classification reasons)");
            }
        }
        Cmd::Ancs { action } => match action {
            AncsCmd::Status { json, ui_port } => {
                let (code, resp) = loopback_get(ui_port, "/api/ancs").await.map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
                if code != 200 {
                    anyhow::bail!(
                        "ANCS status query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    let attached = v.get("attached").and_then(|b| b.as_bool()).unwrap_or(false);
                    let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
                    let retry = v.get("retry_at_ms").and_then(|n| n.as_u64());
                    let allowlist = v.get("allowlist").and_then(|a| a.as_array());

                    println!("Apple Notification Center Service (ANCS):");
                    println!("  Transport Attached: {attached}");
                    println!("  Ingress State:      {state}");
                    match retry {
                        Some(ms) => println!("  Retry Backoff At:   {ms} ms"),
                        None => println!("  Retry Backoff At:   none"),
                    }
                    println!("  Open Allowlist:");
                    match allowlist {
                        Some(arr) if !arr.is_empty() => {
                            for item in arr {
                                if let Some(app) = item.as_str() {
                                    println!("    * {app}");
                                }
                            }
                        }
                        _ => println!("    (no applications in allowlist)"),
                    }
                }
            }
            AncsCmd::Connect { ui_port } => {
                let (code, resp) = loopback_post(ui_port, "/api/ancs/connect", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "ANCS connect failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                println!("ANCS connection triggered successfully.");
            }
            AncsCmd::Allow {
                app_id,
                url,
                ui_port,
            } => {
                let payload = match url {
                    Some(u) => serde_json::json!({ "app_id": app_id, "url": u }),
                    None => serde_json::json!({ "app_id": app_id }),
                };
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = loopback_post(ui_port, "/api/ancs/allow", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "ANCS allow rule failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                println!("Allowed open action for '{app_id}'.");
            }
        },
        Cmd::Storage { action } => match action {
            StorageCmd::Status { json, ui_port } => {
                let (code, resp) = loopback_get(ui_port, "/api/storage").await.map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
                if code != 200 {
                    anyhow::bail!(
                        "storage status query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let data_dir = v["data_dir"].as_str().unwrap_or("-");
                    let inbox_dir = v["inbox_dir"].as_str().unwrap_or("-");
                    let inbox_total_bytes = v["inbox_total_bytes"].as_u64().unwrap_or(0);
                    let inbox_file_count = v["inbox_file_count"].as_u64().unwrap_or(0);
                    let partial_bytes = v["partial_bytes"].as_u64().unwrap_or(0);
                    let partial_count = v["partial_count"].as_u64().unwrap_or(0);
                    let orphaned_partial_count = v["orphaned_partial_count"].as_u64().unwrap_or(0);
                    let orphaned_partial_bytes = v["orphaned_partial_bytes"].as_u64().unwrap_or(0);
                    let meta_bytes = v["meta_bytes"].as_u64().unwrap_or(0);

                    println!("Node Storage Status:");
                    println!("  Data Directory:    {data_dir}");
                    println!("  Inbox Directory:   {inbox_dir}");
                    println!(
                        "  Inbox Files:       {inbox_file_count} ({})",
                        format_bytes(inbox_total_bytes)
                    );
                    println!(
                        "  Partial In-flight: {partial_count} ({})",
                        format_bytes(partial_bytes)
                    );
                    println!(
                        "  Orphaned Partials: {orphaned_partial_count} ({})",
                        format_bytes(orphaned_partial_bytes)
                    );
                    println!("  Metadata Storage:  {}", format_bytes(meta_bytes));
                }
            }
            StorageCmd::Clean { all, ui_port } => {
                let payload = serde_json::json!({ "orphaned_only": !all });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = loopback_post(ui_port, "/api/storage/clean", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "storage clean failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let freed_bytes = v["freed_bytes"].as_u64().unwrap_or(0);
                let deleted_files = v["deleted_files_count"].as_u64().unwrap_or(0);
                let cleared_resumes = v["cleared_resume_entries"].as_u64().unwrap_or(0);

                println!("Storage cleanup complete:");
                println!("  Freed Space:     {}", format_bytes(freed_bytes));
                println!("  Deleted Files:   {deleted_files}");
                if cleared_resumes > 0 {
                    println!("  Cleared Resumes: {cleared_resumes}");
                }
            }
        },
        Cmd::Status {
            watch,
            interval,
            json,
            ui_port,
        } => loop {
            let (code, resp) = loopback_get(ui_port, "/api/state").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                bail!(
                    "node refused status query (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                if watch {
                    print!("\x1b[2J\x1b[H");
                }
                print_status_summary(&v);
            }
            if !watch {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(interval.max(1))).await;
        },
        Cmd::Health { json, ui_port } => {
            let t0 = std::time::Instant::now();
            let (code, resp) = loopback_get(ui_port, "/api/health").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            let rtt = t0.elapsed();
            if code != 200 {
                bail!(
                    "node health check failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let status = v["status"].as_str().unwrap_or("unknown");
                let version = v["version"].as_str().unwrap_or("?");
                let device_id = v["device_id"].as_str().unwrap_or("?");
                let name = v["name"].as_str().unwrap_or("?");
                let platform = v["platform"].as_str().unwrap_or("?");
                let pid = v["pid"].as_u64().unwrap_or(0);
                let uptime_secs = v["uptime_secs"].as_u64().unwrap_or(0);
                let live_peers = v["live_peers"].as_u64().unwrap_or(0);
                let trusted_peers = v["trusted_peers"].as_u64().unwrap_or(0);
                let ports = &v["ports"];
                let control_port = ports["control"].as_u64().unwrap_or(0);
                let file_port = ports["file"].as_u64().unwrap_or(0);
                let input_port = ports["input"].as_u64().unwrap_or(0);
                let screen_port = ports["screen"].as_u64().unwrap_or(0);
                let ui_port = ports["ui"].as_u64().unwrap_or(0);

                println!("Node Health Status: {status} (200 OK)");
                println!("  Version:          {version}");
                println!("  Device ID:        {device_id}");
                println!("  Node Name:        {name} ({platform})");
                println!("  PID:              {pid}");
                println!(
                    "  Uptime:           {} ({uptime_secs}s)",
                    format_uptime(uptime_secs)
                );
                println!("  Bound Ports:");
                println!("    Control (Mesh): {control_port}");
                println!("    File Transfer:  {file_port}");
                println!("    Input (Remote): {input_port}");
                println!("    Screen (Video): {screen_port}");
                println!("    UI (Loopback):  {ui_port}");
                println!("  Live Peers:       {live_peers}");
                println!("  Trusted Peers:    {trusted_peers}");
                println!("  Loopback Latency: {:.2}ms", rtt.as_secs_f64() * 1000.0);
            }
        }
        Cmd::Ping {
            peer,
            count,
            json,
            ui_port,
        } => {
            if peer.is_some() {
                return do_peer_ping(ui_port, peer, count, json).await;
            }
            let count = count.max(1);
            let mut rtts = Vec::new();
            let mut last_v = None;
            for i in 1..=count {
                let t0 = std::time::Instant::now();
                let (code, resp) = loopback_get(ui_port, "/api/health").await.map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
                let rtt = t0.elapsed();
                if code != 200 {
                    bail!(
                        "node ping failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let rtt_ms = rtt.as_secs_f64() * 1000.0;
                rtts.push(rtt_ms);
                last_v = Some(v.clone());
                if !json {
                    let status = v["status"].as_str().unwrap_or("unknown");
                    let version = v["version"].as_str().unwrap_or("?");
                    let pid = v["pid"].as_u64().unwrap_or(0);
                    let uptime_secs = v["uptime_secs"].as_u64().unwrap_or(0);
                    println!(
                        "[{i}/{count}] pong: node {status}, rtt: {rtt_ms:.2}ms, uptime: {} (pid: {pid}, v{version})",
                        format_uptime(uptime_secs)
                    );
                }
                if i < count {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
            if json {
                if let Some(mut v) = last_v {
                    let avg_rtt = rtts.iter().copied().sum::<f64>() / rtts.len() as f64;
                    v["rtt_ms"] = serde_json::json!(avg_rtt);
                    v["count"] = serde_json::json!(count);
                    println!("{v}");
                }
            }
        }
        Cmd::Stop { ui_port } => {
            let (code, resp) = loopback_post(ui_port, "/api/daemon/shutdown", b"")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "node shutdown failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            println!("Node daemon shutdown initiated (HTTP 200).");
        }
        Cmd::Events {
            count,
            timeout_secs,
            json,
            ui_port,
        } => {
            let mut sse = loopback_events_connect(ui_port).await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if !json {
                println!("Listening for real-time node events on 127.0.0.1:{ui_port}...");
            }
            let deadline = if timeout_secs > 0 {
                Some(std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs))
            } else {
                None
            };
            let mut received = 0usize;
            loop {
                let next_res = if let Some(dl) = deadline {
                    let now = std::time::Instant::now();
                    if now >= dl {
                        break;
                    }
                    match tokio::time::timeout(dl - now, sse.next_event()).await {
                        Ok(res) => res,
                        Err(_) => break,
                    }
                } else {
                    sse.next_event().await
                };

                match next_res {
                    Ok(Some(raw)) => {
                        received += 1;
                        if json {
                            println!("{raw}");
                        } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                            print_event(&val);
                        } else {
                            println!("{raw}");
                        }
                        if count > 0 && received >= count {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        bail!("event stream error: {e}");
                    }
                }
            }
        }
    }
    Ok(())
}

fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    let s = secs % 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m {s}s")
    } else if hours > 0 {
        format!("{hours}h {minutes}m {s}s")
    } else if minutes > 0 {
        format!("{minutes}m {s}s")
    } else {
        format!("{s}s")
    }
}

fn print_status_summary(v: &serde_json::Value) {
    let device_id = v.get("device_id").and_then(|s| s.as_str()).unwrap_or("?");
    let name = v.get("name").and_then(|s| s.as_str()).unwrap_or("?");
    let platform = v.get("platform").and_then(|s| s.as_str()).unwrap_or("?");
    let pin_active = v
        .get("pin_active")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    let lan_only = v.get("lan_only").and_then(|s| s.as_bool()).unwrap_or(false);

    println!("Tetherly Node Status:");
    println!("  Device ID:  {device_id}");
    println!("  Node Name:  {name} ({platform})");
    println!("  PIN Active: {pin_active}");
    println!("  LAN Only:   {lan_only}");

    if let Some(peers) = v.get("peers").and_then(|p| p.as_array()) {
        println!("  Connected Peers ({}):", peers.len());
        for p in peers {
            let id = p.get("device_id").and_then(|s| s.as_str()).unwrap_or("?");
            let name = p.get("name").and_then(|s| s.as_str()).unwrap_or("?");
            let addr = p.get("addr").and_then(|s| s.as_str()).unwrap_or("?");
            let path = p.get("path").and_then(|s| s.as_str()).unwrap_or("?");
            println!("    - {id} ({name}) at {addr} [{path}]");
        }
    }

    if let Some(transfers) = v.get("transfers").and_then(|t| t.as_array()) {
        println!("  Transfers ({}):", transfers.len());
        for t in transfers {
            let id = t.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
            let dir = t.get("direction").and_then(|s| s.as_str()).unwrap_or("?");
            let filename = t.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
            let transferred = t.get("transferred").and_then(|s| s.as_u64()).unwrap_or(0);
            let total = t.get("total").and_then(|s| s.as_u64()).unwrap_or(0);
            let speed = t.get("speed_bps").and_then(|s| s.as_u64()).unwrap_or(0);
            let phase = t.get("phase").and_then(|s| s.as_str()).unwrap_or("?");
            let pct = if total > 0 {
                (transferred as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            let speed_str = if speed > 0 {
                format!("{} ", format_speed(speed))
            } else {
                "".to_string()
            };
            println!(
                "    - [{dir}] {filename} ({:>5.1}%, {} / {}) {speed_str}[{phase}] ({id})",
                pct,
                format_bytes(transferred),
                format_bytes(total)
            );
        }
    }

    if let Some(stats) = v.get("transfer_stats") {
        let sent = stats
            .get("bytes_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let recv = stats
            .get("bytes_received")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let completed = stats
            .get("completed_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let failed = stats
            .get("failed_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        println!(
            "  Transfer Metrics: sent: {}, received: {}, completed: {}, failed: {}",
            format_bytes(sent),
            format_bytes(recv),
            completed,
            failed
        );
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.2} GiB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MiB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KiB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn format_speed(bps: u64) -> String {
    format!("{}/s", format_bytes(bps))
}

async fn do_peer_ping(
    ui_port: u16,
    peer: Option<String>,
    count: u32,
    json: bool,
) -> anyhow::Result<()> {
    let body = serde_json::json!({ "peer": peer });
    let body_bytes = serde_json::to_vec(&body)?;
    let count = count.max(1);
    let mut rtts: Vec<u64> = Vec::new();
    let mut last_rep: Option<serde_json::Value> = None;
    for i in 1..=count {
        let (code, resp) = loopback_post(ui_port, "/api/peers/ping", &body_bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            bail!(
                "peer ping failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let v: serde_json::Value = serde_json::from_slice(&resp)
            .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let rtt = v["rtt_ms"].as_u64().unwrap_or(0);
        let dev = v["device_id"].as_str().unwrap_or("-");
        let path = v["path"].as_str().unwrap_or("-");
        let addr = v["addr"].as_str().unwrap_or("-");
        rtts.push(rtt);
        last_rep = Some(v);
        if !json {
            println!("[{i}/{count}] Ping {dev} ({addr} [{path}]): time={rtt}ms");
        }
        if i < count {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    if json {
        if let Some(mut rep) = last_rep {
            let min_rtt = rtts.iter().copied().min().unwrap_or(0);
            let max_rtt = rtts.iter().copied().max().unwrap_or(0);
            let avg_rtt = (rtts.iter().copied().sum::<u64>() as f64) / (rtts.len() as f64);
            rep["count"] = serde_json::json!(count);
            rep["min_rtt_ms"] = serde_json::json!(min_rtt);
            rep["max_rtt_ms"] = serde_json::json!(max_rtt);
            rep["avg_rtt_ms"] = serde_json::json!(avg_rtt);
            rep["history"] = serde_json::json!(rtts);
            println!("{rep}");
        }
    } else if count > 1 {
        let min_rtt = rtts.iter().copied().min().unwrap_or(0);
        let max_rtt = rtts.iter().copied().max().unwrap_or(0);
        let avg_rtt = (rtts.iter().copied().sum::<u64>() as f64) / (rtts.len() as f64);
        println!(
            "\n--- Ping Statistics ({count} probes) ---\nmin/avg/max = {min_rtt}/{avg_rtt:.1}/{max_rtt} ms"
        );
    }
    Ok(())
}

