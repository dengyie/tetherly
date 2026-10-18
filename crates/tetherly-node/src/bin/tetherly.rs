// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Desktop CLI: LAN + optional EasyTier sidecar overlay.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tetherly_node::{Node, NodeConfig, SystemSink};
use tracing::{info, warn};

#[derive(Parser)]
#[command(
    name = "tetherly",
    about = "Tetherly LAN node with optional EasyTier sidecar"
)]
struct Cli {
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    name: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Persist identity, listen on LAN + loopback, advertise mDNS.
    Run {
        #[arg(long, default_value_t = 45717)]
        port: u16,
        /// Extra EasyTier overlay CIDRs, comma-separated. Default 10.144.144.0/24.
        #[arg(long)]
        overlay_cidr: Option<String>,
        /// Disable overlay bind/scan (LAN only).
        #[arg(long, default_value_t = false)]
        no_overlay: bool,
        /// Stream an adb device's screen (serial, e.g. 10.144.144.51:5555)
        /// instead of the local DXGI desktop.
        #[arg(long)]
        screen_adb: Option<String>,
        /// Bind a second node on one host: file/input/screen/UI ports.
        #[arg(long)]
        file_port: Option<u16>,
        #[arg(long)]
        input_port: Option<u16>,
        #[arg(long)]
        screen_port: Option<u16>,
        #[arg(long)]
        ui_port: Option<u16>,
        /// Receive iPhone notifications over ANCS/BLE from the paired device with
        /// this Bluetooth address (Windows notation, e.g. `11:22:33:44:55:66`).
        /// Pair the phone in Windows Bluetooth settings first. Runtime against a
        /// real iPhone is Manual-required.
        #[arg(long)]
        ancs: Option<String>,
    },
    /// Print device_id of the persisted identity.
    Identity,
    /// Show an 8-digit PIN (3 min TTL) on the *running* node.
    ///
    /// A PIN is only meaningful to the process holding it, so this asks the
    /// already-running node (its loopback UI API) to arm one. Starting a second
    /// node here would print a code no listener could ever accept.
    Pin {
        /// Query the current PIN status and remaining TTL without generating a new one
        #[arg(long)]
        status: bool,
        /// Revoke and clear any currently armed PIN
        #[arg(long)]
        clear: bool,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Ask the *running* node to dial a peer and pair or resume.
    ///
    /// Starting a second node here would bind the same LAN ports as the daemon
    /// and fail with "no LAN address bound". The live node already holds the
    /// identity, the PIN, and the listeners, so this only posts to its loopback
    /// UI API — the same route the desktop shell's 「连接」 button uses.
    Dial {
        #[arg(long)]
        addr: SocketAddr,
        #[arg(long)]
        pin: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Ask the *running* node to deliver a notify.push to one live peer.
    ///
    /// The body is the user's own text: it is delivered, never executed, and
    /// never logged. A second node cannot deliver it — only the process that
    /// holds the Noise session can. With one live peer the target is implied;
    /// with more than one, `--peer` is required.
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
    /// Ask the *running* node to send a file to one live peer.
    ///
    /// The file is read from the local filesystem and offered over the live
    /// session. With one live peer the target is implied; with more than one,
    /// `--peer` is required.
    Send {
        /// Path to the file to send.
        path: PathBuf,
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
    /// List active and recent file transfers or query a specific transfer.
    Transfers {
        /// Query a specific transfer by its ID.
        #[arg(long)]
        id: Option<String>,
        /// Filter for active in-flight transfers only (transferring or verifying).
        #[arg(long)]
        active: bool,
        /// Filter for completed or failed transfers only.
        #[arg(long)]
        terminal: bool,
        /// Filter by direction ("in" / "out").
        #[arg(long)]
        direction: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
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
    /// Set or update the friendly alias for a trusted peer device.
    Alias {
        /// Target peer (device_id like tdev_... or existing name/alias).
        peer: String,
        /// New friendly alias to assign.
        alias: String,
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
    /// Manage physical addressing routes and static peer injection.
    Route {
        #[command(subcommand)]
        action: RouteCmd,
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
    /// Inspect or dynamically update the node's running configuration.
    Config {
        /// Update node friendly broadcast name.
        #[arg(long)]
        name: Option<String>,
        /// Enable or disable mDNS LAN discovery broadcast.
        #[arg(long)]
        advertise: Option<bool>,
        /// Enable or disable automatic background reconnection to trusted peers.
        #[arg(long)]
        reconnect: Option<bool>,
        /// Output in JSON format.
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
    /// Introspect local and peer capabilities, or dynamically update enabled features.
    Caps {
        /// Target peer (tdev_... or alias) to introspect capability profile
        #[arg(long)]
        peer: Option<String>,
        /// Enable a feature capability (e.g. notify, clip, file, input, screen)
        #[arg(long)]
        enable: Option<String>,
        /// Disable a feature capability (e.g. notify, clip, file, input, screen)
        #[arg(long)]
        disable: Option<String>,
        /// Output in JSON format
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect unified network traffic, multi-channel bandwidth, and protocol telemetry.
    Metrics {
        /// Reset all traffic and protocol telemetry counters to zero
        #[arg(long)]
        reset: bool,
        /// Output raw JSON format instead of human-readable tables
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Alias for `metrics`: query multi-channel network traffic and protocol counters.
    Traffic {
        /// Reset all traffic and protocol telemetry counters to zero
        #[arg(long)]
        reset: bool,
        /// Output raw JSON format instead of human-readable tables
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Actively connect to a trusted peer device on demand.
    Connect {
        /// Target peer (device_id like tdev_... or friendly alias/name).
        #[arg(long)]
        peer: String,
        /// Optional direct physical socket address (e.g. 192.168.1.50:45717).
        #[arg(long)]
        addr: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Actively disconnect an active session with a peer device without unpairing.
    Disconnect {
        /// Target peer (device_id like tdev_... or friendly alias/name).
        #[arg(long)]
        peer: String,
        /// Output raw JSON instead of formatted text.
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

#[derive(Subcommand, Debug)]
enum RouteCmd {
    /// List cached physical addressing routes.
    List {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Add or update a static physical route for a peer.
    Add {
        /// Peer device ID (tdev_...), alias, or friendly name.
        #[arg(long, short)]
        peer: String,
        /// Physical socket address (e.g. 192.168.1.50:45717).
        #[arg(long, short)]
        addr: String,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Remove a physical route for a peer.
    #[command(alias = "rm", alias = "del")]
    Remove {
        /// Peer device ID (tdev_...), alias, or friendly name.
        #[arg(long, short)]
        peer: String,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
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
        tracing::warn!("TETHERLY_INSECURE_LOG=1 is set; debug profile only");
    }

    let cli = Cli::parse();
    let mut cfg = NodeConfig::default();
    if let Some(d) = cli.data_dir {
        cfg.data_dir = d;
    }
    if let Some(n) = cli.name {
        cfg.name = n;
    }

    match cli.cmd {
        Cmd::Identity => {
            let node = Node::start(cfg)?;
            println!("{}", node.identity().device_id());
        }
        Cmd::Pin {
            status,
            clear,
            json,
            ui_port,
        } => {
            if clear {
                let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/pin/clear", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused PIN clear request (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                } else {
                    let cleared = v.get("cleared").and_then(|c| c.as_bool()).unwrap_or(false);
                    if cleared {
                        println!("armed PIN successfully revoked and cleared.");
                    } else {
                        println!("no active PIN was armed.");
                    }
                }
            } else if status {
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/pin")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused PIN status request (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                } else {
                    let active = v.get("active").and_then(|a| a.as_bool()).unwrap_or(false);
                    if active {
                        let pin = v.get("pin").and_then(|p| p.as_str()).unwrap_or("???? ????");
                        let expires = v
                            .get("expires_in_secs")
                            .and_then(|e| e.as_u64())
                            .unwrap_or(0);
                        let ttl = v.get("ttl_secs").and_then(|t| t.as_u64()).unwrap_or(180);
                        println!("Active PIN: {pin} (valid for {expires}s / {ttl}s TTL)");
                    } else {
                        println!("No active PIN armed.");
                    }
                }
            } else {
                let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/pin", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused PIN request (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                } else {
                    let pin = v
                        .get("pin")
                        .and_then(|p| p.as_str())
                        .ok_or_else(|| anyhow::anyhow!("node returned no pin"))?;
                    let expires = v
                        .get("expires_in_secs")
                        .and_then(|e| e.as_u64())
                        .unwrap_or(180);
                    println!("{pin}");
                    println!("不要把数字发给任何人,只在你自己的另一台设备上输入。(有效时间: {expires} 秒)");
                }
            }
        }
        Cmd::Run {
            port,
            overlay_cidr,
            no_overlay,
            screen_adb,
            file_port,
            input_port,
            screen_port,
            ui_port,
            ancs,
        } => {
            cfg.control_port = port;
            // Real presentation: received frames open a window via GDI BitBlt.
            cfg.screen_sink = Some(Arc::new(SystemSink::new()));
            // Real injection for the controlled side (45719): received key and
            // mouse events drive SendInput. Harmless while no visitor holds
            // the seat (nothing arrives until the controller crosses an edge).
            cfg.input_sink = Some(Arc::new(std::sync::Mutex::new(
                tetherly_node::SystemInputSink,
            )));
            if let Some(serial) = screen_adb {
                // The screen is the adb device; encode it losslessly like DXGI.
                cfg.screen_source = Some(Arc::new(tetherly_node::EncodedSource::new(
                    tetherly_node::AdbSource::new(Some(serial)),
                    Arc::new(tetherly_node::ZstdCodec::default()),
                )));
            }
            if let Some(v) = file_port {
                cfg.file_port = v;
            }
            if let Some(v) = input_port {
                cfg.input_port = v;
            }
            if let Some(v) = screen_port {
                cfg.screen_port = v;
            }
            if let Some(v) = ui_port {
                cfg.ui_port = v;
            }
            if no_overlay {
                cfg.overlay.enabled = false;
            }
            if let Some(c) = overlay_cidr {
                cfg.overlay.cidrs = tetherly_node::parse_overlay_cidrs(&c);
            }
            // ANCS is optional and, on targets without a stack yet, honestly
            // absent; the pump below is only started when a link comes up.
            let ancs_link = match ancs.as_deref() {
                Some(address) => match tetherly_node::ancs::open(address) {
                    Ok(Some(link)) => Some(link),
                    Ok(None) => {
                        info!("ANCS is not implemented on this platform; iPhone notifications stay off");
                        None
                    }
                    Err(e) => {
                        warn!(%e, "ANCS address {address} is malformed; iPhone notifications stay off");
                        None
                    }
                },
                None => None,
            };
            let node = Node::start(cfg)?;
            info!(device_id = %node.identity().device_id(), "run");
            info!("ui http://127.0.0.1:45716");
            if let Some((_transport, inbound)) = ancs_link {
                node.spawn_ancs_pump(inbound);
                info!(
                    "ANCS link up; pair the iPhone in Windows Bluetooth settings if you have not"
                );
            }
            // Controller-side capture: low-level mouse hook for edge crossings;
            // the keyboard hook is armed only while the seat is remote.
            if let Err(e) = node.spawn_input_capture() {
                info!(%e, "input capture unavailable (key/mouse sharing stays off)");
            }
            // Frame pump: grab + encode + send while a visitor is streaming.
            // Idle ticks are cheap (Ok(false)). Source errors are recoverable
            // (adb respawns on the next grab, DXGI retries): log and back off
            // instead of killing the pump, or the stream would be dead for
            // every future visitor until process restart.
            let pump = node.clone_handle();
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_millis(16));
                // A blocking grab overruns the 16ms tick constantly: skip
                // missed ticks instead of bursting catch-up grabs.
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                let mut backoff = std::time::Duration::from_millis(250);
                loop {
                    tick.tick().await;
                    match pump.screen_send_frame().await {
                        Ok(_) => backoff = std::time::Duration::from_millis(250),
                        Err(e) => {
                            tracing::warn!(%e, "screen pump tick failed; backing off");
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(std::time::Duration::from_secs(5));
                        }
                    }
                }
            });
            node.run().await?;
        }
        Cmd::Dial { addr, pin, ui_port } => {
            // The running node owns the listeners. A second `Node::start` binds
            // the same ports and dies with "no LAN address bound".
            let payload = serde_json::json!({ "addr": addr.to_string(), "pin": pin });
            let body = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/dial", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused dial (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            println!("dialed {addr}");
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
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/notify", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
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
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/send", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
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
                    let (status_code, state_resp) =
                        tetherly_node::loopback_get(ui_port, "/api/state").await?;
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
                            anyhow::bail!("\n✗ Transfer failed: {err_msg}");
                        }
                    }
                }
            }
        }
        Cmd::Cancel { id, ui_port } => {
            let body = serde_json::to_vec(&serde_json::json!({ "id": id }))?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/cancel", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "cancel failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            println!("cancelled transfer [{id}]");
        }
        Cmd::Transfers {
            id,
            active,
            terminal,
            direction,
            json,
            ui_port,
        } => {
            do_transfers(ui_port, id, active, terminal, direction, json).await?;
        }
        Cmd::ClearTransfers { ui_port } => {
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/clear", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
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
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/unpair", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
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
                let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/clip/sync", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
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
                let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/clip/sync", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
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
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/clip")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
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
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/notify/candidates")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "candidates query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
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
                let (c_code, c_resp) = tetherly_node::loopback_post(ui_port, "/api/copy", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if c_code != 200 {
                    anyhow::bail!(
                        "copy OTP failed (HTTP {c_code}): {}",
                        String::from_utf8_lossy(&c_resp)
                    );
                }
            }
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/otp/get", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "get OTP failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
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
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/dismiss", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "dismiss failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            println!("dismissed candidate {id}");
        }
        Cmd::Reply { id, text, ui_port } => {
            let payload = serde_json::json!({ "id": id, "text": text });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/reply", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "reply failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            println!("reply sent for candidate {id}");
        }
        Cmd::Replies { json, ui_port } => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/notify/replies")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "replies query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
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
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/reply/take", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "take reply failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let app = v.get("app_id").and_then(|s| s.as_str()).unwrap_or("-");
            let uid = v.get("uid").and_then(|s| s.as_str()).unwrap_or("-");
            let text = v.get("text").and_then(|s| s.as_str()).unwrap_or("");
            println!("app_id: {app}, uid: {uid}, text: {text}");
        }
        Cmd::Screen { action } => match action {
            ScreenCmd::Allow { ui_port } => {
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/screen/allow", b"{}")
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "screen allow failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                println!("Screen viewing allowed (local consent granted).");
            }
            ScreenCmd::Revoke { ui_port } => {
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/screen/revoke", b"{}")
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "screen revoke failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                println!("Screen viewing consent revoked.");
            }
            ScreenCmd::Start { peer, ui_port } => {
                let payload = serde_json::json!({ "peer": peer });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/screen/start", &bytes)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "screen start failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                println!("Screen streaming started.");
            }
            ScreenCmd::Stop { ui_port } => {
                let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/screen/stop", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "screen stop failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                println!("Screen streaming stopped.");
            }
            ScreenCmd::Status { json, ui_port } => {
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/screen")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "screen status query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
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
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/input/start", &bytes)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "input start failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let seat = v.get("seat").and_then(|s| s.as_str()).unwrap_or("unknown");
                println!("Input channel connected. Cursor seat: {seat}");
            }
            InputCmd::Stop { ui_port } => {
                let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/input/stop", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "input stop failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let seat = v.get("seat").and_then(|s| s.as_str()).unwrap_or("local");
                println!("Input channel disconnected. Cursor seat: {seat}");
            }
            InputCmd::Status { json, ui_port } => {
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/input")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "input status query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
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
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/peers")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
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
                        let alias = p["alias"].as_str();
                        let display_name = match alias {
                            Some(a) if !a.is_empty() && a != name => {
                                format!("{name} (alias: \"{a}\")")
                            }
                            _ => name.to_string(),
                        };
                        let platform = p["platform"].as_str().unwrap_or("-");
                        let addr = p["addr"].as_str().unwrap_or("-");
                        let path = p["path"].as_str().unwrap_or("-");
                        let rtt_str = match p["rtt_ms"].as_u64() {
                            Some(rtt) => format!("{rtt}ms"),
                            None => "n/a".to_string(),
                        };
                        let feats_str = match p["features"].as_array() {
                            Some(farr) if !farr.is_empty() => {
                                let fstrs: Vec<&str> =
                                    farr.iter().filter_map(|x| x.as_str()).collect();
                                format!(" · caps: [{}]", fstrs.join(", "))
                            }
                            _ => String::new(),
                        };
                        println!(
                            "  * {display_name} ({platform}) · {addr} [{path}] · rtt: {rtt_str}{feats_str} · {dev}"
                        );
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
        Cmd::Alias {
            peer,
            alias,
            ui_port,
        } => {
            let payload = serde_json::json!({
                "peer": peer,
                "alias": alias,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/alias", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "failed to set peer alias (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value = serde_json::from_slice(&resp)
                .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
            let device_id = v.get("device_id").and_then(|s| s.as_str()).unwrap_or(&peer);
            let new_alias = v.get("alias").and_then(|s| s.as_str()).unwrap_or(&alias);
            println!("Updated alias for {device_id} -> \"{new_alias}\"");
        }
        Cmd::Routes { json, ui_port } => {
            print_routes_list(ui_port, json).await?;
        }
        Cmd::Route { action } => match action {
            RouteCmd::List { json, ui_port } => {
                print_routes_list(ui_port, json).await?;
            }
            RouteCmd::Add {
                peer,
                addr,
                json,
                ui_port,
            } => {
                let payload = serde_json::json!({
                    "peer": peer,
                    "addr": addr,
                });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/routes", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "route add failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                    return Ok(());
                }
                let rep: tetherly_node::RouteMutateReport = serde_json::from_slice(&resp)?;
                println!(
                    "Route {}: {} -> {}",
                    rep.action,
                    rep.device_id,
                    rep.addr.unwrap_or_default()
                );
            }
            RouteCmd::Remove {
                peer,
                json,
                ui_port,
            } => {
                let path = format!("/api/routes?peer={}", peer.trim());
                let (code, resp) = tetherly_node::loopback_delete(ui_port, &path)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "route remove failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                    return Ok(());
                }
                let rep: tetherly_node::RouteMutateReport = serde_json::from_slice(&resp)?;
                println!("Route {}: {}", rep.action, rep.device_id);
            }
        },
        Cmd::Net {
            interfaces,
            json,
            ui_port,
        } => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/net")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
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
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/ancs")
                    .await
                    .map_err(|e| {
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
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/ancs/connect", b"{}")
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
                let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/ancs/allow", &bytes)
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
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/storage")
                    .await
                    .map_err(|e| {
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
                    let rep: tetherly_node::StorageReport = serde_json::from_slice(&resp)?;
                    println!("Node Storage Status:");
                    println!("  Data Directory:    {}", rep.data_dir);
                    println!("  Inbox Directory:   {}", rep.inbox_dir);
                    println!(
                        "  Inbox Files:       {} ({})",
                        rep.inbox_file_count,
                        format_bytes(rep.inbox_total_bytes)
                    );
                    println!(
                        "  Partial In-flight: {} ({})",
                        rep.partial_count,
                        format_bytes(rep.partial_bytes)
                    );
                    println!(
                        "  Orphaned Partials: {} ({})",
                        rep.orphaned_partial_count,
                        format_bytes(rep.orphaned_partial_bytes)
                    );
                    println!("  Metadata Storage:  {}", format_bytes(rep.meta_bytes));
                }
            }
            StorageCmd::Clean { all, ui_port } => {
                let payload = serde_json::json!({ "orphaned_only": !all });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/storage/clean", &bytes)
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
                let rep: tetherly_node::StorageCleanReport = serde_json::from_slice(&resp)?;
                println!("Storage cleanup complete:");
                println!("  Freed Space:     {}", format_bytes(rep.freed_bytes));
                println!("  Deleted Files:   {}", rep.deleted_files_count);
                if rep.cleared_resume_entries > 0 {
                    println!("  Cleared Resumes: {}", rep.cleared_resume_entries);
                }
            }
        },
        Cmd::Status {
            watch,
            interval,
            json,
            ui_port,
        } => loop {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/state")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
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
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/health")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            let rtt = t0.elapsed();
            if code != 200 {
                anyhow::bail!(
                    "node health check failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let rep: tetherly_node::HealthReport = serde_json::from_slice(&resp)?;
                println!("Node Health Status: {} (200 OK)", rep.status);
                println!("  Version:          {}", rep.version);
                println!("  Device ID:        {}", rep.device_id);
                println!("  Node Name:        {} ({})", rep.name, rep.platform);
                println!("  PID:              {}", rep.pid);
                println!(
                    "  Uptime:           {} ({}s)",
                    format_uptime(rep.uptime_secs),
                    rep.uptime_secs
                );
                println!("  Bound Ports:");
                println!("    Control (Mesh): {}", rep.ports.control);
                println!("    File Transfer:  {}", rep.ports.file);
                println!("    Input (Remote): {}", rep.ports.input);
                println!("    Screen (Video): {}", rep.ports.screen);
                println!("    UI (Loopback):  {}", rep.ports.ui);
                println!("  Live Peers:       {}", rep.live_peers);
                println!("  Trusted Peers:    {}", rep.trusted_peers);
                println!("  Loopback Latency: {:.2}ms", rtt.as_secs_f64() * 1000.0);
            }
        }
        Cmd::Config {
            name,
            advertise,
            reconnect,
            json,
            ui_port,
        } => {
            let has_updates = name.is_some() || advertise.is_some() || reconnect.is_some();
            let (code, resp) = if has_updates {
                let body = serde_json::json!({
                    "name": name,
                    "advertise": advertise,
                    "reconnect": reconnect,
                });
                let bytes = serde_json::to_vec(&body)?;
                tetherly_node::loopback_post(ui_port, "/api/config", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?
            } else {
                tetherly_node::loopback_get(ui_port, "/api/config")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?
            };
            if code != 200 {
                anyhow::bail!(
                    "node config query/update failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let rep: tetherly_node::NodeConfigReport = serde_json::from_slice(&resp)?;
                if has_updates {
                    println!("Tetherly Node Configuration (Updated):");
                } else {
                    println!("Tetherly Node Configuration:");
                }
                println!("  Device ID:        {}", rep.device_id);
                println!("  Node Name:        {} ({})", rep.name, rep.platform);
                println!("  Data Directory:   {}", rep.data_dir);
                println!(
                    "  Loopback Only:    {}",
                    if rep.loopback_only {
                        "true (127.0.0.1 only)"
                    } else {
                        "false (LAN + Loopback)"
                    }
                );
                println!(
                    "  mDNS Advertise:   {}",
                    if rep.advertise { "enabled" } else { "disabled" }
                );
                println!(
                    "  Auto Reconnect:   {}",
                    if rep.reconnect { "enabled" } else { "disabled" }
                );
                println!("  Memory Clip:      {}", rep.memory_clip);
                println!(
                    "  Overlay Sidecar:  {}",
                    if rep.overlay_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
                println!(
                    "  ANCS Service:     {}",
                    if rep.ancs_attached {
                        format!("attached ({})", rep.ancs_peripheral)
                    } else {
                        format!("detached ({})", rep.ancs_peripheral)
                    }
                );
                println!("  Bound Ports:");
                println!("    Control (Mesh): {}", rep.control_port);
                println!("    File Transfer:  {}", rep.file_port);
                println!("    Input (Remote): {}", rep.input_port);
                println!("    Screen (Video): {}", rep.screen_port);
                println!("    UI (Loopback):  {}", rep.ui_port);
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
            let mut last_rep = None;
            for i in 1..=count {
                let t0 = std::time::Instant::now();
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/health")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                let rtt = t0.elapsed();
                if code != 200 {
                    anyhow::bail!(
                        "node ping failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let rep: tetherly_node::HealthReport = serde_json::from_slice(&resp)?;
                let rtt_ms = rtt.as_secs_f64() * 1000.0;
                rtts.push(rtt_ms);
                last_rep = Some(rep.clone());
                if !json {
                    println!(
                        "[{i}/{count}] pong: node {}, rtt: {rtt_ms:.2}ms, uptime: {} (pid: {}, v{})",
                        rep.status,
                        format_uptime(rep.uptime_secs),
                        rep.pid,
                        rep.version
                    );
                }
                if i < count {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
            if json {
                if let Some(rep) = last_rep {
                    let avg_rtt = rtts.iter().copied().sum::<f64>() / rtts.len() as f64;
                    let mut val = serde_json::to_value(&rep)?;
                    val["rtt_ms"] = serde_json::json!(avg_rtt);
                    val["count"] = serde_json::json!(count);
                    println!("{val}");
                }
            }
        }
        Cmd::Stop { ui_port } => {
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/daemon/shutdown", b"")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
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
            let mut sse = tetherly_node::loopback_events_connect(ui_port)
                .await
                .map_err(|e| {
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
                        anyhow::bail!("event stream error: {e}");
                    }
                }
            }
        }
        Cmd::Metrics {
            reset,
            json,
            ui_port,
        }
        | Cmd::Traffic {
            reset,
            json,
            ui_port,
        } => {
            handle_traffic_cmd(reset, json, ui_port).await?;
        }
        Cmd::Caps {
            peer,
            enable,
            disable,
            json,
            ui_port,
        } => {
            handle_caps_cmd(peer, enable, disable, json, ui_port).await?;
        }
        Cmd::Connect {
            peer,
            addr,
            json,
            ui_port,
        } => {
            let payload = serde_json::json!({
                "peer": peer,
                "addr": addr,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/peers/connect", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused connect (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: tetherly_node::PeerConnectReport = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let display_name = rep.alias.or(rep.name).unwrap_or_else(|| "peer".to_string());
                if rep.already_connected {
                    println!(
                        "peer {} ({}) is already connected (addr: {})",
                        rep.device_id,
                        display_name,
                        rep.addr.as_deref().unwrap_or("unknown")
                    );
                } else {
                    println!(
                        "successfully connected to {} ({}) at {}",
                        rep.device_id,
                        display_name,
                        rep.addr.as_deref().unwrap_or("unknown")
                    );
                }
            }
        }
        Cmd::Disconnect {
            peer,
            json,
            ui_port,
        } => {
            let payload = serde_json::json!({ "peer": peer });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/peers/disconnect", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused disconnect (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: tetherly_node::PeerDisconnectReport = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let display_name = rep.alias.or(rep.name).unwrap_or_else(|| "peer".to_string());
                if rep.already_disconnected {
                    println!(
                        "peer {} ({}) is already disconnected",
                        rep.device_id, display_name
                    );
                } else {
                    println!(
                        "successfully disconnected session with {} ({})",
                        rep.device_id, display_name
                    );
                }
            }
        }
    }
    Ok(())
}

async fn handle_traffic_cmd(reset: bool, json: bool, ui_port: u16) -> anyhow::Result<()> {
    if reset {
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/metrics/reset", b"{}")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "traffic telemetry reset failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
        } else {
            println!("Traffic and protocol telemetry counters successfully reset to zero.");
        }
        return Ok(());
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/metrics")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "traffic telemetry query failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let uptime_secs = v["uptime_secs"].as_u64().unwrap_or(0);
    let total_tx_bytes = v["total_tx_bytes"].as_u64().unwrap_or(0);
    let total_rx_bytes = v["total_rx_bytes"].as_u64().unwrap_or(0);
    let total_tx_frames = v["total_tx_frames"].as_u64().unwrap_or(0);
    let total_rx_frames = v["total_rx_frames"].as_u64().unwrap_or(0);

    let ctrl = &v["control"];
    let file = &v["file"];
    let input = &v["input"];
    let screen = &v["screen"];
    let msgs = &v["messages"];

    println!(
        "Network Traffic & Protocol Telemetry (Uptime: {})",
        format_uptime(uptime_secs)
    );
    println!("  Total Throughput:");
    println!(
        "    Outbound (TX): {:<14} ({} frames)",
        format_bytes(total_tx_bytes),
        total_tx_frames
    );
    println!(
        "    Inbound  (RX): {:<14} ({} frames)",
        format_bytes(total_rx_bytes),
        total_rx_frames
    );

    println!("\n  Multi-Channel Bandwidth Distribution:");
    println!(
        "    {:<18} {:<16} {:<16} {:<12} {:<12}",
        "Channel", "TX Bytes", "RX Bytes", "TX Frames", "RX Frames"
    );
    println!("    {}", "-".repeat(76));
    for (name, ch) in [
        ("Control (Noise)", ctrl),
        ("File Transfer", file),
        ("Input Stream", input),
        ("Screen Capture", screen),
    ] {
        let tx_b = ch["tx_bytes"].as_u64().unwrap_or(0);
        let rx_b = ch["rx_bytes"].as_u64().unwrap_or(0);
        let tx_f = ch["tx_frames"].as_u64().unwrap_or(0);
        let rx_f = ch["rx_frames"].as_u64().unwrap_or(0);
        println!(
            "    {:<18} {:<16} {:<16} {:<12} {:<12}",
            name,
            format_bytes(tx_b),
            format_bytes(rx_b),
            tx_f,
            rx_f
        );
    }

    println!("\n  Core Protocol Message Distribution:");
    println!(
        "    {:<18} {:<12} {:<12}",
        "Message Type", "TX Count", "RX Count"
    );
    println!("    {}", "-".repeat(44));
    for (name, tx_key, rx_key) in [
        ("Ping", "ping_tx", "ping_rx"),
        ("Pong", "pong_tx", "pong_rx"),
        ("Notify", "notify_tx", "notify_rx"),
        ("Clipboard", "clip_tx", "clip_rx"),
        ("File Offer", "file_offer_tx", "file_offer_rx"),
    ] {
        let tx_c = msgs[tx_key].as_u64().unwrap_or(0);
        let rx_c = msgs[rx_key].as_u64().unwrap_or(0);
        println!("    {:<18} {:<12} {:<12}", name, tx_c, rx_c);
    }

    Ok(())
}

async fn handle_caps_cmd(
    peer: Option<String>,
    enable: Option<String>,
    disable: Option<String>,
    json: bool,
    ui_port: u16,
) -> anyhow::Result<()> {
    if enable.is_some() || disable.is_some() {
        let mut enables = Vec::new();
        if let Some(e) = enable {
            enables.push(e);
        }
        let mut disables = Vec::new();
        if let Some(d) = disable {
            disables.push(d);
        }
        let req_body = serde_json::json!({
            "enable": enables,
            "disable": disables,
        });
        let (code, resp) = tetherly_node::loopback_post(
            ui_port,
            "/api/caps",
            serde_json::to_vec(&req_body).unwrap_or_default().as_slice(),
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
        if code != 200 {
            anyhow::bail!(
                "node returned HTTP {code}: {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
        let rep: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let dev = rep["device_id"].as_str().unwrap_or("-");
        let feats = rep["features"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let fp = rep["file_port"].as_u64().unwrap_or(0);
        let ip = rep["input_port"].as_u64().unwrap_or(0);
        let sp = rep["screen_port"].as_u64().unwrap_or(0);

        println!("Local node capabilities updated:");
        println!("  Node ID:     {dev}");
        println!("  Features:    [{feats}]");
        println!("  Ports:       file: {fp}, input: {ip}, screen: {sp}");
        return Ok(());
    }

    if let Some(ref target) = peer {
        let path = format!("/api/peers/caps?peer={target}");
        let (code, resp) = tetherly_node::loopback_get(ui_port, &path)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node returned HTTP {code}: {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
        let rep: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!("Peer Capabilities ({target}):");
        if let Some(arr) = rep["peers"].as_array() {
            if arr.is_empty() {
                println!("  (peer not found)");
            } else {
                for p in arr {
                    let dev = p["device_id"].as_str().unwrap_or("-");
                    let name = p["name"].as_str().unwrap_or("-");
                    let alias = p["alias"].as_str();
                    let alias_str = match alias {
                        Some(a) if !a.is_empty() && a != name => format!(" (alias: \"{a}\")"),
                        _ => String::new(),
                    };
                    let online = p["online"].as_bool().unwrap_or(false);
                    let online_str = if online { "ONLINE" } else { "OFFLINE" };
                    let feats = p["features"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    let fp = p["file_port"]
                        .as_u64()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "n/a".into());
                    let ip = p["input_port"]
                        .as_u64()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "n/a".into());
                    let sp = p["screen_port"]
                        .as_u64()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "n/a".into());

                    println!("  * [{online_str}] {name}{alias_str} · {dev}");
                    println!("    Features: [{feats}]");
                    println!("    Ports:    file: {fp}, input: {ip}, screen: {sp}");
                }
            }
        }
        return Ok(());
    }

    // Both local and peers
    let (code_local, resp_local) = tetherly_node::loopback_get(ui_port, "/api/caps")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    let (code_peers, resp_peers) = tetherly_node::loopback_get(ui_port, "/api/peers/caps")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code_local != 200 {
        anyhow::bail!(
            "node returned HTTP {code_local}: {}",
            String::from_utf8_lossy(&resp_local)
        );
    }
    if code_peers != 200 {
        anyhow::bail!(
            "node returned HTTP {code_peers}: {}",
            String::from_utf8_lossy(&resp_peers)
        );
    }
    if json {
        let local_v: serde_json::Value = serde_json::from_slice(&resp_local)
            .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let peers_v: serde_json::Value = serde_json::from_slice(&resp_peers)
            .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!(
            "{}",
            serde_json::json!({
                "local": local_v,
                "peers": peers_v,
            })
        );
        return Ok(());
    }

    let local_v: serde_json::Value =
        serde_json::from_slice(&resp_local).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    let peers_v: serde_json::Value =
        serde_json::from_slice(&resp_peers).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;

    let self_id = local_v["device_id"].as_str().unwrap_or("-");
    let local_feats = local_v["features"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let fp = local_v["file_port"].as_u64().unwrap_or(0);
    let ip = local_v["input_port"].as_u64().unwrap_or(0);
    let sp = local_v["screen_port"].as_u64().unwrap_or(0);

    println!("Local Node Capabilities:");
    println!("  Node ID:     {self_id}");
    println!("  Features:    [{local_feats}]");
    println!("  Ports:       file: {fp}, input: {ip}, screen: {sp}");

    let count = peers_v["count"].as_u64().unwrap_or(0);
    println!("\nConnected Peers Capabilities ({count}):");
    if let Some(arr) = peers_v["peers"].as_array() {
        if arr.is_empty() {
            println!("  (no live peers connected)");
        } else {
            for p in arr {
                let dev = p["device_id"].as_str().unwrap_or("-");
                let name = p["name"].as_str().unwrap_or("-");
                let alias = p["alias"].as_str();
                let alias_str = match alias {
                    Some(a) if !a.is_empty() && a != name => format!(" (alias: \"{a}\")"),
                    _ => String::new(),
                };
                let feats = p["features"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let p_fp = p["file_port"]
                    .as_u64()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".into());
                let p_ip = p["input_port"]
                    .as_u64()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".into());
                let p_sp = p["screen_port"]
                    .as_u64()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".into());

                println!("  * {name}{alias_str} · {dev}");
                println!("    Features: [{feats}]");
                println!("    Ports:    file: {p_fp}, input: {p_ip}, screen: {p_sp}");
            }
        }
    }
    Ok(())
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
        "metrics_reset" => {
            let ts = v["timestamp_ms"].as_u64().unwrap_or(0);
            println!("[metrics_reset] timestamp_ms={ts}");
        }
        "peer_caps_changed" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let feats = v["features"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("[peer_caps_changed] peer={dev} features=[{feats}]");
        }
        other => {
            println!("[{other}] {v}");
        }
    }
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
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/ping", &body_bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "peer ping failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let rtt = v["rtt_ms"].as_u64().unwrap_or(0);
        rtts.push(rtt);
        if !json {
            let dev = v["device_id"].as_str().unwrap_or("-");
            let path = v["path"].as_str().unwrap_or("-");
            let addr = v["addr"].as_str().unwrap_or("-");
            println!("[{i}/{count}] Ping {dev} ({addr} [{path}]): time={rtt}ms");
        }
        last_rep = Some(v);
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

async fn do_transfers(
    ui_port: u16,
    id: Option<String>,
    active: bool,
    terminal: bool,
    direction: Option<String>,
    json: bool,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(ref target_id) = id {
        query_params.push(format!("id={target_id}"));
    }
    if active {
        query_params.push("active=true".to_string());
    }
    if terminal {
        query_params.push("terminal=true".to_string());
    }
    if let Some(ref dir) = direction {
        query_params.push(format!("direction={dir}"));
    }

    let path = if query_params.is_empty() {
        "/api/file/transfers".to_string()
    } else {
        format!("/api/file/transfers?{}", query_params.join("&"))
    };

    let (code, resp) = tetherly_node::loopback_get(ui_port, &path)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code == 404 {
        if let Some(ref target_id) = id {
            anyhow::bail!("transfer not found: {target_id}");
        } else {
            anyhow::bail!("endpoint returned 404: {}", String::from_utf8_lossy(&resp));
        }
    }
    if code != 200 {
        anyhow::bail!(
            "query transfers failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    if id.is_some() {
        let tid = v.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
        let filename = v.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
        let dir = v.get("direction").and_then(|s| s.as_str()).unwrap_or("?");
        let peer = v.get("peer").and_then(|s| s.as_str()).unwrap_or("?");
        let total = v.get("total").and_then(|s| s.as_u64()).unwrap_or(0);
        let transferred = v.get("transferred").and_then(|s| s.as_u64()).unwrap_or(0);
        let speed = v.get("speed_bps").and_then(|s| s.as_u64()).unwrap_or(0);
        let phase = v.get("phase").and_then(|s| s.as_str()).unwrap_or("?");
        let started_ms = v.get("started_ms").and_then(|s| s.as_u64()).unwrap_or(0);
        let error = v.get("error").and_then(|s| s.as_str());
        let pct = if total > 0 {
            (transferred as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        println!("Transfer {tid}:");
        println!("  File:      {filename}");
        println!("  Direction: {dir}");
        println!("  Peer:      {peer}");
        println!("  Phase:     {phase}");
        println!(
            "  Progress:  {:>5.1}% ({} / {})",
            pct,
            format_bytes(transferred),
            format_bytes(total)
        );
        if speed > 0 {
            println!("  Speed:     {}", format_speed(speed));
        }
        if started_ms > 0 {
            println!("  Started:   {started_ms} ms");
        }
        if let Some(err_msg) = error {
            println!("  Error:     {err_msg}");
        }
        return Ok(());
    }

    let active_count = v.get("active_count").and_then(|s| s.as_u64()).unwrap_or(0);
    let terminal_count = v
        .get("terminal_count")
        .and_then(|s| s.as_u64())
        .unwrap_or(0);
    let total_count = v.get("total_count").and_then(|s| s.as_u64()).unwrap_or(0);

    println!("Transfers: {total_count} total ({active_count} active, {terminal_count} terminal)");

    if let Some(stats) = v.get("stats") {
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
            "  Cumulative Metrics: sent: {}, received: {}, completed: {}, failed: {}",
            format_bytes(sent),
            format_bytes(recv),
            completed,
            failed
        );
    }

    let transfers = v.get("transfers").and_then(|t| t.as_array());
    match transfers {
        Some(list) if !list.is_empty() => {
            println!("\n  Transfers List:");
            for t in list {
                let id = t.get("transfer_id").and_then(|v| v.as_str()).unwrap_or("?");
                let filename = t.get("filename").and_then(|v| v.as_str()).unwrap_or("?");
                let transferred = t.get("transferred").and_then(|v| v.as_u64()).unwrap_or(0);
                let total = t.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                let speed_bps = t.get("speed_bps").and_then(|v| v.as_u64()).unwrap_or(0);
                let phase = t.get("phase").and_then(|v| v.as_str()).unwrap_or("?");
                let dir = t.get("direction").and_then(|v| v.as_str()).unwrap_or("?");
                let pct = if total > 0 {
                    (transferred as f64 / total as f64) * 100.0
                } else {
                    0.0
                };
                let speed_str = if speed_bps > 0 {
                    format!("{} ", format_speed(speed_bps))
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
        _ => {
            println!("  No matching transfers recorded.");
        }
    }

    Ok(())
}

async fn print_routes_list(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/routes")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
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
                let alias_str = r
                    .get("alias")
                    .and_then(|a| a.as_str())
                    .map(|a| format!(" (alias: \"{a}\")"))
                    .unwrap_or_default();
                let online_str = if r["online"].as_bool().unwrap_or(false) {
                    "[online]"
                } else {
                    "[offline]"
                };
                println!("  * {dev}{alias_str} -> {addr} {online_str}");
            }
        }
        _ => println!("  (no cached routes)"),
    }
    Ok(())
}
