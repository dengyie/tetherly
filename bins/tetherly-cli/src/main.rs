// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 0 CLI: listen / dial / pair / ping. No GUI, no BLE, no EasyTier.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
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
    /// Show, inspect or clear the 8-digit pairing PIN on the *running* node.
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
    /// Listen on 127.0.0.1:port, pair with the given PIN, then answer one ping.
    Listen {
        #[arg(long, default_value_t = CONTROL_PORT)]
        port: u16,
        #[arg(long)]
        pin: String,
        #[arg(long, default_value = "host")]
        name: String,
    },
    /// Ask the *running* node to dial a peer and pair or resume.
    Dial {
        #[arg(long)]
        addr: SocketAddr,
        #[arg(long)]
        pin: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
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
    /// Ask the *running* node to send one or more files to a live peer.
    ///
    /// The file is read from the local filesystem and offered over the live
    /// session. Supports multiple paths for batch sending. With one live peer
    /// the target is implied; with more than one, `--peer` is required.
    Send {
        /// Path(s) to the file(s) to send.
        #[arg(required = true, num_args = 1..)]
        paths: Vec<std::path::PathBuf>,
        /// Optional filename override to declare to the peer (single file only).
        #[arg(long)]
        name: Option<String>,
        /// `tdev_…` of the peer. Omit only when exactly one peer is live.
        #[arg(long)]
        peer: Option<String>,
        /// Output response as JSON.
        #[arg(long)]
        json: bool,
        /// Watch transfer progress until completion (single file only).
        #[arg(long)]
        watch: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect recoverable/interrupted transfer markers and sidecars.
    Resume {
        /// Output raw JSON instead of table.
        #[arg(long)]
        json: bool,
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
    /// List pending incoming file transfer offers.
    Offers {
        /// Filter offers by source peer ID or alias.
        #[arg(long)]
        peer: Option<String>,
        /// Filter by specific transfer ID.
        #[arg(long)]
        id: Option<String>,
        /// Output raw JSON instead of table.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Accept one or all pending incoming file transfer offers.
    Accept {
        /// Transfer ID to accept (e.g. tfl_...).
        id: Option<String>,
        /// Accept all pending offers (optionally filtered by peer).
        #[arg(long)]
        all: bool,
        /// Filter by source peer ID or alias when accepting.
        #[arg(long)]
        peer: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reject one or all pending incoming file transfer offers.
    Reject {
        /// Transfer ID to reject (e.g. tfl_...).
        id: Option<String>,
        /// Reject all pending offers (optionally filtered by peer).
        #[arg(long)]
        all: bool,
        /// Filter by source peer ID or alias when rejecting.
        #[arg(long)]
        peer: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Query or configure automatic acceptance of files from trusted peers.
    AutoAccept {
        /// Enable auto-accepting files from trusted peers.
        #[arg(long)]
        enable: bool,
        /// Disable auto-accepting files from trusted peers.
        #[arg(long)]
        disable: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
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
        /// Filter candidates by source peer ID or alias.
        #[arg(long)]
        peer: Option<String>,
        /// Filter candidates by application ID.
        #[arg(long)]
        app_id: Option<String>,
        /// Filter candidates by presence of OTP.
        #[arg(long)]
        has_otp: Option<bool>,
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
    /// Dismiss active notification candidate(s) by ID, or batch dismiss by peer, app, or all.
    Dismiss {
        /// Candidate ID (e.g. ntc_...). Optional if --all, --peer, or --app-id is specified.
        id: Option<String>,
        /// Filter by source peer (device ID or alias) to dismiss candidates from.
        #[arg(long)]
        peer: Option<String>,
        /// Filter by app ID (e.g. com.tencent.mm) to dismiss candidates from.
        #[arg(long)]
        app_id: Option<String>,
        /// Dismiss all active notification candidates.
        #[arg(long)]
        all: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
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
        /// Actively trigger dynamic rescan of network interfaces and EasyTier sidecar probe.
        #[arg(long, short)]
        refresh: bool,
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
    /// Manage received files in the incoming inbox (list, delete, clear).
    Inbox {
        #[command(subcommand)]
        action: InboxCmd,
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
    /// Block a peer device from connecting or pairing.
    Block {
        /// Target peer (device_id like tdev_... or friendly alias/name).
        #[arg(long)]
        peer: String,
        /// Optional reason for blocking.
        #[arg(long)]
        reason: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Unblock a previously blocked peer device.
    Unblock {
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
    /// List all currently blocked peer devices.
    Blocked {
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
    /// View clipboard status, size, and synchronization statistics without leaking plaintext.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect bounded clipboard synchronization history without leaking plaintext.
    History {
        /// Maximum number of history entries to return (1-32, default: 32).
        #[arg(long)]
        limit: Option<usize>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clear the local system clipboard and reset synchronization history.
    Clear {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
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
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Revoke local screen viewing consent.
    Revoke {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Start viewing a remote peer's screen (as visitor).
    Start {
        /// Target peer device_id (tdev_...). Required if multiple peers are connected.
        #[arg(long)]
        peer: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stop screen streaming.
    Stop {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
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
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stop remote keyboard/mouse channel and return cursor locally.
    Stop {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
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
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Disconnect ANCS Bluetooth subscription and enter idle or backoff state.
    Disconnect {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
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
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Remove an application ID from the notification open allowlist.
    Remove {
        /// Target application bundle ID (e.g. com.tencent.xin).
        app_id: String,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clear all rules from the notification open allowlist.
    Clear {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
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
enum InboxCmd {
    /// List completed received files in the incoming inbox.
    List {
        /// Filter by filename search term (case-insensitive substring).
        #[arg(long, short)]
        search: Option<String>,
        /// Limit maximum number of displayed entries.
        #[arg(long, short)]
        limit: Option<usize>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Safely delete a specific file from the incoming inbox.
    Delete {
        /// Filename to delete (basename inside inbox).
        name: String,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clear completed received files from the incoming inbox.
    Clear {
        /// Skip confirmation prompt.
        #[arg(long, short = 'y')]
        yes: bool,
        /// Only delete files modified before this UNIX timestamp (seconds).
        #[arg(long)]
        before_secs: Option<u64>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
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

async fn loopback_delete(port: u16, path: &str) -> std::io::Result<(u16, Vec<u8>)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await?;
    let head = format!("DELETE {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
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
        "file_rejected" => {
            let tid = v["transfer_id"].as_str().unwrap_or("?");
            let src = v["source"].as_str().unwrap_or("?");
            let reason = v["reason"]
                .as_str()
                .map(|r| format!(" reason=\"{r}\""))
                .unwrap_or_default();
            println!("[file_rejected] transfer={tid} peer={src}{reason}");
        }
        "file_auto_accept_changed" => {
            let enabled = v["enabled"].as_bool().unwrap_or(false);
            println!("[file_auto_accept_changed] enabled={enabled}");
        }
        "inbox_file_deleted" => {
            let name = v["name"].as_str().unwrap_or("?");
            let freed = v["freed_bytes"].as_u64().unwrap_or(0);
            println!(
                "[inbox_file_deleted] name={name} freed={}",
                format_bytes(freed)
            );
        }
        "inbox_cleared" => {
            let count = v["deleted_count"].as_u64().unwrap_or(0);
            let freed = v["freed_bytes"].as_u64().unwrap_or(0);
            println!(
                "[inbox_cleared] deleted={count} freed={}",
                format_bytes(freed)
            );
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
        "candidate_dismissed" => {
            let id = v["id"].as_str().unwrap_or("?");
            let source = v["source"].as_str().unwrap_or("?");
            let app = v["app_id"].as_str().unwrap_or("?");
            println!("[candidate_dismissed] id={id} peer={source} app={app}");
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
        "peer_blocked" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let reason = v["reason"]
                .as_str()
                .map(|r| format!(" reason=\"{r}\""))
                .unwrap_or_default();
            println!("[peer_blocked] peer={dev}{reason}");
        }
        "peer_unblocked" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            println!("[peer_unblocked] peer={dev}");
        }
        "clipboard_synced" => {
            let seq = v["seq"].as_u64().unwrap_or(0);
            let dir = v["direction"].as_str().unwrap_or("?");
            let src = v["source"].as_str().unwrap_or("local");
            let bytes = v["bytes"].as_u64().unwrap_or(0);
            let hash = v["hash"].as_str().unwrap_or("");
            let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
            let peers = v["sent_peers"].as_u64().unwrap_or(0);
            if dir == "outbound" {
                println!(
                    "[clipboard_synced] seq={seq} outbound from={src} ({bytes} bytes, sha256:{short_hash}...) to {peers} peer(s)"
                );
            } else {
                println!(
                    "[clipboard_synced] seq={seq} inbound from={src} ({bytes} bytes, sha256:{short_hash}...)"
                );
            }
        }
        "clipboard_cleared" => {
            let ts = v["timestamp_ms"].as_u64().unwrap_or(0);
            println!("[clipboard_cleared] timestamp_ms={ts}");
        }
        "screen_state_changed" => {
            let state = v["state"].as_str().unwrap_or("unknown");
            let allowed = v["allowed"].as_bool().unwrap_or(false);
            let peer = v["peer"]
                .as_str()
                .map(|p| format!(" peer={p}"))
                .unwrap_or_default();
            println!("[screen_state_changed] state={state} allowed={allowed}{peer}");
        }
        "input_seat_changed" => {
            let seat = v["seat"].as_str().unwrap_or("unknown");
            let open = v["channel_open"].as_bool().unwrap_or(false);
            let peer = v["peer"]
                .as_str()
                .map(|p| format!(" peer={p}"))
                .unwrap_or_default();
            println!("[input_seat_changed] seat={seat} channel_open={open}{peer}");
        }
        "network_refreshed" => {
            let iface_count = v["iface_count"].as_u64().unwrap_or(0);
            let listen_count = v["listen_count"].as_u64().unwrap_or(0);
            let overlay_present = v["overlay_present"].as_bool().unwrap_or(false);
            let overlay_source = v["overlay_source"].as_str().unwrap_or("Absent");
            println!(
                "[network_refreshed] scanned={iface_count} listening={listen_count} overlay={overlay_present} ({overlay_source})"
            );
        }
        "overlay_state_changed" => {
            let present = v["present"].as_bool().unwrap_or(false);
            let source = v["source"].as_str().unwrap_or("Absent");
            let peer_count = v["peer_count"].as_u64().unwrap_or(0);
            let local_ips = v["local_ips"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!(
                "[overlay_state_changed] present={present} source={source} peers={peer_count} local_ips=[{local_ips}]"
            );
        }
        "ancs_state_changed" => {
            let state = v["state"].as_str().unwrap_or("unknown");
            let attached = v["attached"].as_bool().unwrap_or(false);
            let retry = v["retry_at_ms"]
                .as_u64()
                .map(|r| format!(" retry_at_ms={r}"))
                .unwrap_or_default();
            println!("[ancs_state_changed] state={state} attached={attached}{retry}");
        }
        "ancs_rule_changed" => {
            let action = v["action"].as_str().unwrap_or("unknown");
            let app_id = v["app_id"]
                .as_str()
                .map(|a| format!(" app_id={a}"))
                .unwrap_or_default();
            let url = v["url"]
                .as_str()
                .map(|u| format!(" url={u}"))
                .unwrap_or_default();
            let rules_count = v["rules_count"].as_u64().unwrap_or(0);
            println!("[ancs_rule_changed] action={action}{app_id}{url} rules_count={rules_count}");
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
        Cmd::Pin {
            status,
            clear,
            json,
            ui_port,
        } => {
            if clear {
                let (code, resp) = loopback_post(ui_port, "/api/pin/clear", b"{}")
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
                let (code, resp) = loopback_get(ui_port, "/api/pin").await.map_err(|e| {
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
                let (code, resp) =
                    loopback_post(ui_port, "/api/pin", b"{}")
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
        Cmd::Dial { addr, pin, ui_port } => {
            let payload = serde_json::json!({ "addr": addr.to_string(), "pin": pin });
            let body = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/dial", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused dial request (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            println!("dial dispatched to {addr}");
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
            paths,
            name,
            peer,
            json,
            watch,
            ui_port,
        } => {
            if paths.is_empty() {
                bail!("no file paths provided");
            }
            if paths.len() == 1 {
                let path = &paths[0];
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                    return Ok(());
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
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
                            let speed_bps =
                                t.get("speed_bps").and_then(|v| v.as_u64()).unwrap_or(0);
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
            } else {
                if name.is_some() {
                    bail!("--name cannot be specified when sending multiple files in batch");
                }
                let path_strs: Vec<String> = paths
                    .iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();
                let payload = serde_json::json!({
                    "paths": path_strs,
                    "peer": peer,
                });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = loopback_post(ui_port, "/api/file/send-batch", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "node refused batch file send (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                    return Ok(());
                }
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let batch_id = rep["batch_id"].as_str().unwrap_or("?");
                let target = rep["peer_alias"]
                    .as_str()
                    .or_else(|| rep["peer"].as_str())
                    .unwrap_or("?");
                let succ = rep["successful_files"].as_u64().unwrap_or(0);
                let total_f = rep["total_files"].as_u64().unwrap_or(0);
                let failed_f = rep["failed_files"].as_u64().unwrap_or(0);
                let total_bytes = rep["total_bytes"].as_u64().unwrap_or(0);
                println!(
                    "Batch [{batch_id}] to {target}: {succ}/{total_f} succeeded, {failed_f} failed ({} total)",
                    format_bytes(total_bytes)
                );
                if let Some(items) = rep["items"].as_array() {
                    for item in items {
                        let iname = item["name"].as_str().unwrap_or("?");
                        let isize = item["size"].as_u64().unwrap_or(0);
                        let status = item["status"].as_str().unwrap_or("?");
                        if status == "sent" || status == "queued" {
                            let tid = item["transfer_id"].as_str().unwrap_or("?");
                            println!("  ✓ {} ({}) -> [{tid}]", iname, format_bytes(isize));
                        } else {
                            let err = item["error"].as_str().unwrap_or("unknown error");
                            println!("  ✗ {iname} -> failed: {err}");
                        }
                    }
                }
            }
        }
        Cmd::Resume { json, ui_port } => {
            let (code, resp) = loopback_get(ui_port, "/api/file/resume")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "node returned HTTP {code} querying resume markers: {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
                return Ok(());
            }
            let rep: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let markers = rep["markers"].as_array();
            let count = rep["count"].as_u64().unwrap_or(0);
            let total_part = rep["total_partial_bytes"].as_u64().unwrap_or(0);
            if count == 0 || markers.map(|m| m.is_empty()).unwrap_or(true) {
                println!("No resumable transfer markers found in inbox.");
                return Ok(());
            }
            println!(
                "Found {} resumable marker(s) ({} total partial data):",
                count,
                format_bytes(total_part)
            );
            println!(
                "{:<20} {:<24} {:<22} {:<10} {:<10} {:<10}",
                "TRANSFER ID", "SOURCE", "OFFSET / TOTAL", "PROGRESS", "ALIGNED", ".PART FILE"
            );
            if let Some(list) = markers {
                for m in list {
                    let tid = m["transfer_id"].as_str().unwrap_or("?");
                    let src = m["source_alias"]
                        .as_str()
                        .or_else(|| m["source"].as_str())
                        .unwrap_or("?");
                    let off = m["offset_bytes"].as_u64().unwrap_or(0);
                    let tot = m["total_bytes"].as_u64().unwrap_or(0);
                    let pct = m["progress_percent"].as_f64().unwrap_or(0.0);
                    let aligned = if m["chunk_aligned"].as_bool().unwrap_or(false) {
                        "yes"
                    } else {
                        "no"
                    };
                    let part_exists = m["part_file_exists"].as_bool().unwrap_or(false);
                    let part_bytes = m["part_file_bytes"].as_u64().unwrap_or(0);
                    let part = if part_exists {
                        format!("yes ({})", format_bytes(part_bytes))
                    } else {
                        "no".to_string()
                    };
                    println!(
                        "{:<20} {:<24} {:<22} {:<10} {:<10} {:<10}",
                        tid,
                        src,
                        format!("{} / {}", format_bytes(off), format_bytes(tot)),
                        format!("{:.1}%", pct),
                        aligned,
                        part
                    );
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
        Cmd::Offers {
            peer,
            id,
            json,
            ui_port,
        } => {
            do_file_offers(ui_port, peer, id, json).await?;
        }
        Cmd::Accept {
            id,
            all,
            peer,
            json,
            ui_port,
        } => {
            do_file_accept(ui_port, id, all, peer, json).await?;
        }
        Cmd::Reject {
            id,
            all,
            peer,
            json,
            ui_port,
        } => {
            do_file_reject(ui_port, id, all, peer, json).await?;
        }
        Cmd::AutoAccept {
            enable,
            disable,
            json,
            ui_port,
        } => {
            do_file_auto_accept(ui_port, enable, disable, json).await?;
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
            ClipCmd::Status { json, ui_port } => {
                let (code, resp) =
                    loopback_get(ui_port, "/api/clip/status")
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    bail!(
                        "clipboard status query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let has_text = v["has_text"].as_bool().unwrap_or(false);
                    let bytes = v["bytes"].as_u64().unwrap_or(0);
                    let hash = v["hash"].as_str().unwrap_or("-");
                    let last_seq = v["last_seq"].as_u64().unwrap_or(0);
                    let inbound = v["inbound_count"].as_u64().unwrap_or(0);
                    let outbound = v["outbound_count"].as_u64().unwrap_or(0);
                    let history_count = v["history_count"].as_u64().unwrap_or(0);
                    println!("Clipboard Status:");
                    println!("  Has Content:     {has_text}");
                    println!("  Bytes:           {bytes}");
                    println!("  SHA-256 Digest:  {hash}");
                    println!("  Last Sequence:   {last_seq}");
                    println!("  Inbound Synced:  {inbound}");
                    println!("  Outbound Synced: {outbound}");
                    println!("  History Count:   {history_count}");
                }
            }
            ClipCmd::History {
                limit,
                json,
                ui_port,
            } => {
                let path = if let Some(n) = limit {
                    format!("/api/clip/history?limit={n}")
                } else {
                    "/api/clip/history".to_string()
                };
                let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
                if code != 200 {
                    bail!(
                        "clipboard history query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let entries = v["entries"].as_array();
                    match entries {
                        Some(items) if !items.is_empty() => {
                            println!(
                                "{:<6} {:<10} {:<34} {:<8} {:<16} TIME",
                                "SEQ", "DIR", "SOURCE", "BYTES", "SHA-256"
                            );
                            for item in items {
                                let seq = item["seq"].as_u64().unwrap_or(0);
                                let dir = item["direction"].as_str().unwrap_or("-");
                                let src = item["source"].as_str().unwrap_or("-");
                                let bytes = item["bytes"].as_u64().unwrap_or(0);
                                let hash = item["hash"].as_str().unwrap_or("-");
                                let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
                                let ts = item["timestamp_ms"].as_u64().unwrap_or(0);
                                println!(
                                    "{:<6} {:<10} {:<34} {:<8} {:<16} {}ms",
                                    seq, dir, src, bytes, short_hash, ts
                                );
                            }
                        }
                        _ => println!("(no clipboard sync history)"),
                    }
                }
            }
            ClipCmd::Clear { json, ui_port } => {
                let (code, resp) = loopback_post(ui_port, "/api/clip/clear", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "clipboard clear failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let cleared = v["cleared_history_count"].as_u64().unwrap_or(0);
                    println!("clipboard cleared successfully (evicted {cleared} history entries)");
                }
            }
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
        Cmd::Candidates {
            peer,
            app_id,
            has_otp,
            json,
            ui_port,
        } => {
            let mut path = "/api/notify/candidates".to_string();
            let mut query = Vec::new();
            if let Some(ref p) = peer {
                query.push(format!("peer={p}"));
            }
            if let Some(ref a) = app_id {
                query.push(format!("app_id={a}"));
            }
            if let Some(h) = has_otp {
                query.push(format!("has_otp={h}"));
            }
            if !query.is_empty() {
                path = format!("{path}?{}", query.join("&"));
            }
            let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
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
        Cmd::Dismiss {
            id,
            peer,
            app_id,
            all,
            json,
            ui_port,
        } => {
            if id.is_none() && peer.is_none() && app_id.is_none() && !all {
                anyhow::bail!(
                    "dismiss target required: specify candidate ID, or use --all, --peer <peer>, or --app-id <app>"
                );
            }
            let payload = serde_json::json!({
                "id": id,
                "peer": peer,
                "app_id": app_id,
                "all": if all { Some(true) } else { None },
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/notify/dismiss", &bytes)
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
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else if let Ok(batch) = serde_json::from_slice::<serde_json::Value>(&resp) {
                if let Some(count) = batch.get("count").and_then(|c| c.as_u64()) {
                    let synced = batch
                        .get("notified_remote_count")
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0);
                    println!("dismissed {count} candidate(s) (remote peer synced: {synced})");
                } else if let Some(cand_id) = batch.get("id").and_then(|c| c.as_str()) {
                    let remote = batch
                        .get("notified_remote")
                        .and_then(|c| c.as_bool())
                        .unwrap_or(false);
                    println!("dismissed candidate {cand_id} (remote synced: {remote})");
                } else {
                    println!("dismissed successfully");
                }
            } else {
                println!("dismissed successfully");
            }
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
            ScreenCmd::Allow { json, ui_port } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
                    let allowed = v.get("allowed").and_then(|b| b.as_bool()).unwrap_or(true);
                    println!(
                        "Screen viewing allowed (local consent granted). State: {state}, Allowed: {allowed}"
                    );
                }
            }
            ScreenCmd::Revoke { json, ui_port } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
                    let allowed = v.get("allowed").and_then(|b| b.as_bool()).unwrap_or(false);
                    println!("Screen viewing consent revoked. State: {state}, Allowed: {allowed}");
                }
            }
            ScreenCmd::Start {
                peer,
                json,
                ui_port,
            } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
                    let p = v.get("peer").and_then(|s| s.as_str()).unwrap_or("none");
                    println!("Screen streaming started. State: {state}, Target peer: {p}");
                }
            }
            ScreenCmd::Stop { json, ui_port } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let state = v.get("state").and_then(|s| s.as_str()).unwrap_or("unknown");
                    println!("Screen streaming stopped. State: {state}");
                }
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
                    let allowed = v.get("allowed").and_then(|b| b.as_bool()).unwrap_or(false);
                    let open = v
                        .get("channel_open")
                        .and_then(|b| b.as_bool())
                        .unwrap_or(false);
                    let active_peer = v.get("active_peer").and_then(|s| s.as_str());
                    let sent = v.get("sent").and_then(|n| n.as_u64()).unwrap_or(0);
                    let received = v.get("received").and_then(|n| n.as_u64()).unwrap_or(0);
                    let dropped = v.get("dropped").and_then(|n| n.as_u64()).unwrap_or(0);
                    let refused = v.get("refused").and_then(|n| n.as_u64()).unwrap_or(0);
                    let bytes = v.get("bytes").and_then(|n| n.as_u64()).unwrap_or(0);
                    let rx_bytes = v.get("rx_bytes").and_then(|n| n.as_u64()).unwrap_or(0);
                    let p95 = v.get("p95_frame_interval_us").and_then(|n| n.as_u64());

                    println!("Screen Channel Status:");
                    println!("  State:        {state}");
                    println!("  Allowed:      {allowed}");
                    println!("  Channel Open: {open}");
                    if let Some(peer) = active_peer {
                        println!("  Active Peer:  {peer}");
                    }
                    println!("  Frames Sent:  {sent}");
                    println!("  Frames Recv:  {received}");
                    println!("  Dropped:      {dropped}");
                    println!("  Refused:      {refused}");
                    println!("  Bytes Sent:   {bytes}");
                    println!("  Bytes Recv:   {rx_bytes}");
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
            InputCmd::Start {
                peer,
                json,
                ui_port,
            } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let seat = v.get("seat").and_then(|s| s.as_str()).unwrap_or("unknown");
                    let p = v.get("peer").and_then(|s| s.as_str()).unwrap_or("none");
                    println!("Input channel connected. Cursor seat: {seat}, Peer: {p}");
                }
            }
            InputCmd::Stop { json, ui_port } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let seat = v.get("seat").and_then(|s| s.as_str()).unwrap_or("local");
                    println!("Input channel disconnected. Cursor seat: {seat}");
                }
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
                    let active_peer = v.get("active_peer").and_then(|s| s.as_str());
                    println!("Input Channel Status:");
                    println!("  Channel Open: {open}");
                    println!("  Cursor Seat:  {seat}");
                    if let Some(peer) = active_peer {
                        println!("  Active Peer:  {peer}");
                    }
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
            let (code, resp) = loopback_post(ui_port, "/api/peers/alias", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
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
                let (code, resp) = loopback_post(ui_port, "/api/routes", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "route add failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                    return Ok(());
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let action = v
                    .get("action")
                    .and_then(|s| s.as_str())
                    .unwrap_or("updated");
                let device_id = v.get("device_id").and_then(|s| s.as_str()).unwrap_or(&peer);
                let addr_res = v.get("addr").and_then(|s| s.as_str()).unwrap_or(&addr);
                println!("Route {action}: {device_id} -> {addr_res}");
            }
            RouteCmd::Remove {
                peer,
                json,
                ui_port,
            } => {
                let path = format!("/api/routes?peer={}", peer.trim());
                let (code, resp) = loopback_delete(ui_port, &path).await.map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
                if code != 200 {
                    bail!(
                        "route remove failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                    return Ok(());
                }
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let action = v
                    .get("action")
                    .and_then(|s| s.as_str())
                    .unwrap_or("removed");
                let device_id = v.get("device_id").and_then(|s| s.as_str()).unwrap_or(&peer);
                println!("Route {action}: {device_id}");
            }
        },
        Cmd::Net {
            refresh,
            interfaces,
            json,
            ui_port,
        } => {
            if refresh {
                let (code, resp) = loopback_post(ui_port, "/api/net/refresh", b"{}")
                    .await
                    .map_err(|e| {
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
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                println!("Network & Overlay Rescanned:");
                let scanned = v["ifaces_scanned"].as_u64().unwrap_or(0);
                let listening = v["ifaces_listening"].as_u64().unwrap_or(0);
                let present = v["overlay_present"].as_bool().unwrap_or(false);
                let source = v["overlay_source"].as_str().unwrap_or("Absent");
                let peers_count = v["overlay_peers_count"].as_u64().unwrap_or(0);
                let local_ips = v["overlay_local_ips"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let local_str = if local_ips.is_empty() {
                    "(none)".to_string()
                } else {
                    local_ips
                };
                println!("  Interfaces Scanned: {scanned} ({listening} active listening)");
                println!("  EasyTier Sidecar:   {present} ({source})");
                println!("  Overlay Local IPs:  {local_str}");
                println!("  Overlay Mesh Peers: {peers_count}");
                return Ok(());
            }

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
                    let rules = v.get("rules").and_then(|r| r.as_array());
                    let allowlist = v.get("allowlist").and_then(|a| a.as_array());

                    println!("Apple Notification Center Service (ANCS):");
                    println!("  Transport Attached: {attached}");
                    println!("  Ingress State:      {state}");
                    match retry {
                        Some(ms) => println!("  Retry Backoff At:   {ms} ms"),
                        None => println!("  Retry Backoff At:   none"),
                    }
                    if let Some(r_arr) = rules {
                        if !r_arr.is_empty() {
                            println!("  Open Allowlist Rules ({}):", r_arr.len());
                            for r in r_arr {
                                let app = r["app_id"].as_str().unwrap_or("?");
                                let url = r["url"].as_str().unwrap_or("?");
                                println!("    * {app} -> {url}");
                            }
                        } else {
                            println!("  Open Allowlist Rules: none");
                        }
                    } else {
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
            }
            AncsCmd::Connect { json, ui_port } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let state = v["state"].as_str().unwrap_or("unknown");
                    let msg = v["message"]
                        .as_str()
                        .unwrap_or("ANCS connection triggered successfully.");
                    println!("{msg} (state: {state})");
                }
            }
            AncsCmd::Disconnect { json, ui_port } => {
                let (code, resp) = loopback_post(ui_port, "/api/ancs/disconnect", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "ANCS disconnect failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let state = v["state"].as_str().unwrap_or("unknown");
                    let msg = v["message"].as_str().unwrap_or("ANCS disconnected.");
                    println!("{msg} (state: {state})");
                }
            }
            AncsCmd::Allow {
                app_id,
                url,
                json,
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let total = v["total_rules"].as_u64().unwrap_or(0);
                    println!("Allowed open action for '{app_id}' (total rules: {total}).");
                }
            }
            AncsCmd::Remove {
                app_id,
                json,
                ui_port,
            } => {
                let payload = serde_json::json!({ "app_id": app_id });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = loopback_post(ui_port, "/api/ancs/rules/remove", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "ANCS remove rule failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let action = v["action"].as_str().unwrap_or("removed");
                    let total = v["total_rules"].as_u64().unwrap_or(0);
                    if action == "removed" {
                        println!("Removed rule for '{app_id}' (total rules: {total}).");
                    } else {
                        println!("Rule for '{app_id}' not found (total rules: {total}).");
                    }
                }
            }
            AncsCmd::Clear { json, ui_port } => {
                let (code, resp) = loopback_post(ui_port, "/api/ancs/rules/clear", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "ANCS clear rules failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let cleared = v["cleared_count"].as_u64().unwrap_or(0);
                    let total = v["total_rules"].as_u64().unwrap_or(0);
                    println!("Cleared {cleared} allowlist rule(s) (remaining: {total}).");
                }
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
        Cmd::Inbox { action } => match action {
            InboxCmd::List {
                search,
                limit,
                json,
                ui_port,
            } => {
                let mut path = "/api/file/inbox".to_string();
                let mut query_params = Vec::new();
                if let Some(s) = search {
                    query_params.push(format!("search={s}"));
                }
                if let Some(l) = limit {
                    query_params.push(format!("limit={l}"));
                }
                if !query_params.is_empty() {
                    path.push('?');
                    path.push_str(&query_params.join("&"));
                }
                let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
                if code != 200 {
                    bail!(
                        "inbox list query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let count = v["count"].as_u64().unwrap_or(0);
                    let total_bytes = v["total_bytes"].as_u64().unwrap_or(0);
                    let empty_vec = Vec::new();
                    let files = v["files"].as_array().unwrap_or(&empty_vec);

                    if files.is_empty() {
                        println!("Inbox is empty (0 files).");
                    } else {
                        println!(
                            "Inbox Files ({}, total {}):",
                            count,
                            format_bytes(total_bytes)
                        );
                        println!(
                            "{:<32} {:<12} {:<16} {:<6}",
                            "NAME", "SIZE", "MODIFIED", "SAFE"
                        );
                        println!("{:-<70}", "");
                        for f in files {
                            let name = f["name"].as_str().unwrap_or("-");
                            let size = f["size"].as_u64().unwrap_or(0);
                            let mod_secs = f["modified_at_secs"].as_u64().unwrap_or(0);
                            let is_safe = f["is_safe"].as_bool().unwrap_or(false);
                            let mod_str = format_timestamp_ago(mod_secs);
                            let safe_str = if is_safe { "yes" } else { "no" };
                            let name_display = if name.len() > 30 {
                                format!("{}...", &name[..27])
                            } else {
                                name.to_string()
                            };
                            println!(
                                "{:<32} {:<12} {:<16} {:<6}",
                                name_display,
                                format_bytes(size),
                                mod_str,
                                safe_str,
                            );
                        }
                    }
                }
            }
            InboxCmd::Delete {
                name,
                json,
                ui_port,
            } => {
                let payload = serde_json::json!({ "name": name });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = loopback_post(ui_port, "/api/file/inbox/delete", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "inbox delete failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let deleted_name = v["name"].as_str().unwrap_or(&name);
                    let freed_bytes = v["freed_bytes"].as_u64().unwrap_or(0);
                    println!(
                        "Deleted inbox file '{}' (freed {}).",
                        deleted_name,
                        format_bytes(freed_bytes)
                    );
                }
            }
            InboxCmd::Clear {
                yes,
                before_secs,
                json,
                ui_port,
            } => {
                if !yes && !json {
                    eprint!(
                        "Are you sure you want to clear completed files from the inbox? [y/N] "
                    );
                    let mut input = String::new();
                    let _ = std::io::stdin().read_line(&mut input);
                    if !input.trim().eq_ignore_ascii_case("y") {
                        println!("Inbox clear aborted.");
                        return Ok(());
                    }
                }
                let payload = serde_json::json!({ "before_secs": before_secs });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) = loopback_post(ui_port, "/api/file/inbox/clear", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    bail!(
                        "inbox clear failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let v: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    let deleted_count = v["deleted_count"].as_u64().unwrap_or(0);
                    let freed_bytes = v["freed_bytes"].as_u64().unwrap_or(0);
                    println!(
                        "Inbox cleared: deleted {} files (freed {}).",
                        deleted_count,
                        format_bytes(freed_bytes)
                    );
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
                loopback_post(ui_port, "/api/config", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?
            } else {
                loopback_get(ui_port, "/api/config").await.map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?
            };
            if code != 200 {
                bail!(
                    "node config query/update failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                if has_updates {
                    println!("Tetherly Node Configuration (Updated):");
                } else {
                    println!("Tetherly Node Configuration:");
                }
                let device_id = v["device_id"].as_str().unwrap_or("?");
                let name = v["name"].as_str().unwrap_or("?");
                let platform = v["platform"].as_str().unwrap_or("?");
                let data_dir = v["data_dir"].as_str().unwrap_or("?");
                let loopback_only = v["loopback_only"].as_bool().unwrap_or(false);
                let advertise = v["advertise"].as_bool().unwrap_or(false);
                let reconnect = v["reconnect"].as_bool().unwrap_or(false);
                let memory_clip = v["memory_clip"].as_bool().unwrap_or(false);
                let overlay_enabled = v["overlay_enabled"].as_bool().unwrap_or(false);
                let ancs_attached = v["ancs_attached"].as_bool().unwrap_or(false);
                let ancs_peripheral = v["ancs_peripheral"].as_str().unwrap_or("-");
                let control_port = v["control_port"].as_u64().unwrap_or(0);
                let file_port = v["file_port"].as_u64().unwrap_or(0);
                let input_port = v["input_port"].as_u64().unwrap_or(0);
                let screen_port = v["screen_port"].as_u64().unwrap_or(0);
                let ui_port_num = v["ui_port"].as_u64().unwrap_or(0);

                println!("  Device ID:        {device_id}");
                println!("  Node Name:        {name} ({platform})");
                println!("  Data Directory:   {data_dir}");
                println!(
                    "  Loopback Only:    {}",
                    if loopback_only {
                        "true (127.0.0.1 only)"
                    } else {
                        "false (LAN + Loopback)"
                    }
                );
                println!(
                    "  mDNS Advertise:   {}",
                    if advertise { "enabled" } else { "disabled" }
                );
                println!(
                    "  Auto Reconnect:   {}",
                    if reconnect { "enabled" } else { "disabled" }
                );
                println!("  Memory Clip:      {memory_clip}");
                println!(
                    "  Overlay Sidecar:  {}",
                    if overlay_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                );
                println!(
                    "  ANCS Service:     {}",
                    if ancs_attached {
                        format!("attached ({ancs_peripheral})")
                    } else {
                        format!("detached ({ancs_peripheral})")
                    }
                );
                println!("  Bound Ports:");
                println!("    Control (Mesh): {control_port}");
                println!("    File Transfer:  {file_port}");
                println!("    Input (Remote): {input_port}");
                println!("    Screen (Video): {screen_port}");
                println!("    UI (Loopback):  {ui_port_num}");
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
            let (code, resp) = loopback_post(ui_port, "/api/peers/connect", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "node refused connect (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let device_id = v["device_id"].as_str().unwrap_or("?");
                let name = v["alias"]
                    .as_str()
                    .or_else(|| v["name"].as_str())
                    .unwrap_or("peer");
                let addr_str = v["addr"].as_str().unwrap_or("unknown");
                let already = v["already_connected"].as_bool().unwrap_or(false);
                if already {
                    println!("peer {device_id} ({name}) is already connected (addr: {addr_str})");
                } else {
                    println!("successfully connected to {device_id} ({name}) at {addr_str}");
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
            let (code, resp) = loopback_post(ui_port, "/api/peers/disconnect", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "node refused disconnect (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let device_id = v["device_id"].as_str().unwrap_or("?");
                let name = v["alias"]
                    .as_str()
                    .or_else(|| v["name"].as_str())
                    .unwrap_or("peer");
                let already = v["already_disconnected"].as_bool().unwrap_or(false);
                if already {
                    println!("peer {device_id} ({name}) is already disconnected");
                } else {
                    println!("successfully disconnected session with {device_id} ({name})");
                }
            }
        }
        Cmd::Block {
            peer,
            reason,
            json,
            ui_port,
        } => {
            let payload = serde_json::json!({
                "peer": peer,
                "reason": reason,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/peers/block", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "node refused block (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let device_id = v["device_id"].as_str().unwrap_or("?");
                let name = v["alias"]
                    .as_str()
                    .or_else(|| v["name"].as_str())
                    .unwrap_or("peer");
                let reason_str = v["reason"]
                    .as_str()
                    .map(|r| format!(" (reason: \"{r}\")"))
                    .unwrap_or_default();
                let already = v["already_blocked"].as_bool().unwrap_or(false);
                if already {
                    println!("peer {device_id} ({name}) is already blocked{reason_str}");
                } else {
                    let active_str = if v["disconnected_active"].as_bool().unwrap_or(false) {
                        " (active session terminated)"
                    } else {
                        ""
                    };
                    println!(
                        "successfully blocked peer {device_id} ({name}){reason_str}{active_str}"
                    );
                }
            }
        }
        Cmd::Unblock {
            peer,
            json,
            ui_port,
        } => {
            let payload = serde_json::json!({ "peer": peer });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(ui_port, "/api/peers/unblock", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "node refused unblock (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let device_id = v["device_id"].as_str().unwrap_or("?");
                let was_blocked = v["was_blocked"].as_bool().unwrap_or(false);
                if was_blocked {
                    println!("successfully unblocked peer {device_id}");
                } else {
                    println!("peer {device_id} was not in the blocklist");
                }
            }
        }
        Cmd::Blocked { json, ui_port } => {
            let (code, resp) = loopback_get(ui_port, "/api/peers/blocked")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                bail!(
                    "node refused blocked query (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let peers = v["peers"].as_array();
                let count = peers.map(|p| p.len()).unwrap_or(0);
                if count == 0 {
                    println!("no peers currently blocked");
                } else {
                    println!("blocked peers ({count}):");
                    for p in peers.unwrap() {
                        let dev = p["device_id"].as_str().unwrap_or("?");
                        let reason = p["reason"]
                            .as_str()
                            .map(|r| format!(" · reason: \"{r}\""))
                            .unwrap_or_default();
                        println!("  - {dev}{reason}");
                    }
                }
            }
        }
    }
    Ok(())
}

async fn handle_caps_cmd(
    peer: Option<String>,
    enable: Option<String>,
    disable: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
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
        let (code, resp) = loopback_post(
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
            bail!(
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
        let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
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
    let (code_local, resp_local) = loopback_get(ui_port, "/api/caps").await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    let (code_peers, resp_peers) = loopback_get(ui_port, "/api/peers/caps")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code_local != 200 {
        bail!(
            "node returned HTTP {code_local}: {}",
            String::from_utf8_lossy(&resp_local)
        );
    }
    if code_peers != 200 {
        bail!(
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

async fn handle_traffic_cmd(reset: bool, json: bool, ui_port: u16) -> Result<()> {
    if reset {
        let (code, resp) = loopback_post(ui_port, "/api/metrics/reset", b"{}")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            bail!(
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

    let (code, resp) = loopback_get(ui_port, "/api/metrics").await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
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
    println!("    {}", "-".repeat(74));
    println!(
        "    {:<18} {:<16} {:<16} {:<12} {:<12}",
        "Control (45717)",
        format_bytes(ctrl["tx_bytes"].as_u64().unwrap_or(0)),
        format_bytes(ctrl["rx_bytes"].as_u64().unwrap_or(0)),
        ctrl["tx_frames"].as_u64().unwrap_or(0),
        ctrl["rx_frames"].as_u64().unwrap_or(0)
    );
    println!(
        "    {:<18} {:<16} {:<16} {:<12} {:<12}",
        "File (45718)",
        format_bytes(file["tx_bytes"].as_u64().unwrap_or(0)),
        format_bytes(file["rx_bytes"].as_u64().unwrap_or(0)),
        "-",
        "-"
    );
    println!(
        "    {:<18} {:<16} {:<16} {:<12} {:<12}",
        "Input (45719)",
        format_bytes(input["tx_bytes"].as_u64().unwrap_or(0)),
        format_bytes(input["rx_bytes"].as_u64().unwrap_or(0)),
        input["tx_frames"].as_u64().unwrap_or(0),
        input["rx_frames"].as_u64().unwrap_or(0)
    );
    println!(
        "    {:<18} {:<16} {:<16} {:<12} {:<12}",
        "Screen (45720)",
        format_bytes(screen["tx_bytes"].as_u64().unwrap_or(0)),
        format_bytes(screen["rx_bytes"].as_u64().unwrap_or(0)),
        screen["tx_frames"].as_u64().unwrap_or(0),
        screen["rx_frames"].as_u64().unwrap_or(0)
    );

    println!("\n  Protocol Message Telemetry:");
    println!(
        "    Ping Probes:        Sent {:<8} Recv {}",
        msgs["pings_sent"].as_u64().unwrap_or(0),
        msgs["pings_recv"].as_u64().unwrap_or(0)
    );
    println!(
        "    Pong Replies:       Sent {:<8} Recv {}",
        msgs["pongs_sent"].as_u64().unwrap_or(0),
        msgs["pongs_recv"].as_u64().unwrap_or(0)
    );
    println!(
        "    Notifications:      Sent {:<8} Recv {}",
        msgs["notifies_sent"].as_u64().unwrap_or(0),
        msgs["notifies_recv"].as_u64().unwrap_or(0)
    );
    println!(
        "    Clipboard Sync:     Sent {:<8} Recv {}",
        msgs["clips_sent"].as_u64().unwrap_or(0),
        msgs["clips_recv"].as_u64().unwrap_or(0)
    );
    println!(
        "    File Offers:        Sent {:<8} Recv {}",
        msgs["file_offers_sent"].as_u64().unwrap_or(0),
        msgs["file_offers_recv"].as_u64().unwrap_or(0)
    );

    let established = v["sessions_established"].as_u64().unwrap_or(0);
    let closed = v["sessions_closed"].as_u64().unwrap_or(0);
    let handshakes_failed = v["handshakes_failed"].as_u64().unwrap_or(0);
    println!("\n  Session Lifecycle:");
    println!("    Sessions Established: {established}");
    println!("    Sessions Closed:      {closed}");
    println!("    Handshakes Failed:    {handshakes_failed}");

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

fn format_timestamp_ago(secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if secs == 0 {
        return "-".to_string();
    }
    if now >= secs {
        let diff = now - secs;
        if diff < 60 {
            format!("{diff}s ago")
        } else if diff < 3600 {
            format!("{}m ago", diff / 60)
        } else if diff < 86400 {
            format!("{}h ago", diff / 3600)
        } else {
            format!("{}d ago", diff / 86400)
        }
    } else {
        format!("in {}s", secs - now)
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

    let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;

    if code == 404 {
        if let Some(ref target_id) = id {
            bail!("transfer not found: {target_id}");
        } else {
            bail!("endpoint returned 404: {}", String::from_utf8_lossy(&resp));
        }
    }
    if code != 200 {
        bail!(
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

async fn do_file_offers(
    ui_port: u16,
    peer: Option<String>,
    id: Option<String>,
    json: bool,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(ref target_id) = id {
        query_params.push(format!("id={target_id}"));
    }
    if let Some(ref target_peer) = peer {
        query_params.push(format!("peer={target_peer}"));
    }
    let path = if query_params.is_empty() {
        "/api/file/offers".to_string()
    } else {
        format!("/api/file/offers?{}", query_params.join("&"))
    };
    let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code == 404 {
        if let Some(ref target_id) = id {
            bail!("file offer not found: {target_id}");
        } else {
            bail!("endpoint returned 404: {}", String::from_utf8_lossy(&resp));
        }
    }
    if code != 200 {
        bail!(
            "query file offers failed (HTTP {code}): {}",
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
        let source = v.get("source").and_then(|s| s.as_str()).unwrap_or("?");
        let total_bytes = v.get("total_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
        let resume_from = v.get("resume_from").and_then(|s| s.as_u64()).unwrap_or(0);
        let ts = v.get("timestamp_ms").and_then(|s| s.as_u64()).unwrap_or(0);

        println!("File Offer {tid}:");
        println!("  File:        {filename}");
        println!("  Source:      {source}");
        println!(
            "  Total Bytes: {total_bytes} ({})",
            format_bytes(total_bytes)
        );
        println!(
            "  Resume From: {resume_from} ({})",
            format_bytes(resume_from)
        );
        println!("  Offered At:  {ts}ms");
        return Ok(());
    }

    let count = v.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
    let auto_accept = v
        .get("auto_accept_trusted")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    println!("Pending File Offers (count: {count}, auto_accept_trusted: {auto_accept}):");
    if let Some(offers) = v.get("offers").and_then(|o| o.as_array()) {
        if offers.is_empty() {
            println!("  (no pending file offers)");
        } else {
            for o in offers {
                let tid = o.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
                let filename = o.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
                let source = o.get("source").and_then(|s| s.as_str()).unwrap_or("?");
                let total_bytes = o.get("total_bytes").and_then(|s| s.as_u64()).unwrap_or(0);
                let resume_from = o.get("resume_from").and_then(|s| s.as_u64()).unwrap_or(0);
                let resume_str = if resume_from > 0 {
                    format!(" [resuming from {}]", format_bytes(resume_from))
                } else {
                    "".to_string()
                };
                println!(
                    "  - [{tid}] {filename} ({}) from {source}{resume_str}",
                    format_bytes(total_bytes)
                );
            }
        }
    }
    Ok(())
}

async fn do_file_accept(
    ui_port: u16,
    id: Option<String>,
    all: bool,
    peer: Option<String>,
    json: bool,
) -> Result<()> {
    if !all && id.is_none() {
        bail!("specify a transfer ID to accept or use --all");
    }
    let body_json = serde_json::json!({
        "id": id,
        "all": if all { Some(true) } else { None },
        "peer": peer,
    });
    let body = serde_json::to_vec(&body_json)?;
    let (code, resp) = loopback_post(ui_port, "/api/file/accept", &body)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "accept file offer failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if all {
        let count = v
            .get("accepted_count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        let total = v
            .get("total_evaluated")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        println!("Accepted {count} of {total} pending file offer(s)");
        if let Some(decisions) = v.get("decisions").and_then(|d| d.as_array()) {
            for d in decisions {
                let tid = d.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
                let filename = d.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
                let p = d.get("peer").and_then(|s| s.as_str()).unwrap_or("?");
                let bytes = d.get("total_bytes").and_then(|b| b.as_u64()).unwrap_or(0);
                println!(
                    "  * accepted [{tid}] {filename} ({}) from {p}",
                    format_bytes(bytes)
                );
            }
        }
    } else {
        let tid = v.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
        let filename = v.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
        let p = v.get("peer").and_then(|s| s.as_str()).unwrap_or("?");
        let bytes = v.get("total_bytes").and_then(|b| b.as_u64()).unwrap_or(0);
        println!(
            "Accepted file offer [{tid}] {filename} ({}) from {p}",
            format_bytes(bytes)
        );
    }
    Ok(())
}

async fn do_file_reject(
    ui_port: u16,
    id: Option<String>,
    all: bool,
    peer: Option<String>,
    json: bool,
) -> Result<()> {
    if !all && id.is_none() {
        bail!("specify a transfer ID to reject or use --all");
    }
    let body_json = serde_json::json!({
        "id": id,
        "all": if all { Some(true) } else { None },
        "peer": peer,
    });
    let body = serde_json::to_vec(&body_json)?;
    let (code, resp) = loopback_post(ui_port, "/api/file/reject", &body)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "reject file offer failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }
    if all {
        let count = v
            .get("rejected_count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        let total = v
            .get("total_evaluated")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        println!("Rejected {count} of {total} pending file offer(s)");
        if let Some(decisions) = v.get("decisions").and_then(|d| d.as_array()) {
            for d in decisions {
                let tid = d.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
                let filename = d.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
                let p = d.get("peer").and_then(|s| s.as_str()).unwrap_or("?");
                println!("  * rejected [{tid}] {filename} from {p}");
            }
        }
    } else {
        let tid = v.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
        let filename = v.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
        let p = v.get("peer").and_then(|s| s.as_str()).unwrap_or("?");
        println!("Rejected file offer [{tid}] {filename} from {p}");
    }
    Ok(())
}

async fn do_file_auto_accept(ui_port: u16, enable: bool, disable: bool, json: bool) -> Result<()> {
    if enable && disable {
        bail!("cannot specify both --enable and --disable");
    }
    if enable || disable {
        let body_json = serde_json::json!({
            "enabled": enable,
        });
        let body = serde_json::to_vec(&body_json)?;
        let (code, resp) = loopback_post(ui_port, "/api/file/auto-accept", &body)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            bail!(
                "configure auto-accept failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            let enabled = v.get("enabled").and_then(|b| b.as_bool()).unwrap_or(false);
            let count = v
                .get("auto_accepted_count")
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            println!(
                "File auto-accept policy updated: enabled={} (trusted peers only, cumulative auto-accepted: {})",
                enabled, count
            );
        }
        return Ok(());
    }

    let (code, resp) = loopback_get(ui_port, "/api/file/auto-accept")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "query auto-accept status failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        let enabled = v.get("enabled").and_then(|b| b.as_bool()).unwrap_or(false);
        let count = v
            .get("auto_accepted_count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        println!("File Auto-Accept Policy:");
        println!("  Enabled:             {enabled}");
        println!("  Auto-Accepted Count: {count}");
        println!("  Target:              Trusted Noise-authenticated peers only");
    }
    Ok(())
}

async fn print_routes_list(ui_port: u16, json: bool) -> Result<()> {
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
