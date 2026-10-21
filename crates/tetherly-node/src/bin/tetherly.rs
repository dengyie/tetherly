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
        /// this Bluetooth address (Windows MAC `11:22:33:44:55:66` or macOS
        /// CoreBluetooth UUID `9F82A2E6-B9C0-4E90-B8E5-3A21F5A67C43`).
        /// Pair the phone in system Bluetooth settings first. Runtime against a
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
        addr: Option<SocketAddr>,
        /// Peer device ID (`tdev_...`) or advertised hostname to dial via discovery/route table.
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        pin: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Discover available Tetherly peers on the local area network.
    Discover {
        /// Trigger an active rescan across network interfaces before reporting.
        #[arg(long)]
        rescan: bool,
        /// Output raw JSON instead of human-readable table.
        #[arg(long)]
        json: bool,
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
    /// Ask the *running* node to send one or more files to a live peer.
    ///
    /// The file is read from the local filesystem and offered over the live
    /// session. Supports multiple paths for batch sending. With one live peer
    /// the target is implied; with more than one, `--peer` is required.
    Send {
        /// Path(s) to the file(s) to send.
        #[arg(required = true, num_args = 1..)]
        paths: Vec<PathBuf>,
        /// Optional filename override to declare to the peer (single file only).
        #[arg(long)]
        name: Option<String>,
        /// `tdev_…` of the peer. Omit only when exactly one peer is live.
        #[arg(long)]
        peer: Option<String>,
        /// Target peers having the specified tag.
        #[arg(long)]
        tag: Option<String>,
        /// Target peers in the specified group.
        #[arg(long)]
        group: Option<String>,
        /// Target peers of the specified device type (desktop, laptop, phone, tablet, server, cloud).
        #[arg(long, name = "type")]
        device_type: Option<String>,
        /// Broadcast to all online peers.
        #[arg(long)]
        all: bool,
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
        /// Enable auto-accepting files from trusted peers according to policy.
        #[arg(long)]
        enable: bool,
        /// Disable auto-accepting files from trusted peers.
        #[arg(long)]
        disable: bool,
        /// Allowed peer group(s) for auto-accept (e.g. --group dev --group homelab).
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        /// Allowed peer tag(s) for auto-accept (e.g. --tag backup --tag lab).
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        /// Allowed peer device type(s) for auto-accept (e.g. --type desktop --type server).
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        /// Maximum allowed file size in bytes for auto-accept (0 to clear).
        #[arg(long)]
        max_size: Option<u64>,
        /// Clear all group, tag, type and size filters.
        #[arg(long)]
        clear_filters: bool,
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
    /// Cross-device secure resource & URL handoff pipeline (v1.72 / ADR-065).
    Handoff {
        #[command(subcommand)]
        action: HandoffCmd,
    },
    /// Cross-device media playback control & state synchronization (v1.73 / ADR-066).
    Media {
        #[command(subcommand)]
        action: MediaCmd,
    },
    /// Cross-device battery state, power status synchronization, and energy governance (v1.74 / ADR-067).
    #[command(alias = "battery")]
    Power {
        #[command(subcommand)]
        action: PowerCmd,
    },
    /// Cross-device device locator, acoustic ringing, and find-my-device pipeline (v1.75 / ADR-068).
    #[command(alias = "find")]
    Locate {
        #[command(subcommand)]
        action: LocateCmd,
    },
    /// Cross-device predefined quick actions, local allowlist execution, and governance pipeline (v1.76 / ADR-069).
    #[command(alias = "act")]
    Action {
        #[command(subcommand)]
        action: ActionCmd,
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
    /// Peer link quality profiling, keep-alive tuning, and multi-packet ping burst probing.
    Link {
        #[command(subcommand)]
        action: Option<LinkCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Structured security audit trail, circular log inspection and governance.
    Audit {
        #[command(subcommand)]
        action: Option<AuditCmd>,
        /// Category filter: security, peer, file, clipboard, notify, route, config
        #[arg(long, short)]
        category: Option<String>,
        /// Severity minimum threshold filter: info, notice, warning, alert
        #[arg(long, short)]
        severity: Option<String>,
        /// Maximum number of audit entries to return (default: 50)
        #[arg(long, short)]
        limit: Option<usize>,
        /// Filter entries recorded since timestamp in ms
        #[arg(long)]
        since: Option<u64>,
        /// Watch mode: continuously stream live audit events
        #[arg(long, short)]
        watch: bool,
        /// Refresh interval in seconds for --watch (default: 1)
        #[arg(long, default_value_t = 1)]
        interval: u64,
        /// Clear and purge all entries from the audit log
        #[arg(long)]
        clear: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Peer direct secure messaging, ephemeral notes, and chat history governance.
    #[command(alias = "msg")]
    Message {
        #[command(subcommand)]
        action: Option<MessageCmd>,
        /// Target peer device ID (`tdev_...`) or advertised hostname.
        #[arg(long, short)]
        peer: Option<String>,
        /// Target peers having the specified tag.
        #[arg(long)]
        tag: Option<String>,
        /// Target peers in the specified group.
        #[arg(long)]
        group: Option<String>,
        /// Target peers of the specified device type (desktop, laptop, phone, tablet, server, cloud).
        #[arg(long, name = "type")]
        device_type: Option<String>,
        /// Broadcast to all online peers.
        #[arg(long)]
        all: bool,
        /// Text message content (or note) to send (convenience shorthand for `message send`).
        #[arg(long, short)]
        text: Option<String>,
        /// Maximum number of messages to return (default: 50).
        #[arg(long, short)]
        limit: Option<usize>,
        /// Filter only unread messages.
        #[arg(long)]
        unread: bool,
        /// Automatically mark returned unread messages as read.
        #[arg(long)]
        mark_read: bool,
        /// Clear messages matching peer or all messages.
        #[arg(long)]
        clear: bool,
        /// Watch mode: continuously stream live message events.
        #[arg(long, short)]
        watch: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Sent files lifecycle governance, outbox inspection, resend pipeline, and purge closed loop.
    #[command(alias = "out")]
    Outbox {
        #[command(subcommand)]
        action: Option<OutboxCmd>,
        /// Target peer device ID (`tdev_...`) or alias.
        #[arg(long, short)]
        peer: Option<String>,
        /// Filter by transfer status: in_flight, completed, rejected, failed, cancelled.
        #[arg(long, short)]
        status: Option<String>,
        /// Maximum number of outbox entries to return (default: 50).
        #[arg(long, short)]
        limit: Option<usize>,
        /// Clear outbox records matching filter.
        #[arg(long)]
        clear: bool,
        /// One-key resend outbox file by id or transfer_id.
        #[arg(long)]
        resend: Option<String>,
        /// Watch mode: continuously stream live outbox events via SSE.
        #[arg(long, short)]
        watch: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage native desktop notification banners and Do-Not-Disturb (DND) policy.
    Banner {
        #[command(subcommand)]
        action: BannerCmd,
    },
    /// Manage peer device profiles and classification.
    #[command(alias = "prof")]
    Profile {
        #[command(subcommand)]
        action: Option<ProfileCmd>,
        /// Filter by tag when listing profiles
        #[arg(long)]
        tag: Option<String>,
        /// Filter by group when listing profiles
        #[arg(long)]
        group: Option<String>,
        /// Filter by device type when listing profiles
        #[arg(long, name = "type")]
        device_type: Option<String>,
        /// Output raw JSON
        #[arg(long)]
        json: bool,
        /// UI API port of running node
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage peer device multi-tags.
    Tag {
        #[command(subcommand)]
        action: TagCmd,
    },
    /// Manage peer device group association.
    Group {
        #[command(subcommand)]
        action: GroupCmd,
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
    /// Inspect daemon process resources (RSS/Virt memory, threads, FDs, CPU) and host metrics.
    Sys {
        /// Watch mode: refresh system telemetry periodically until interrupted.
        #[arg(long, short)]
        watch: bool,
        /// Refresh interval in seconds for --watch (default: 1).
        #[arg(long, default_value_t = 1)]
        interval: u64,
        /// Output raw JSON system metrics report instead of formatted text.
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
enum HandoffCmd {
    /// Send a URL or web resource to a live peer or target selector with security validation.
    Send {
        /// Target URL to hand off (must pass ResourceHandoffValidator firewall).
        url: String,
        /// Optional human-readable title or page summary.
        #[arg(long)]
        title: Option<String>,
        /// Target peer device_id or alias (`tdev_...`).
        #[arg(long)]
        peer: Option<String>,
        /// Match peers by tag.
        #[arg(long)]
        tag: Option<String>,
        /// Match peers by group.
        #[arg(long)]
        group: Option<String>,
        /// Match peers by device type (desktop, phone, tablet, server).
        #[arg(long, name = "type")]
        device_type: Option<String>,
        /// Target all connected live peers.
        #[arg(long)]
        all: bool,
        /// Request target peer to auto-open the URL (subject to target's handoff policy).
        #[arg(long)]
        auto_open: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// List received resource handoff entries in the inbox.
    List {
        /// Filter by status: pending, opened, dismissed, rejected.
        #[arg(long)]
        status: Option<String>,
        /// Output raw JSON instead of formatted table.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Open a received handoff resource entry by its ID using local system opener.
    Open {
        /// Handoff entry ID (e.g., ho_12345).
        id: String,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Mark a received handoff entry as dismissed/ignored.
    Dismiss {
        /// Handoff entry ID (e.g., ho_12345).
        id: String,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clear all handoff entries from the inbox.
    Clear {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update the handoff ingress policy and auto-open permissions.
    Policy {
        /// Policy subcommand: show or set.
        action: Option<String>,
        /// Ingress action mode: auto_open, prompt_only, save_only, disabled.
        #[arg(long)]
        mode: Option<String>,
        /// Add one or more allowed peer groups for auto-open.
        #[arg(long)]
        group: Option<Vec<String>>,
        /// Add one or more allowed peer tags for auto-open.
        #[arg(long)]
        tag: Option<Vec<String>>,
        /// Add one or more allowed peer device types for auto-open.
        #[arg(long, name = "type")]
        device_type: Option<Vec<String>>,
        /// Maximum inbox entries to retain (default: 100).
        #[arg(long)]
        max_entries: Option<usize>,
        /// Clear all group, tag, and type filters.
        #[arg(long)]
        clear_filters: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch resource handoff events via real-time SSE stream.
    Watch {
        /// Output raw JSON events instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum MediaCmd {
    /// Send play playback command to target peer(s).
    Play {
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, name = "type")]
        device_type: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        player: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Send pause playback command to target peer(s).
    Pause {
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, name = "type")]
        device_type: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        player: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Toggle play/pause playback state on target peer(s).
    #[command(alias = "playpause")]
    Toggle {
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, name = "type")]
        device_type: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        player: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Send stop playback command to target peer(s).
    Stop {
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, name = "type")]
        device_type: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        player: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Skip to next track on target peer(s).
    Next {
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, name = "type")]
        device_type: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        player: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Return to previous track on target peer(s).
    #[command(alias = "previous")]
    Prev {
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, name = "type")]
        device_type: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        player: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Seek forward or backward by an offset in milliseconds.
    Seek {
        /// Offset in milliseconds (positive forward, negative backward).
        offset_ms: i64,
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, name = "type")]
        device_type: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        player: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Set playback volume (0..=100) on target peer(s).
    #[command(alias = "vol")]
    Volume {
        /// Volume percentage (0..=100).
        percent: u8,
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, name = "type")]
        device_type: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        player: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Toggle audio mute on target peer(s).
    Mute {
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, name = "type")]
        device_type: Option<String>,
        #[arg(long)]
        all: bool,
        #[arg(long)]
        player: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Query current playback status (title, artist, album, state, volume) of local player or a remote peer.
    #[command(alias = "state")]
    Status {
        /// Optional peer ID or alias to inspect. When omitted, inspects local player.
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update remote media control ingress policy and volume safety ceilings.
    Policy {
        /// Policy subcommand: show or set.
        action: Option<String>,
        /// Ingress mode: allow, prompt_only, disabled.
        #[arg(long)]
        mode: Option<String>,
        /// Add one or more allowed peer groups.
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        /// Add one or more allowed peer tags.
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        /// Add one or more allowed peer device types.
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        /// Maximum volume percentage limit ceiling (0..=100).
        #[arg(long = "max-volume")]
        max_volume: Option<u8>,
        /// Allow or deny remote seek commands.
        #[arg(long)]
        allow_seek: Option<bool>,
        /// Clear all group, tag, and device type filters.
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch media control commands, playback state changes, and policy events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum PowerCmd {
    /// Query battery and power status (level, charging, source, saver, health) of local device or a remote peer.
    #[command(alias = "state")]
    Status {
        /// Optional peer ID or alias to inspect. When omitted, inspects local device.
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Report or update local battery and power status and broadcast to online peers.
    #[command(alias = "update")]
    Report {
        /// Battery percentage level (0..=100).
        #[arg(long, alias = "battery-level")]
        level: Option<u8>,
        /// Device is currently plugged in / charging.
        #[arg(long)]
        charging: Option<bool>,
        /// Power source type: battery, ac, wireless, unknown.
        #[arg(long = "source", alias = "power-source")]
        power_source: Option<String>,
        /// Device is currently in power saver / low-power mode.
        #[arg(long = "saver", alias = "battery-saver")]
        battery_saver: Option<bool>,
        /// Battery health percentage (0..=100).
        #[arg(long = "health", alias = "health-percent")]
        health_percent: Option<u8>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update battery monitoring policy, alert thresholds, and scoped peer filters.
    Policy {
        /// Policy subcommand: show or set.
        action: Option<String>,
        /// Low battery warning threshold percentage (default: 20%).
        #[arg(long = "low-threshold", alias = "low")]
        low_threshold: Option<u8>,
        /// Critical battery alert threshold percentage (default: 10%).
        #[arg(long = "critical-threshold", alias = "critical")]
        critical_threshold: Option<u8>,
        /// Enable or disable low battery alert events.
        #[arg(long)]
        alert: Option<bool>,
        /// Enable or disable automatic throughput throttling when peer is in battery saver mode.
        #[arg(long)]
        throttle: Option<bool>,
        /// Add one or more allowed peer groups.
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        /// Add one or more allowed peer tags.
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        /// Add one or more allowed peer device types.
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        /// Clear all group, tag, and device type filters.
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch battery state changes and low battery alert events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum LocateCmd {
    /// Ring/locate a remote peer device with acoustic alarm or vibration.
    #[command(alias = "alarm")]
    Ring {
        /// Peer ID or alias to ring / locate.
        #[arg(long, alias = "target")]
        peer: String,
        /// Ring duration in seconds (1..=60, default: 15).
        #[arg(long = "duration", alias = "duration-secs")]
        duration_secs: Option<u32>,
        /// Acoustic alarm pattern: chime, siren, pulse, vibrate_only.
        #[arg(long)]
        pattern: Option<String>,
        /// Override target device silent / DND mode (default: true).
        #[arg(long = "override-silent", alias = "override")]
        override_silent: Option<bool>,
        /// Optional display message (e.g. "Looking for keys", max 128 chars).
        #[arg(long)]
        message: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stop active acoustic ringing on target peer or locally.
    #[command(alias = "cancel")]
    Stop {
        /// Optional peer ID or alias. When omitted or target is local, stops local ringing.
        #[arg(long, alias = "target")]
        peer: Option<String>,
        /// Optional locate request ID to stop.
        #[arg(long = "id", alias = "request-id")]
        request_id: Option<String>,
        /// Optional reason for stopping (e.g. "found", "user_cancelled").
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Query device locator status (whether currently ringing, active ring details, policy).
    #[command(alias = "state")]
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update device locator policy, silent override permissions, duration limits, and scoped peer filters.
    Policy {
        /// Policy subcommand: show or set.
        action: Option<String>,
        /// Enable or disable device locator functionality on this node.
        #[arg(long)]
        enabled: Option<bool>,
        /// Allow or deny remote locate requests to override silent / DND mode.
        #[arg(long = "override-silent", alias = "allow-override-silent")]
        override_silent: Option<bool>,
        /// Maximum ringing duration in seconds allowed on this node (1..=60).
        #[arg(long = "max-duration", alias = "max-duration-secs")]
        max_duration: Option<u32>,
        /// Add one or more allowed peer groups.
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        /// Add one or more allowed peer tags.
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        /// Add one or more allowed peer device types.
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        /// Clear all group, tag, and device type filters.
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch device locate events (ring triggered, stopped, policy changed) via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum ActionCmd {
    /// Manage predefined local actions on this host (ADR-008 allowlist).
    Local {
        #[command(subcommand)]
        sub: ActionLocalSubCmd,
    },
    /// List advertised safe action metadata from a remote peer.
    Remote {
        /// Target peer ID or alias.
        #[arg(long)]
        peer: String,
        /// Force an active remote query over mesh to refresh advertised actions.
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Trigger execution of a predefined quick action on a remote target peer.
    Run {
        /// Target peer ID or alias.
        #[arg(long)]
        peer: String,
        /// Action identifier to trigger on target (must match target's registered allowlist).
        #[arg(long = "id", alias = "action")]
        action_id: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// View recent inbound and outbound action execution history.
    History {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update action governance policy, access mode, timeout bounds, and peer filters.
    Policy {
        /// Policy subcommand: show or set.
        action: Option<String>,
        /// Access mode: Allow, PromptOnly, or Disabled.
        #[arg(long)]
        mode: Option<String>,
        /// Maximum allowed execution timeout in seconds (1..=30).
        #[arg(long = "timeout", alias = "max-timeout")]
        max_timeout_secs: Option<u32>,
        /// Allow advertising safe action metadata list to eligible peers.
        #[arg(long)]
        expose: Option<bool>,
        /// Add one or more allowed peer groups.
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        /// Add one or more allowed peer tags.
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        /// Add one or more allowed peer device types.
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        /// Clear all group, tag, and device type filters.
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch action events (requested, received, triggered, result) via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum ActionLocalSubCmd {
    /// List all registered local action definitions.
    List {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Register or update a predefined local action.
    Add {
        /// Unique action identifier (e.g., lock_screen, sleep_display).
        #[arg(long)]
        id: String,
        /// Human-readable action name.
        #[arg(long)]
        name: String,
        /// Executable command on local host.
        #[arg(long)]
        cmd: String,
        /// Optional command arguments (comma-separated or multiple flags).
        #[arg(long = "arg")]
        args: Option<Vec<String>>,
        /// Action description.
        #[arg(long)]
        desc: Option<String>,
        /// Optional icon identifier or emoji.
        #[arg(long)]
        icon: Option<String>,
        /// Maximum execution timeout in seconds (1..=30).
        #[arg(long, default_value_t = 10)]
        timeout: u32,
        /// Whether the action is active and triggerable.
        #[arg(long, default_value_t = true)]
        enabled: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Remove a registered local action by identifier.
    Remove {
        /// Action identifier to remove.
        #[arg(long)]
        id: String,
        #[arg(long)]
        json: bool,
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
    /// Manage clipboard automatic synchronization policy, sensitive leak guard, and distribution scope.
    Policy {
        /// Optional action: "show" or "set" (default: "show").
        action: Option<String>,
        /// Set operating mode (manual, bidirectional_auto, send_only, receive_only).
        #[arg(long)]
        mode: Option<String>,
        /// Enable sensitive content leak guard (blocks private keys, tokens, credentials).
        #[arg(long, conflicts_with = "no_sensitive_guard")]
        sensitive_guard: bool,
        /// Disable sensitive content leak guard.
        #[arg(long)]
        no_sensitive_guard: bool,
        /// Allowed peer group(s) for clipboard sync (e.g. --group dev).
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        /// Allowed peer tag(s) for clipboard sync (e.g. --tag workstation).
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        /// Allowed peer device type(s) for clipboard sync (e.g. --type desktop --type laptop).
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        /// Maximum allowed payload size in bytes for auto-sync (e.g. 65536).
        #[arg(long = "max-bytes")]
        max_bytes: Option<usize>,
        /// Optional timeout in seconds to auto-clear clipboard (0 to disable).
        #[arg(long = "auto-clear")]
        auto_clear: Option<u64>,
        /// Clear all group, tag, and type filters.
        #[arg(long)]
        clear_filters: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch clipboard events (synced, blocked, policy changed) via real-time SSE stream.
    Watch {
        /// Output raw JSON events instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Trigger an immediate evaluation and poll of clipboard auto-synchronization.
    Poll {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
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
    /// Extract / download a completed file from the incoming inbox to a local destination.
    Get {
        /// Filename to extract (basename inside inbox).
        name: String,
        /// Destination local file path (defaults to current directory with same filename).
        #[arg(long, short)]
        dest: Option<PathBuf>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Open a file in the system default application, or reveal the inbox directory if omitted.
    Open {
        /// Filename to open (basename inside inbox; omit to open inbox directory).
        name: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum LinkCmd {
    /// Show current link quality metrics for all connected peers (default).
    Status {
        /// Watch mode: continuously refresh link quality until interrupted.
        #[arg(long, short)]
        watch: bool,
        /// Refresh interval in seconds for --watch (default: 1).
        #[arg(long, default_value_t = 1)]
        interval: u64,
        /// Output raw JSON instead of human-readable tables.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Perform an on-demand multi-packet ping burst probe to measure latency, jitter and loss.
    Probe {
        /// Specific peer to probe (e.g. tdev_... or alias). Defaults to single connected peer.
        #[arg(long)]
        peer: Option<String>,
        /// Number of ping probes to send (1 to 20, default: 5).
        #[arg(long, default_value_t = 5)]
        count: usize,
        /// Pacing interval between probes in milliseconds (10 to 1000, default: 100).
        #[arg(long, default_value_t = 100)]
        interval_ms: u64,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update session keep-alive heartbeat interval and silent timeout deadline.
    Config {
        /// Update heartbeat ping interval in seconds (1 to 3600).
        #[arg(long)]
        interval: Option<u64>,
        /// Update silent timeout deadline in seconds (interval to 86400).
        #[arg(long)]
        deadline: Option<u64>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum AuditCmd {
    /// List and filter recorded security audit log entries (default).
    List {
        /// Category filter: security, peer, file, clipboard, notify, route, config
        #[arg(long, short)]
        category: Option<String>,
        /// Severity minimum threshold filter: info, notice, warning, alert
        #[arg(long, short)]
        severity: Option<String>,
        /// Maximum number of audit entries to return (default: 50)
        #[arg(long, short)]
        limit: Option<usize>,
        /// Filter entries recorded since timestamp in ms
        #[arg(long)]
        since: Option<u64>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clear and purge all entries from the in-memory audit ring buffer.
    Clear {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stream live security audit events in real time via Server-Sent Events (SSE).
    Watch {
        /// Category filter: security, peer, file, clipboard, notify, route, config
        #[arg(long, short)]
        category: Option<String>,
        /// Severity minimum threshold filter: info, notice, warning, alert
        #[arg(long, short)]
        severity: Option<String>,
        /// Maximum number of audit events to receive before exiting (0 for unlimited).
        #[arg(long, default_value_t = 0)]
        count: usize,
        /// Output raw JSON per event line instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum MessageCmd {
    /// Send a direct secure text message to a connected peer or target group/tag.
    Send {
        /// Target peer device ID (`tdev_...`) or advertised hostname. Omit if exactly one peer is live.
        #[arg(long, short)]
        peer: Option<String>,
        /// Target peers having the specified tag.
        #[arg(long)]
        tag: Option<String>,
        /// Target peers in the specified group.
        #[arg(long)]
        group: Option<String>,
        /// Target peers of the specified device type (desktop, laptop, phone, tablet, server, cloud).
        #[arg(long, name = "type")]
        device_type: Option<String>,
        /// Broadcast to all online peers.
        #[arg(long)]
        all: bool,
        /// Text message content (or note) to send.
        #[arg(required = true)]
        text: String,
        /// Output raw JSON response instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// List and query direct message history.
    #[command(alias = "list")]
    History {
        /// Filter by peer device ID (`tdev_...`) or advertised hostname.
        #[arg(long, short)]
        peer: Option<String>,
        /// Maximum number of messages to return (default: 50).
        #[arg(long, short)]
        limit: Option<usize>,
        /// Filter only unread messages.
        #[arg(long)]
        unread: bool,
        /// Automatically mark returned unread messages as read.
        #[arg(long)]
        mark_read: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clear and purge message history from the in-memory ring buffer.
    Clear {
        /// Filter by peer to clear only messages with that peer, or omit to clear all.
        #[arg(long, short)]
        peer: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Watch live direct message events in real time via Server-Sent Events (SSE).
    Watch {
        /// Filter by peer device ID or hostname.
        #[arg(long, short)]
        peer: Option<String>,
        /// Maximum number of events to receive before exiting (0 for unlimited).
        #[arg(long, default_value_t = 0)]
        count: usize,
        /// Output raw JSON per event line instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum OutboxCmd {
    /// List outbox records with optional peer, status and limit filtering.
    #[command(alias = "ls")]
    List {
        /// Target peer device ID (`tdev_...`) or alias.
        #[arg(long, short)]
        peer: Option<String>,
        /// Filter by transfer status: in_flight, completed, rejected, failed, cancelled.
        #[arg(long, short)]
        status: Option<String>,
        /// Maximum number of outbox entries to return (default: 50).
        #[arg(long, short)]
        limit: Option<usize>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clear and purge outbox transmission records.
    Clear {
        /// Target peer filter to selectively clear.
        #[arg(long, short)]
        peer: Option<String>,
        /// Status filter to selectively clear.
        #[arg(long, short)]
        status: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// One-key resend and re-offer an outbox file.
    Resend {
        /// Outbox entry ID (`out_tr_...`) or transfer ID (`tr_...`).
        id: String,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stream real-time outbox events over SSE.
    Watch {
        /// Target peer filter for events.
        #[arg(long, short)]
        peer: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum BannerCmd {
    /// Query or update desktop notification banner settings and DND mode.
    Config {
        /// Enable or disable desktop notification banners.
        #[arg(long)]
        banners: Option<bool>,
        /// Set Do-Not-Disturb mode (`off`, `silent`, or `all`).
        #[arg(long)]
        dnd: Option<String>,
        /// Enable or disable notification sounds.
        #[arg(long)]
        sound: Option<bool>,
        /// Enable or disable masking OTP codes on desktop notification banners.
        #[arg(long)]
        mask_otp: Option<bool>,
        /// Allow high-urgency OTP notifications even in DND mode.
        #[arg(long)]
        allow_otp_dnd: Option<bool>,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Send a test notification banner to the local desktop.
    Test {
        /// Optional custom notification title.
        #[arg(long)]
        title: Option<String>,
        /// Optional custom notification body text.
        #[arg(long, short)]
        body: Option<String>,
        /// Output in JSON format.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum ProfileCmd {
    /// List peer device profiles with optional tag/group/type filters.
    #[command(alias = "ls")]
    List {
        #[arg(long, short)]
        tag: Option<String>,
        #[arg(long, short)]
        group: Option<String>,
        #[arg(long, name = "type")]
        device_type: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Retrieve detailed profile metadata for a peer.
    Get {
        /// Target peer (device_id, alias, or name)
        peer: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Update peer device profile attributes.
    Set {
        /// Target peer (device_id, alias, or name)
        peer: String,
        /// Device type (desktop, laptop, phone, tablet, server, cloud, other)
        #[arg(long, name = "type")]
        device_type: Option<String>,
        /// Organization group name (or empty string to clear)
        #[arg(long)]
        group: Option<String>,
        /// Private notes (stored securely, not broadcast over SSE)
        #[arg(long)]
        notes: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum TagCmd {
    /// Add a tag to a peer device.
    Add {
        /// Target peer (device_id, alias, or name)
        peer: String,
        /// Tag to add (1-32 chars, [a-zA-Z0-9_-])
        tag: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Remove a tag from a peer device.
    #[command(alias = "rm")]
    Remove {
        /// Target peer (device_id, alias, or name)
        peer: String,
        /// Tag to remove
        tag: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
enum GroupCmd {
    /// Assign a peer device to a group.
    Set {
        /// Target peer (device_id, alias, or name)
        peer: String,
        /// Group name
        group: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clear a peer device's group assignment.
    Clear {
        /// Target peer (device_id, alias, or name)
        peer: String,
        #[arg(long)]
        json: bool,
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
                    Ok(Some(link)) => Some((address.to_string(), link)),
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
            let (ancs_addr, inbound) = match ancs_link {
                Some((addr, (transport, inbound))) => {
                    cfg.ancs = Some(transport);
                    cfg.ancs_peripheral = addr.clone();
                    (Some(addr), Some(inbound))
                }
                None => (None, None),
            };
            let node = Node::start(cfg)?;
            info!(device_id = %node.identity().device_id(), "run");
            info!("ui http://127.0.0.1:45716");
            if let Some(inbound) = inbound {
                node.spawn_ancs_pump(inbound);
                info!(
                    peripheral = ?ancs_addr,
                    "ANCS link up; pair the iPhone in system Bluetooth settings if you have not"
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
        Cmd::Dial {
            addr,
            peer,
            pin,
            ui_port,
        } => {
            if addr.is_none() && peer.is_none() {
                anyhow::bail!("either --addr <ip:port> or --peer <name|id> must be specified");
            }
            let payload = serde_json::json!({
                "addr": addr.map(|a| a.to_string()),
                "peer": peer,
                "pin": pin,
            });
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
            let target_desc = addr
                .map(|a| a.to_string())
                .or_else(|| peer.clone())
                .unwrap_or_default();
            println!("dialed {target_desc}");
        }
        Cmd::Discover {
            rescan,
            json,
            ui_port,
        } => {
            if rescan {
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/discovery/rescan", b"{}")
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused discovery rescan (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
            }
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/discovery")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node returned HTTP {code} on /api/discovery: {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
                return Ok(());
            }
            let rep: tetherly_node::DiscoveryReport = serde_json::from_slice(&resp)?;
            println!("Discovered LAN Peers ({}):", rep.count);
            if rep.peers.is_empty() {
                println!("  (no peers discovered on local network)");
            } else {
                for p in rep.peers {
                    let trusted_mark = if p.is_trusted {
                        "[trusted]"
                    } else {
                        "[unpaired]"
                    };
                    let online_mark = if p.is_online { "[online]" } else { "[idle]" };
                    let fp_str = p.fp.as_deref().unwrap_or("none");
                    let addrs_str = p.addrs.join(", ");
                    println!(
                        "  * {} ({}) {} {} - port: {}, fp: {}, caps: [{}], addrs: [{}]",
                        p.name,
                        p.device_id,
                        trusted_mark,
                        online_mark,
                        p.port,
                        fp_str,
                        p.caps.join(", "),
                        addrs_str
                    );
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
            paths,
            name,
            peer,
            tag,
            group,
            device_type,
            all,
            json,
            watch,
            ui_port,
        } => {
            if paths.is_empty() {
                anyhow::bail!("no file paths provided");
            }
            let is_multi = all || tag.is_some() || group.is_some() || device_type.is_some();
            if is_multi {
                for path in &paths {
                    let path_str = path
                        .to_str()
                        .ok_or_else(|| anyhow::anyhow!("path contains invalid UTF-8"))?;
                    let payload = serde_json::json!({
                        "path": path_str,
                        "name": name,
                        "peer": peer,
                        "tag": tag,
                        "group": group,
                        "type": device_type,
                        "all": all,
                    });
                    let bytes = serde_json::to_vec(&payload)?;
                    let (code, resp) =
                        tetherly_node::loopback_post(ui_port, "/api/file/send-multi", &bytes)
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!(
                                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                                )
                            })?;
                    if code != 200 {
                        anyhow::bail!(
                            "node refused targeted file send (HTTP {code}): {}",
                            String::from_utf8_lossy(&resp)
                        );
                    }
                    if json {
                        println!("{}", String::from_utf8_lossy(&resp).trim());
                    } else {
                        let rep: serde_json::Value = serde_json::from_slice(&resp)
                            .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                        let fname = rep["filename"].as_str().unwrap_or("file");
                        let fsize = rep["size"].as_u64().unwrap_or(0);
                        let fhash = rep["sha256"].as_str().unwrap_or("");
                        let short_hash = if fhash.len() > 12 {
                            &fhash[..12]
                        } else {
                            fhash
                        };
                        let selector = rep["selector"].as_str().unwrap_or("target");
                        let total = rep["target_count"].as_u64().unwrap_or(0);
                        let succ = rep["success_count"].as_u64().unwrap_or(0);
                        let skipped = rep["skipped_offline_count"].as_u64().unwrap_or(0);
                        let failed = rep["failed_count"].as_u64().unwrap_or(0);
                        println!(
                            "Multi-Dispatch File [{fname}] ({}, sha256:{short_hash}...) to [{selector}]: {succ}/{total} offered, {skipped} skipped offline, {failed} failed",
                            format_bytes(fsize)
                        );
                        if let Some(dispatches) = rep["dispatches"].as_array() {
                            if !dispatches.is_empty() {
                                println!(
                                    "  {:<20} {:<16} {:<10} {:<20} {:<20} {:<20}",
                                    "TARGET",
                                    "ALIAS",
                                    "STATUS",
                                    "TRANSFER ID",
                                    "OUTBOX ID",
                                    "ERROR"
                                );
                                println!("  {}", "-".repeat(108));
                                for d in dispatches {
                                    let dev = d["peer"].as_str().unwrap_or("?");
                                    let dev_short = if dev.len() > 18 { &dev[..18] } else { dev };
                                    let alias = d["peer_alias"].as_str().unwrap_or("-");
                                    let alias_short = if alias.len() > 14 {
                                        &alias[..14]
                                    } else {
                                        alias
                                    };
                                    let status = if d["success"].as_bool().unwrap_or(false) {
                                        "offered"
                                    } else {
                                        "failed"
                                    };
                                    let tid = d["transfer_id"].as_str().unwrap_or("-");
                                    let tid_short = if tid.len() > 18 { &tid[..18] } else { tid };
                                    let obid = d["outbox_id"].as_str().unwrap_or("-");
                                    let obid_short =
                                        if obid.len() > 18 { &obid[..18] } else { obid };
                                    let err = d["error"].as_str().unwrap_or("-");
                                    let err_short = if err.len() > 18 { &err[..18] } else { err };
                                    println!(
                                        "  {:<20} {:<16} {:<10} {:<20} {:<20} {:<20}",
                                        dev_short,
                                        alias_short,
                                        status,
                                        tid_short,
                                        obid_short,
                                        err_short
                                    );
                                }
                            }
                        }
                    }
                }
                return Ok(());
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
                                anyhow::bail!("\n✗ Transfer failed: {err_msg}");
                            }
                        }
                    }
                }
            } else {
                if name.is_some() {
                    anyhow::bail!(
                        "--name cannot be specified when sending multiple files in batch"
                    );
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
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/file/send-batch", &bytes)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused batch file send (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                    return Ok(());
                }
                let rep: tetherly_node::BatchSendReport = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let target = rep.peer_alias.as_deref().unwrap_or(&rep.peer);
                println!(
                    "Batch [{}] to {}: {}/{} succeeded, {} failed ({} total)",
                    rep.batch_id,
                    target,
                    rep.successful_files,
                    rep.total_files,
                    rep.failed_files,
                    format_bytes(rep.total_bytes)
                );
                for item in rep.items {
                    if item.status == "sent" || item.status == "queued" {
                        let tid = item.transfer_id.as_deref().unwrap_or("?");
                        println!("  ✓ {} ({}) -> [{tid}]", item.name, format_bytes(item.size));
                    } else {
                        let err = item.error.as_deref().unwrap_or("unknown error");
                        println!("  ✗ {} -> failed: {err}", item.name);
                    }
                }
            }
        }
        Cmd::Resume { json, ui_port } => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/file/resume")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node returned HTTP {code} querying resume markers: {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp));
                return Ok(());
            }
            let rep: tetherly_node::ResumeMarkersReport =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            if rep.markers.is_empty() {
                println!("No resumable transfer markers found in inbox.");
                return Ok(());
            }
            println!(
                "Found {} resumable marker(s) ({} total partial data):",
                rep.count,
                format_bytes(rep.total_partial_bytes)
            );
            println!(
                "{:<20} {:<24} {:<22} {:<10} {:<10} {:<10}",
                "TRANSFER ID", "SOURCE", "OFFSET / TOTAL", "PROGRESS", "ALIGNED", ".PART FILE"
            );
            for m in rep.markers {
                let src = m.source_alias.as_deref().unwrap_or(&m.source);
                let off_tot = format!(
                    "{} / {}",
                    format_bytes(m.offset_bytes),
                    format_bytes(m.total_bytes)
                );
                let prog = format!("{:.1}%", m.progress_percent);
                let align = if m.chunk_aligned { "yes" } else { "no" };
                let part = if m.part_file_exists {
                    format!("yes ({})", format_bytes(m.part_file_bytes))
                } else {
                    "no".to_string()
                };
                println!(
                    "{:<20} {:<24} {:<22} {:<10} {:<10} {:<10}",
                    m.transfer_id, src, off_tot, prog, align, part
                );
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
            group,
            tag,
            device_type,
            max_size,
            clear_filters,
            json,
            ui_port,
        } => {
            do_file_auto_accept(
                ui_port,
                enable,
                disable,
                group,
                tag,
                device_type,
                max_size,
                clear_filters,
                json,
            )
            .await?;
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
            ClipCmd::Status { json, ui_port } => {
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/clip/status")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
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
                let (code, resp) =
                    tetherly_node::loopback_get(ui_port, &path)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
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
                let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/clip/clear", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
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
            ClipCmd::Policy {
                action,
                mode,
                sensitive_guard,
                no_sensitive_guard,
                group,
                tag,
                device_type,
                max_bytes,
                auto_clear,
                clear_filters,
                json,
                ui_port,
            } => {
                do_clip_policy(
                    ui_port,
                    action,
                    mode,
                    sensitive_guard,
                    no_sensitive_guard,
                    group,
                    tag,
                    device_type,
                    max_bytes,
                    auto_clear,
                    clear_filters,
                    json,
                )
                .await?;
            }
            ClipCmd::Watch { json, ui_port } => {
                do_clip_watch(ui_port, json).await?;
            }
            ClipCmd::Poll { json, ui_port } => {
                do_clip_poll(ui_port, json).await?;
            }
        },
        Cmd::Handoff { action } => match action {
            HandoffCmd::Send {
                url,
                title,
                peer,
                tag,
                group,
                device_type,
                all,
                auto_open,
                json,
                ui_port,
            } => {
                do_handoff_send(
                    ui_port,
                    &url,
                    title.as_deref(),
                    peer.as_deref(),
                    tag.as_deref(),
                    group.as_deref(),
                    device_type.as_deref(),
                    all,
                    auto_open,
                    json,
                )
                .await?;
            }
            HandoffCmd::List {
                status,
                json,
                ui_port,
            } => {
                do_handoff_list(ui_port, status.as_deref(), json).await?;
            }
            HandoffCmd::Open { id, json, ui_port } => {
                do_handoff_open(ui_port, &id, json).await?;
            }
            HandoffCmd::Dismiss { id, json, ui_port } => {
                do_handoff_dismiss(ui_port, &id, json).await?;
            }
            HandoffCmd::Clear { json, ui_port } => {
                do_handoff_clear(ui_port, json).await?;
            }
            HandoffCmd::Policy {
                action,
                mode,
                group,
                tag,
                device_type,
                max_entries,
                clear_filters,
                json,
                ui_port,
            } => {
                do_handoff_policy(
                    ui_port,
                    action,
                    mode,
                    group,
                    tag,
                    device_type,
                    max_entries,
                    clear_filters,
                    json,
                )
                .await?;
            }
            HandoffCmd::Watch { json, ui_port } => {
                do_handoff_watch(ui_port, json).await?;
            }
        },
        Cmd::Media { action } => match action {
            MediaCmd::Play {
                peer,
                tag,
                group,
                device_type,
                all,
                player,
                json,
                ui_port,
            } => {
                do_media_command(
                    ui_port,
                    "play",
                    None,
                    None,
                    peer.as_deref(),
                    tag.as_deref(),
                    group.as_deref(),
                    device_type.as_deref(),
                    all,
                    player.as_deref(),
                    json,
                )
                .await?;
            }
            MediaCmd::Pause {
                peer,
                tag,
                group,
                device_type,
                all,
                player,
                json,
                ui_port,
            } => {
                do_media_command(
                    ui_port,
                    "pause",
                    None,
                    None,
                    peer.as_deref(),
                    tag.as_deref(),
                    group.as_deref(),
                    device_type.as_deref(),
                    all,
                    player.as_deref(),
                    json,
                )
                .await?;
            }
            MediaCmd::Toggle {
                peer,
                tag,
                group,
                device_type,
                all,
                player,
                json,
                ui_port,
            } => {
                do_media_command(
                    ui_port,
                    "toggle",
                    None,
                    None,
                    peer.as_deref(),
                    tag.as_deref(),
                    group.as_deref(),
                    device_type.as_deref(),
                    all,
                    player.as_deref(),
                    json,
                )
                .await?;
            }
            MediaCmd::Stop {
                peer,
                tag,
                group,
                device_type,
                all,
                player,
                json,
                ui_port,
            } => {
                do_media_command(
                    ui_port,
                    "stop",
                    None,
                    None,
                    peer.as_deref(),
                    tag.as_deref(),
                    group.as_deref(),
                    device_type.as_deref(),
                    all,
                    player.as_deref(),
                    json,
                )
                .await?;
            }
            MediaCmd::Next {
                peer,
                tag,
                group,
                device_type,
                all,
                player,
                json,
                ui_port,
            } => {
                do_media_command(
                    ui_port,
                    "next",
                    None,
                    None,
                    peer.as_deref(),
                    tag.as_deref(),
                    group.as_deref(),
                    device_type.as_deref(),
                    all,
                    player.as_deref(),
                    json,
                )
                .await?;
            }
            MediaCmd::Prev {
                peer,
                tag,
                group,
                device_type,
                all,
                player,
                json,
                ui_port,
            } => {
                do_media_command(
                    ui_port,
                    "prev",
                    None,
                    None,
                    peer.as_deref(),
                    tag.as_deref(),
                    group.as_deref(),
                    device_type.as_deref(),
                    all,
                    player.as_deref(),
                    json,
                )
                .await?;
            }
            MediaCmd::Seek {
                offset_ms,
                peer,
                tag,
                group,
                device_type,
                all,
                player,
                json,
                ui_port,
            } => {
                do_media_command(
                    ui_port,
                    "seek",
                    None,
                    Some(offset_ms),
                    peer.as_deref(),
                    tag.as_deref(),
                    group.as_deref(),
                    device_type.as_deref(),
                    all,
                    player.as_deref(),
                    json,
                )
                .await?;
            }
            MediaCmd::Volume {
                percent,
                peer,
                tag,
                group,
                device_type,
                all,
                player,
                json,
                ui_port,
            } => {
                do_media_command(
                    ui_port,
                    "volume",
                    Some(percent),
                    None,
                    peer.as_deref(),
                    tag.as_deref(),
                    group.as_deref(),
                    device_type.as_deref(),
                    all,
                    player.as_deref(),
                    json,
                )
                .await?;
            }
            MediaCmd::Mute {
                peer,
                tag,
                group,
                device_type,
                all,
                player,
                json,
                ui_port,
            } => {
                do_media_command(
                    ui_port,
                    "mute",
                    None,
                    None,
                    peer.as_deref(),
                    tag.as_deref(),
                    group.as_deref(),
                    device_type.as_deref(),
                    all,
                    player.as_deref(),
                    json,
                )
                .await?;
            }
            MediaCmd::Status {
                peer,
                json,
                ui_port,
            } => {
                do_media_status(ui_port, peer.as_deref(), json).await?;
            }
            MediaCmd::Policy {
                action,
                mode,
                group,
                tag,
                device_type,
                max_volume,
                allow_seek,
                clear_filters,
                json,
                ui_port,
            } => {
                do_media_policy(
                    ui_port,
                    action,
                    mode,
                    group,
                    tag,
                    device_type,
                    max_volume,
                    allow_seek,
                    clear_filters,
                    json,
                )
                .await?;
            }
            MediaCmd::Watch { json, ui_port } => {
                do_media_watch(ui_port, json).await?;
            }
        },
        Cmd::Power { action } => match action {
            PowerCmd::Status {
                peer,
                json,
                ui_port,
            } => {
                do_power_status(ui_port, peer.as_deref(), json).await?;
            }
            PowerCmd::Report {
                level,
                charging,
                power_source,
                battery_saver,
                health_percent,
                json,
                ui_port,
            } => {
                do_power_report(
                    ui_port,
                    level,
                    charging,
                    power_source.as_deref(),
                    battery_saver,
                    health_percent,
                    json,
                )
                .await?;
            }
            PowerCmd::Policy {
                action,
                low_threshold,
                critical_threshold,
                alert,
                throttle,
                group,
                tag,
                device_type,
                clear_filters,
                json,
                ui_port,
            } => {
                do_power_policy(
                    ui_port,
                    action,
                    low_threshold,
                    critical_threshold,
                    alert,
                    throttle,
                    group,
                    tag,
                    device_type,
                    clear_filters,
                    json,
                )
                .await?;
            }
            PowerCmd::Watch { json, ui_port } => {
                do_power_watch(ui_port, json).await?;
            }
        },
        Cmd::Locate { action } => match action {
            LocateCmd::Ring {
                peer,
                duration_secs,
                pattern,
                override_silent,
                message,
                json,
                ui_port,
            } => {
                do_locate_ring(
                    ui_port,
                    &peer,
                    duration_secs,
                    pattern.as_deref(),
                    override_silent,
                    message.as_deref(),
                    json,
                )
                .await?;
            }
            LocateCmd::Stop {
                peer,
                request_id,
                reason,
                json,
                ui_port,
            } => {
                do_locate_stop(
                    ui_port,
                    peer.as_deref(),
                    request_id.as_deref(),
                    reason.as_deref(),
                    json,
                )
                .await?;
            }
            LocateCmd::Status { json, ui_port } => {
                do_locate_status(ui_port, json).await?;
            }
            LocateCmd::Policy {
                action,
                enabled,
                override_silent,
                max_duration,
                group,
                tag,
                device_type,
                clear_filters,
                json,
                ui_port,
            } => {
                do_locate_policy(
                    ui_port,
                    action,
                    enabled,
                    override_silent,
                    max_duration,
                    group,
                    tag,
                    device_type,
                    clear_filters,
                    json,
                )
                .await?;
            }
            LocateCmd::Watch { json, ui_port } => {
                do_locate_watch(ui_port, json).await?;
            }
        },
        Cmd::Action { action } => match action {
            ActionCmd::Local { sub } => match sub {
                ActionLocalSubCmd::List { json, ui_port } => {
                    do_action_local_list(ui_port, json).await?;
                }
                ActionLocalSubCmd::Add {
                    id,
                    name,
                    cmd,
                    args,
                    desc,
                    icon,
                    timeout,
                    enabled,
                    json,
                    ui_port,
                } => {
                    do_action_local_add(
                        ui_port,
                        &id,
                        &name,
                        &cmd,
                        args,
                        desc.as_deref(),
                        icon.as_deref(),
                        Some(timeout),
                        Some(enabled),
                        json,
                    )
                    .await?;
                }
                ActionLocalSubCmd::Remove { id, json, ui_port } => {
                    do_action_local_remove(ui_port, &id, json).await?;
                }
            },
            ActionCmd::Remote {
                peer,
                refresh,
                json,
                ui_port,
            } => {
                do_action_remote_list(ui_port, &peer, refresh, json).await?;
            }
            ActionCmd::Run {
                peer,
                action_id,
                json,
                ui_port,
            } => {
                do_action_run(ui_port, &peer, &action_id, json).await?;
            }
            ActionCmd::History { json, ui_port } => {
                do_action_history(ui_port, json).await?;
            }
            ActionCmd::Policy {
                action,
                mode,
                max_timeout_secs,
                expose,
                group,
                tag,
                device_type,
                clear_filters,
                json,
                ui_port,
            } => {
                do_action_policy(
                    ui_port,
                    action.as_deref(),
                    mode.as_deref(),
                    max_timeout_secs,
                    expose,
                    group,
                    tag,
                    device_type,
                    clear_filters,
                    json,
                )
                .await?;
            }
            ActionCmd::Watch { json, ui_port } => {
                do_action_watch(ui_port, json).await?;
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
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/notify/dismiss", &bytes)
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
            ScreenCmd::Allow { json, ui_port } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: tetherly_node::ScreenControlReport = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    println!(
                        "Screen viewing allowed (local consent granted). State: {}, Allowed: {}",
                        rep.state, rep.allowed
                    );
                }
            }
            ScreenCmd::Revoke { json, ui_port } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: tetherly_node::ScreenControlReport = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    println!(
                        "Screen viewing consent revoked. State: {}, Allowed: {}",
                        rep.state, rep.allowed
                    );
                }
            }
            ScreenCmd::Start {
                peer,
                json,
                ui_port,
            } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: tetherly_node::ScreenControlReport = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    println!(
                        "Screen streaming started. State: {}, Target peer: {}",
                        rep.state,
                        rep.peer.as_deref().unwrap_or("none")
                    );
                }
            }
            ScreenCmd::Stop { json, ui_port } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: tetherly_node::ScreenControlReport = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    println!("Screen streaming stopped. State: {}", rep.state);
                }
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
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: tetherly_node::ScreenStatusReport = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    println!("Screen Channel Status:");
                    println!("  State:        {}", rep.state);
                    println!("  Allowed:      {}", rep.allowed);
                    println!("  Channel Open: {}", rep.channel_open);
                    if let Some(peer) = &rep.active_peer {
                        println!("  Active Peer:  {peer}");
                    }
                    println!("  Frames Sent:  {}", rep.sent);
                    println!("  Frames Recv:  {}", rep.received);
                    println!("  Dropped:      {}", rep.dropped);
                    println!("  Refused:      {}", rep.refused);
                    println!("  Bytes Sent:   {}", rep.bytes);
                    println!("  Bytes Recv:   {}", rep.rx_bytes);
                    match rep.p95_frame_interval_us {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: tetherly_node::InputControlReport = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    println!(
                        "Input channel connected. Cursor seat: {}, Peer: {}",
                        rep.seat,
                        rep.peer.as_deref().unwrap_or("none")
                    );
                }
            }
            InputCmd::Stop { json, ui_port } => {
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
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: tetherly_node::InputControlReport = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    println!("Input channel disconnected. Cursor seat: {}", rep.seat);
                }
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
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: tetherly_node::InputStatusReport = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    println!("Input Channel Status:");
                    println!("  Channel Open: {}", rep.channel_open);
                    println!("  Cursor Seat:  {}", rep.seat);
                    if let Some(peer) = &rep.active_peer {
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
                        let dev_type = p["device_type"].as_str().unwrap_or("other");
                        let grp_str = match p["group"].as_str() {
                            Some(g) if !g.is_empty() => format!(" · group: \"{g}\""),
                            _ => String::new(),
                        };
                        let tags_str = match p["tags"].as_array() {
                            Some(tarr) if !tarr.is_empty() => {
                                let tstrs: Vec<&str> =
                                    tarr.iter().filter_map(|x| x.as_str()).collect();
                                format!(" · tags: [{}]", tstrs.join(", "))
                            }
                            _ => String::new(),
                        };
                        println!(
                            "  * {display_name} ({platform}/{dev_type}){grp_str}{tags_str} · {addr} [{path}] · rtt: {rtt_str}{feats_str} · {dev}"
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
                            let dev_type = tp["device_type"].as_str().unwrap_or("other");
                            let grp_str = match tp["group"].as_str() {
                                Some(g) if !g.is_empty() => format!(" · group: \"{g}\""),
                                _ => String::new(),
                            };
                            let tags_str = match tp["tags"].as_array() {
                                Some(tarr) if !tarr.is_empty() => {
                                    let tstrs: Vec<&str> =
                                        tarr.iter().filter_map(|x| x.as_str()).collect();
                                    format!(" · tags: [{}]", tstrs.join(", "))
                                }
                                _ => String::new(),
                            };
                            println!(
                                "  * [{status}] {alias} ({dev_type}){grp_str}{tags_str} · {dev}"
                            );
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
            refresh,
            interfaces,
            json,
            ui_port,
        } => {
            if refresh {
                let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/net/refresh", b"{}")
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
                let rep: tetherly_node::NetworkRefreshReport = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                println!("Network & Overlay Rescanned:");
                println!(
                    "  Interfaces Scanned: {} ({} active listening)",
                    rep.ifaces_scanned, rep.ifaces_listening
                );
                println!(
                    "  EasyTier Sidecar:   {} ({})",
                    rep.overlay_present, rep.overlay_source
                );
                let local_str = if rep.overlay_local_ips.is_empty() {
                    "(none)".to_string()
                } else {
                    rep.overlay_local_ips.join(", ")
                };
                println!("  Overlay Local IPs:  {local_str}");
                println!("  Overlay Mesh Peers: {}", rep.overlay_peers_count);
                return Ok(());
            }

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
            let rep: tetherly_node::NetworkReport =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;

            let ov = &rep.overlay;
            let ov_enabled = ov.enabled;
            let ov_present = ov.present;
            let ov_source = &ov.source;
            let local_ips = if ov.local_ips.is_empty() {
                "(none)".to_string()
            } else {
                ov.local_ips.join(", ")
            };
            let peer_count = ov.peer_count;
            let peers = ov.peers.join(", ");

            println!("EasyTier Overlay Sidecar:");
            println!("  Enabled:    {ov_enabled}");
            println!("  Present:    {ov_present} ({ov_source})");
            println!("  Local IPs:  {local_ips}");
            if peer_count == 0 {
                println!("  Mesh Peers: 0");
            } else {
                println!("  Mesh Peers: {peer_count} ({peers})");
            }

            let ifaces_arr = &rep.interfaces;
            if interfaces {
                println!("\nInterface Classification Details:");
                if ifaces_arr.is_empty() {
                    println!("  (no interfaces reported)");
                } else {
                    for iface in ifaces_arr {
                        let name = &iface.iface;
                        let ip = &iface.ip;
                        if iface.listen {
                            println!("  * [LISTEN] {name}: {ip}");
                        } else {
                            let r_str = iface
                                .reason
                                .as_deref()
                                .map(|r| format!(" ({r})"))
                                .unwrap_or_default();
                            println!("  * [SKIP]   {name}: {ip}{r_str}");
                        }
                    }
                }
            } else {
                println!("\nActive Listening Interfaces:");
                let mut listen_count = 0;
                for iface in ifaces_arr {
                    if iface.listen {
                        println!("  * {}: {}", iface.iface, iface.ip);
                        listen_count += 1;
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
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/ancs/disconnect", b"{}")
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
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/ancs/rules/remove", &bytes)
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
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/ancs/rules/clear", b"{}")
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
                let (code, resp) =
                    tetherly_node::loopback_get(ui_port, &path)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "inbox list query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let rep: tetherly_node::InboxReport = serde_json::from_slice(&resp)?;
                    if rep.files.is_empty() {
                        println!("Inbox is empty (0 files).");
                    } else {
                        println!(
                            "Inbox Files ({}, total {}):",
                            rep.count,
                            format_bytes(rep.total_bytes)
                        );
                        println!(
                            "{:<32} {:<12} {:<16} {:<6}",
                            "NAME", "SIZE", "MODIFIED", "SAFE"
                        );
                        println!("{:-<70}", "");
                        for f in &rep.files {
                            let mod_str = format_timestamp_ago(f.modified_at_secs);
                            let safe_str = if f.is_safe { "yes" } else { "no" };
                            let name_display = if f.name.len() > 30 {
                                format!("{}...", &f.name[..27])
                            } else {
                                f.name.clone()
                            };
                            println!(
                                "{:<32} {:<12} {:<16} {:<6}",
                                name_display,
                                format_bytes(f.size),
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
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/file/inbox/delete", &bytes)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "inbox delete failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let rep: tetherly_node::InboxDeleteReport = serde_json::from_slice(&resp)?;
                    println!(
                        "Deleted inbox file '{}' (freed {}).",
                        rep.name,
                        format_bytes(rep.freed_bytes)
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
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/file/inbox/clear", &bytes)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "inbox clear failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let rep: tetherly_node::InboxClearReport = serde_json::from_slice(&resp)?;
                    println!(
                        "Inbox cleared: deleted {} files (freed {}).",
                        rep.deleted_count,
                        format_bytes(rep.freed_bytes)
                    );
                }
            }
            InboxCmd::Get {
                name,
                dest,
                ui_port,
            } => {
                let encoded_name = url_encode(&name);
                let path = format!("/api/file/inbox/download?name={encoded_name}");
                let (code, resp) =
                    tetherly_node::loopback_get(ui_port, &path)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "inbox get failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                let target_path = match dest {
                    Some(p) => {
                        if p.is_dir() {
                            p.join(&name)
                        } else {
                            p
                        }
                    }
                    None => PathBuf::from(&name),
                };
                if let Some(parent) = target_path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)?;
                    }
                }
                std::fs::write(&target_path, &resp)?;
                println!(
                    "Extracted '{}' ({} bytes) -> {}",
                    name,
                    resp.len(),
                    target_path.display()
                );
            }
            InboxCmd::Open {
                name,
                json,
                ui_port,
            } => {
                let payload = serde_json::json!({ "name": name });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/file/inbox/open", &bytes)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "inbox open failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let rep: tetherly_node::InboxOpenReport = serde_json::from_slice(&resp)?;
                    if let Some(f) = rep.name {
                        println!(
                            "Opened file '{}' in system default handler (path: {}).",
                            f, rep.path
                        );
                    } else {
                        println!(
                            "Opened inbox directory in system file manager (path: {}).",
                            rep.path
                        );
                    }
                }
            }
        },
        Cmd::Link {
            action,
            json,
            ui_port,
        } => {
            handle_link_cmd(action, json, ui_port).await?;
        }
        Cmd::Audit {
            action,
            category,
            severity,
            limit,
            since,
            watch,
            interval,
            clear,
            json,
            ui_port,
        } => {
            handle_audit_cmd(
                action, category, severity, limit, since, watch, interval, clear, json, ui_port,
            )
            .await?;
        }
        Cmd::Message {
            action,
            peer,
            tag,
            group,
            device_type,
            all,
            text,
            limit,
            unread,
            mark_read,
            clear,
            watch,
            json,
            ui_port,
        } => {
            handle_message_cmd(
                action,
                peer,
                tag,
                group,
                device_type,
                all,
                text,
                limit,
                unread,
                mark_read,
                clear,
                watch,
                json,
                ui_port,
            )
            .await?;
        }
        Cmd::Outbox {
            action,
            peer,
            status,
            limit,
            clear,
            resend,
            watch,
            json,
            ui_port,
        } => {
            handle_outbox_cmd(
                action, peer, status, limit, clear, resend, watch, json, ui_port,
            )
            .await?;
        }
        Cmd::Banner { action } => match action {
            BannerCmd::Config {
                banners,
                dnd,
                sound,
                mask_otp,
                allow_otp_dnd,
                json,
                ui_port,
            } => {
                let has_update = banners.is_some()
                    || dnd.is_some()
                    || sound.is_some()
                    || mask_otp.is_some()
                    || allow_otp_dnd.is_some();
                let (code, resp) = if has_update {
                    let mut payload = serde_json::Map::new();
                    if let Some(b) = banners {
                        payload.insert("banners_enabled".into(), serde_json::Value::Bool(b));
                    }
                    if let Some(d) = dnd {
                        payload.insert("dnd_mode".into(), serde_json::Value::String(d));
                    }
                    if let Some(s) = sound {
                        payload.insert("sound_enabled".into(), serde_json::Value::Bool(s));
                    }
                    if let Some(m) = mask_otp {
                        payload.insert("mask_otp".into(), serde_json::Value::Bool(m));
                    }
                    if let Some(a) = allow_otp_dnd {
                        payload.insert("allow_otp_in_dnd".into(), serde_json::Value::Bool(a));
                    }
                    let bytes = serde_json::to_vec(&payload)?;
                    tetherly_node::loopback_post(ui_port, "/api/notify/config", &bytes).await?
                } else {
                    tetherly_node::loopback_get(ui_port, "/api/notify/config").await?
                };

                if code != 200 {
                    anyhow::bail!(
                        "notify config failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let rep: tetherly_node::NotifyConfigReport = serde_json::from_slice(&resp)?;
                    println!("Desktop Notification Configuration:");
                    println!("  Banners Enabled:  {}", rep.config.banners_enabled);
                    println!("  DND Mode:         {:?}", rep.config.dnd_mode);
                    println!("  Sound Enabled:    {}", rep.config.sound_enabled);
                    println!("  Mask OTP Code:    {}", rep.config.mask_otp);
                    println!("  Allow OTP in DND: {}", rep.config.allow_otp_in_dnd);
                    println!("  Total Dispatched: {}", rep.total_dispatched);
                    println!("  Total Suppressed: {}", rep.total_suppressed);
                }
            }
            BannerCmd::Test {
                title,
                body,
                json,
                ui_port,
            } => {
                let payload = serde_json::json!({
                    "title": title,
                    "body": body,
                });
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/notify/test", &bytes).await?;
                if code != 200 {
                    anyhow::bail!(
                        "notify test failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp));
                } else {
                    let rep: tetherly_node::NotifyTestReport = serde_json::from_slice(&resp)?;
                    println!("Test notification dispatched: '{}'", rep.title);
                }
            }
        },
        Cmd::Profile {
            action,
            tag,
            group,
            device_type,
            json,
            ui_port,
        } => match action {
            None => handle_profile_list(tag, group, device_type, json, ui_port).await?,
            Some(ProfileCmd::List {
                tag: t,
                group: g,
                device_type: dt,
                json: j,
                ui_port: p,
            }) => {
                let final_tag = t.or(tag);
                let final_group = g.or(group);
                let final_type = dt.or(device_type);
                let final_port = if p != 45716 { p } else { ui_port };
                handle_profile_list(final_tag, final_group, final_type, j || json, final_port)
                    .await?;
            }
            Some(ProfileCmd::Get {
                peer,
                json: j,
                ui_port: p,
            }) => {
                let final_port = if p != 45716 { p } else { ui_port };
                handle_profile_get(&peer, j || json, final_port).await?;
            }
            Some(ProfileCmd::Set {
                peer,
                device_type: dt,
                group: g,
                notes: n,
                json: j,
                ui_port: p,
            }) => {
                let final_port = if p != 45716 { p } else { ui_port };
                handle_profile_set(&peer, dt, g, n, j || json, final_port).await?;
            }
        },
        Cmd::Tag { action } => match action {
            TagCmd::Add {
                peer,
                tag,
                json,
                ui_port,
            } => {
                handle_tag_add(&peer, &tag, json, ui_port).await?;
            }
            TagCmd::Remove {
                peer,
                tag,
                json,
                ui_port,
            } => {
                handle_tag_remove(&peer, &tag, json, ui_port).await?;
            }
        },
        Cmd::Group { action } => match action {
            GroupCmd::Set {
                peer,
                group,
                json,
                ui_port,
            } => {
                handle_group_set(&peer, &group, json, ui_port).await?;
            }
            GroupCmd::Clear {
                peer,
                json,
                ui_port,
            } => {
                handle_group_clear(&peer, json, ui_port).await?;
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
        Cmd::Sys {
            watch,
            interval,
            json,
            ui_port,
        } => loop {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/sys")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused system telemetry query (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let v: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                if watch {
                    print!("\x1b[2J\x1b[H");
                }
                print_system_report(&v);
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
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/connect", &bytes)
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
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/block", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused block (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: tetherly_node::PeerBlockReport = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                let display_name = rep.alias.or(rep.name).unwrap_or_else(|| "peer".to_string());
                let reason_str = rep
                    .reason
                    .as_deref()
                    .map(|r| format!(" (reason: \"{r}\")"))
                    .unwrap_or_default();
                if rep.already_blocked {
                    println!(
                        "peer {} ({}) is already blocked{}",
                        rep.device_id, display_name, reason_str
                    );
                } else {
                    let active_str = if rep.disconnected_active {
                        " (active session terminated)"
                    } else {
                        ""
                    };
                    println!(
                        "successfully blocked peer {} ({}){}{}",
                        rep.device_id, display_name, reason_str, active_str
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
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/unblock", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused unblock (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: tetherly_node::PeerUnblockReport = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                if rep.was_blocked {
                    println!("successfully unblocked peer {}", rep.device_id);
                } else {
                    println!("peer {} was not in the blocklist", rep.device_id);
                }
            }
        }
        Cmd::Blocked { json, ui_port } => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/peers/blocked")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused blocked query (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: tetherly_node::BlockedPeersReport = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                if rep.peers.is_empty() {
                    println!("no peers currently blocked");
                } else {
                    println!("blocked peers ({}):", rep.peers.len());
                    for p in rep.peers {
                        let reason = p
                            .reason
                            .as_deref()
                            .map(|r| format!(" · reason: \"{r}\""))
                            .unwrap_or_default();
                        println!("  - {}{}", p.device_id, reason);
                    }
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

async fn handle_link_cmd(
    action: Option<LinkCmd>,
    default_json: bool,
    default_ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = tetherly_node::loopback_get(default_ui_port, "/api/peers/link")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{default_ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused link quality query (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if default_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let report: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                print_link_quality_report(&report);
            }
        }
        Some(LinkCmd::Status {
            watch: false,
            interval: _,
            json,
            ui_port,
        }) => {
            let json = json || default_json;
            let ui_port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/peers/link")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused link quality query (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let report: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                print_link_quality_report(&report);
            }
        }
        Some(LinkCmd::Status {
            watch: true,
            interval,
            json,
            ui_port,
        }) => {
            let json = json || default_json;
            let ui_port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            loop {
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/peers/link")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused link quality query (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let report: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    print!("\x1b[2J\x1b[H");
                    print_link_quality_report(&report);
                }
                tokio::time::sleep(std::time::Duration::from_secs(interval.max(1))).await;
            }
        }
        Some(LinkCmd::Probe {
            peer,
            count,
            interval_ms,
            json,
            ui_port,
        }) => {
            let json = json || default_json;
            let ui_port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            let body = serde_json::json!({
                "peer": peer,
                "count": count,
                "interval_ms": interval_ms,
            });
            let payload = serde_json::to_vec(&body)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/peers/link/probe", &payload)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused link probe (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                print_link_probe_report(&rep);
            }
        }
        Some(LinkCmd::Config {
            interval,
            deadline,
            json,
            ui_port,
        }) => {
            let json = json || default_json;
            let ui_port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            if interval.is_some() || deadline.is_some() {
                let body = serde_json::json!({
                    "interval_secs": interval,
                    "deadline_secs": deadline,
                });
                let payload = serde_json::to_vec(&body)?;
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/peers/heartbeat", &payload)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused heartbeat config update (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let cfg: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    println!(
                        "Updated heartbeat config: interval = {}s, deadline = {}s",
                        cfg["interval_secs"].as_u64().unwrap_or(0),
                        cfg["deadline_secs"].as_u64().unwrap_or(0)
                    );
                }
            } else {
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/peers/heartbeat")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "node refused heartbeat config query (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let cfg: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
                    println!(
                        "Current heartbeat config: interval = {}s, deadline = {}s",
                        cfg["interval_secs"].as_u64().unwrap_or(0),
                        cfg["deadline_secs"].as_u64().unwrap_or(0)
                    );
                }
            }
        }
    }
    Ok(())
}

fn print_link_quality_report(v: &serde_json::Value) {
    let hb = &v["heartbeat"];
    let hb_interval = hb["interval_secs"].as_u64().unwrap_or(0);
    let hb_deadline = hb["deadline_secs"].as_u64().unwrap_or(0);
    let total = v["total_peers"].as_u64().unwrap_or(0);
    let online = v["online_peers"].as_u64().unwrap_or(0);

    println!(
        "Peer Link Quality Profiling (Heartbeat: {}s, Deadline: {}s) [{} online / {} total]",
        hb_interval, hb_deadline, online, total
    );
    let empty_vec = vec![];
    let links = v["links"].as_array().unwrap_or(&empty_vec);
    if links.is_empty() {
        println!("  (no peer links tracked)");
        return;
    }

    println!(
        "{:<20} {:<15} {:<8} {:<8} {:<18} {:<10} {:<8} {:<10}",
        "PEER ID", "NAME", "STATUS", "RTT", "MIN/MAX/AVG", "JITTER", "LOSS", "GRADE"
    );
    for link in links {
        let dev = link["device_id"].as_str().unwrap_or("?");
        let short_dev = if dev.len() > 18 { &dev[..18] } else { dev };
        let name = link["name"].as_str().unwrap_or("-");
        let short_name = if name.len() > 14 { &name[..14] } else { name };
        let status = if link["is_online"].as_bool().unwrap_or(false) {
            "online"
        } else {
            "offline"
        };
        let rtt = link["rtt_ms"]
            .as_u64()
            .map(|r| format!("{r}ms"))
            .unwrap_or_else(|| "-".into());
        let min_rtt = link["min_rtt_ms"].as_u64();
        let max_rtt = link["max_rtt_ms"].as_u64();
        let avg_rtt = link["avg_rtt_ms"].as_f64();
        let min_max_avg = match (min_rtt, max_rtt, avg_rtt) {
            (Some(min), Some(max), Some(avg)) => format!("{min}/{max}/{avg:.1}ms"),
            _ => "-".into(),
        };
        let jitter = link["jitter_ms"]
            .as_f64()
            .map(|j| format!("{j:.1}ms"))
            .unwrap_or_else(|| "-".into());
        let loss = format!("{:.1}%", link["packet_loss_rate"].as_f64().unwrap_or(0.0));
        let grade = link["grade"].as_str().unwrap_or("unknown");

        println!(
            "{:<20} {:<15} {:<8} {:<8} {:<18} {:<10} {:<8} {:<10}",
            short_dev, short_name, status, rtt, min_max_avg, jitter, loss, grade
        );
    }
}

fn print_link_probe_report(v: &serde_json::Value) {
    let dev = v["device_id"].as_str().unwrap_or("?");
    let name = v["name"].as_str().unwrap_or("-");
    let sent = v["sent_count"].as_u64().unwrap_or(0);
    let recv = v["recv_count"].as_u64().unwrap_or(0);
    let loss = v["packet_loss_rate"].as_f64().unwrap_or(0.0);
    let grade = v["grade"].as_str().unwrap_or("unknown");
    let min_rtt = v["min_rtt_ms"]
        .as_u64()
        .map(|r| format!("{r}ms"))
        .unwrap_or_else(|| "-".into());
    let max_rtt = v["max_rtt_ms"]
        .as_u64()
        .map(|r| format!("{r}ms"))
        .unwrap_or_else(|| "-".into());
    let avg_rtt = v["avg_rtt_ms"]
        .as_f64()
        .map(|a| format!("{a:.1}ms"))
        .unwrap_or_else(|| "-".into());
    let jitter = v["jitter_ms"]
        .as_f64()
        .map(|j| format!("{j:.1}ms"))
        .unwrap_or_else(|| "-".into());
    let empty_vec = vec![];
    let samples = v["samples_ms"].as_array().unwrap_or(&empty_vec);
    let samples_str: Vec<String> = samples.iter().map(|s| s.to_string()).collect();

    println!("Active Link Probe Report: {dev} ({name})");
    println!("  Probes: {sent} sent, {recv} received, loss: {loss:.1}%");
    println!("  Latency: min={min_rtt}, max={max_rtt}, avg={avg_rtt}, jitter={jitter}");
    println!("  Link Grade: {grade}");
    println!("  Samples (ms): [{}]", samples_str.join(", "));
}

#[allow(clippy::too_many_arguments)]
async fn handle_audit_cmd(
    action: Option<AuditCmd>,
    default_cat: Option<String>,
    default_sev: Option<String>,
    default_lim: Option<usize>,
    default_since: Option<u64>,
    default_watch: bool,
    default_interval: u64,
    default_clear: bool,
    default_json: bool,
    default_ui_port: u16,
) -> Result<()> {
    if default_clear {
        return handle_audit_clear(default_json, default_ui_port).await;
    }
    if default_watch {
        return handle_audit_watch(
            default_cat,
            default_sev,
            0,
            default_interval,
            default_json,
            default_ui_port,
        )
        .await;
    }
    match action {
        None => {
            handle_audit_list(
                default_cat,
                default_sev,
                default_lim,
                default_since,
                default_json,
                default_ui_port,
            )
            .await
        }
        Some(AuditCmd::List {
            category,
            severity,
            limit,
            since,
            json,
            ui_port,
        }) => {
            let cat = category.or(default_cat);
            let sev = severity.or(default_sev);
            let lim = limit.or(default_lim);
            let snc = since.or(default_since);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_audit_list(cat, sev, lim, snc, j, port).await
        }
        Some(AuditCmd::Clear { json, ui_port }) => {
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_audit_clear(j, port).await
        }
        Some(AuditCmd::Watch {
            category,
            severity,
            count,
            json,
            ui_port,
        }) => {
            let cat = category.or(default_cat);
            let sev = severity.or(default_sev);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_audit_watch(cat, sev, count, default_interval, j, port).await
        }
    }
}

async fn handle_audit_list(
    category: Option<String>,
    severity: Option<String>,
    limit: Option<usize>,
    since: Option<u64>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(c) = &category {
        query_params.push(format!("category={c}"));
    }
    if let Some(s) = &severity {
        query_params.push(format!("severity={s}"));
    }
    if let Some(l) = limit {
        query_params.push(format!("limit={l}"));
    }
    if let Some(snc) = since {
        query_params.push(format!("since={snc}"));
    }
    let qs = if query_params.is_empty() {
        String::new()
    } else {
        format!("?{}", query_params.join("&"))
    };
    let path = format!("/api/audit{qs}");
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
            "node refused audit query (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        print_audit_report(&val);
    }
    Ok(())
}

async fn handle_audit_clear(json: bool, ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/audit/clear", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to clear audit log (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let count = val["cleared_count"].as_u64().unwrap_or(0);
        let msg = val["message"].as_str().unwrap_or("cleared");
        println!("Audit log cleared: {count} entries purged ({msg})");
    }
    Ok(())
}

async fn handle_audit_watch(
    category: Option<String>,
    severity: Option<String>,
    count: usize,
    _interval: u64,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        let cat_str = category.as_deref().unwrap_or("all");
        let sev_str = severity.as_deref().unwrap_or("all");
        println!("Streaming live security audit events on 127.0.0.1:{ui_port} (category: {cat_str}, min-severity: {sev_str})...");
    }
    let cat_filter = category.map(|c| c.trim().to_ascii_lowercase());
    let min_sev = severity.and_then(|s| match s.trim().to_ascii_lowercase().as_str() {
        "info" => Some(1),
        "notice" => Some(2),
        "warning" | "warn" => Some(3),
        "alert" | "critical" => Some(4),
        _ => None,
    });

    let mut received = 0usize;
    loop {
        match sse.next_event().await {
            Ok(Some(raw)) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
                    if v["kind"].as_str() == Some("audit_event_recorded") {
                        if let Some(ref cf) = cat_filter {
                            let entry_cat =
                                v["category"].as_str().unwrap_or("").to_ascii_lowercase();
                            if !entry_cat.contains(cf) {
                                continue;
                            }
                        }
                        if let Some(target_sev_rank) = min_sev {
                            let sev_rank = match v["severity"]
                                .as_str()
                                .unwrap_or("")
                                .to_ascii_lowercase()
                                .as_str()
                            {
                                "info" => 1,
                                "notice" => 2,
                                "warning" | "warn" => 3,
                                "alert" | "critical" => 4,
                                _ => 0,
                            };
                            if sev_rank < target_sev_rank {
                                continue;
                            }
                        }
                        received += 1;
                        if json {
                            println!("{raw}");
                        } else {
                            let id = v["id"].as_u64().unwrap_or(0);
                            let cat = v["category"].as_str().unwrap_or("general");
                            let sev = v["severity"].as_str().unwrap_or("info");
                            let action = v["action"].as_str().unwrap_or("?");
                            let target = v["target"]
                                .as_str()
                                .map(|t| format!(" target={t}"))
                                .unwrap_or_default();
                            let summary = v["summary"].as_str().unwrap_or("");
                            let success = v["success"].as_bool().unwrap_or(true);
                            let status = if success { "ok" } else { "failed" };
                            println!(
                                "[audit] #{id} [{sev}] [{cat}] {action}{target} -> {summary} ({status})"
                            );
                        }
                        if count > 0 && received >= count {
                            break;
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => anyhow::bail!("event stream error: {e}"),
        }
    }
    Ok(())
}

fn print_audit_report(val: &serde_json::Value) {
    let total = val["total"].as_u64().unwrap_or(0);
    let returned = val["returned"].as_u64().unwrap_or(0);
    println!("Node Security Audit Trail (total: {total}, showing: {returned}):");
    let entries = match val["entries"].as_array() {
        Some(arr) if !arr.is_empty() => arr,
        _ => {
            println!("  (no audit entries recorded)");
            return;
        }
    };
    println!(
        "  {:<5} {:<12} {:<8} {:<10} {:<20} {:<14} {:<32} {:<6}",
        "ID", "TIME", "SEV", "CATEGORY", "ACTION", "TARGET", "SUMMARY", "STATUS"
    );
    println!("  {}", "-".repeat(115));
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    for e in entries {
        let id = e["id"].as_u64().unwrap_or(0);
        let ts = e["timestamp_ms"].as_u64().unwrap_or(0);
        let ago_str = if now_ms >= ts {
            let diff_s = (now_ms - ts) / 1000;
            format!("{diff_s}s ago")
        } else {
            "just now".to_string()
        };
        let sev = e["severity"].as_str().unwrap_or("info");
        let cat = e["category"].as_str().unwrap_or("-");
        let action = e["action"].as_str().unwrap_or("-");
        let target = e["target"].as_str().unwrap_or("-");
        let summary = e["summary"].as_str().unwrap_or("-");
        let success = e["success"].as_bool().unwrap_or(true);
        let status = if success { "ok" } else { "failed" };
        println!(
            "  #{:<4} {:<12} {:<8} {:<10} {:<20} {:<14} {:<32} {:<6}",
            id, ago_str, sev, cat, action, target, summary, status
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_message_cmd(
    action: Option<MessageCmd>,
    default_peer: Option<String>,
    default_tag: Option<String>,
    default_group: Option<String>,
    default_type: Option<String>,
    default_all: bool,
    default_text: Option<String>,
    default_limit: Option<usize>,
    default_unread: bool,
    default_mark_read: bool,
    default_clear: bool,
    default_watch: bool,
    default_json: bool,
    default_ui_port: u16,
) -> Result<()> {
    if default_clear {
        return handle_message_clear(default_peer, default_json, default_ui_port).await;
    }
    if default_watch {
        return handle_message_watch(default_peer, 0, default_json, default_ui_port).await;
    }
    if let Some(txt) = default_text {
        return handle_message_send(
            default_peer,
            default_tag,
            default_group,
            default_type,
            default_all,
            &txt,
            default_json,
            default_ui_port,
        )
        .await;
    }
    match action {
        None => {
            handle_message_history(
                default_peer,
                default_limit,
                default_unread,
                default_mark_read,
                default_json,
                default_ui_port,
            )
            .await
        }
        Some(MessageCmd::Send {
            peer,
            tag,
            group,
            device_type,
            all,
            text,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let tg = tag.or(default_tag);
            let grp = group.or(default_group);
            let dt = device_type.or(default_type);
            let is_all = all || default_all;
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_message_send(p, tg, grp, dt, is_all, &text, j, port).await
        }
        Some(MessageCmd::History {
            peer,
            limit,
            unread,
            mark_read,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let lim = limit.or(default_limit);
            let un = unread || default_unread;
            let mr = mark_read || default_mark_read;
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_message_history(p, lim, un, mr, j, port).await
        }
        Some(MessageCmd::Clear {
            peer,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_message_clear(p, j, port).await
        }
        Some(MessageCmd::Watch {
            peer,
            count,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_message_watch(p, count, j, port).await
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_message_send(
    peer: Option<String>,
    tag: Option<String>,
    group: Option<String>,
    device_type: Option<String>,
    all: bool,
    text: &str,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let is_multi = all || tag.is_some() || group.is_some() || device_type.is_some();
    if is_multi {
        let payload = serde_json::json!({
            "peer": peer,
            "tag": tag,
            "group": group,
            "type": device_type,
            "all": all,
            "text": text,
        });
        let bytes = serde_json::to_vec(&payload)?;
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/message/send-multi", &bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "failed to send targeted message (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let val: serde_json::Value = serde_json::from_slice(&resp)
                .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
            let selector = val["selector"].as_str().unwrap_or("target");
            let total = val["target_count"].as_u64().unwrap_or(0);
            let succ = val["success_count"].as_u64().unwrap_or(0);
            let skipped = val["skipped_offline_count"].as_u64().unwrap_or(0);
            let failed = val["failed_count"].as_u64().unwrap_or(0);
            println!(
                "Multi-Dispatch Message Report [{selector}]: {succ}/{total} sent, {skipped} skipped offline, {failed} failed"
            );
            if let Some(dispatches) = val["dispatches"].as_array() {
                if !dispatches.is_empty() {
                    println!(
                        "  {:<20} {:<16} {:<10} {:<20} {:<20}",
                        "TARGET", "ALIAS", "STATUS", "MESSAGE ID", "ERROR"
                    );
                    println!("  {}", "-".repeat(88));
                    for d in dispatches {
                        let dev = d["peer"].as_str().unwrap_or("?");
                        let dev_short = if dev.len() > 18 { &dev[..18] } else { dev };
                        let alias = d["peer_alias"].as_str().unwrap_or("-");
                        let alias_short = if alias.len() > 14 {
                            &alias[..14]
                        } else {
                            alias
                        };
                        let status = if d["success"].as_bool().unwrap_or(false) {
                            "sent"
                        } else {
                            "failed"
                        };
                        let msg_id = d["message_id"].as_str().unwrap_or("-");
                        let msg_id_short = if msg_id.len() > 18 {
                            &msg_id[..18]
                        } else {
                            msg_id
                        };
                        let err = d["error"].as_str().unwrap_or("-");
                        let err_short = if err.len() > 18 { &err[..18] } else { err };
                        println!(
                            "  {:<20} {:<16} {:<10} {:<20} {:<20}",
                            dev_short, alias_short, status, msg_id_short, err_short
                        );
                    }
                }
            }
        }
        return Ok(());
    }

    let payload = serde_json::json!({
        "peer": peer.unwrap_or_default(),
        "text": text,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/message/send", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to send message (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let msg = &val["message"];
        let id = msg["id"].as_str().unwrap_or("?");
        let to = msg["peer"].as_str().unwrap_or("?");
        let txt = msg["text"].as_str().unwrap_or("");
        println!("Message sent successfully!");
        println!("  ID:   {id}");
        println!("  To:   {to}");
        println!("  Text: {txt}");
    }
    Ok(())
}

async fn handle_message_history(
    peer: Option<String>,
    limit: Option<usize>,
    unread_only: bool,
    mark_read: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(p) = &peer {
        query_params.push(format!("peer={p}"));
    }
    if let Some(l) = limit {
        query_params.push(format!("limit={l}"));
    }
    if unread_only {
        query_params.push("unread_only=true".to_string());
    }
    if mark_read {
        query_params.push("mark_read=true".to_string());
    }
    let qs = if query_params.is_empty() {
        String::new()
    } else {
        format!("?{}", query_params.join("&"))
    };
    let path = format!("/api/message/history{qs}");
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
            "failed to query message history (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        print_message_history_report(&val);
    }
    Ok(())
}

fn print_message_history_report(val: &serde_json::Value) {
    let total = val["total"].as_u64().unwrap_or(0);
    let unread = val["unread"].as_u64().unwrap_or(0);
    let returned = val["returned"].as_u64().unwrap_or(0);
    println!("Direct Message History (total: {total}, unread: {unread}, showing: {returned}):");
    let messages = match val["messages"].as_array() {
        Some(arr) if !arr.is_empty() => arr,
        _ => {
            println!("  (no messages found)");
            return;
        }
    };
    println!(
        "  {:<16} {:<10} {:<10} {:<24} {:<6} {:<30}",
        "ID", "DIR", "TIME", "PEER", "READ", "CONTENT"
    );
    println!("  {}", "-".repeat(100));
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    for m in messages {
        let id = m["id"].as_str().unwrap_or("-");
        let dir = m["direction"].as_str().unwrap_or("in");
        let ts = m["timestamp_ms"].as_u64().unwrap_or(0);
        let ago_str = if now_ms >= ts {
            let diff_s = (now_ms - ts) / 1000;
            format!("{diff_s}s ago")
        } else {
            "just now".to_string()
        };
        let peer = m["peer"].as_str().unwrap_or("-");
        let read = if m["read"].as_bool().unwrap_or(true) {
            "yes"
        } else {
            "NO"
        };
        let text = m["text"].as_str().unwrap_or("");
        let clean_text: String = text.chars().take(40).collect();
        println!(
            "  {:<16} {:<10} {:<10} {:<24} {:<6} {:<30}",
            id, dir, ago_str, peer, read, clean_text
        );
    }
}

async fn handle_message_clear(peer: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/message/clear", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to clear messages (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let count = val["cleared_count"].as_u64().unwrap_or(0);
        let target = val["target"].as_str().unwrap_or("all");
        println!("Direct messages cleared: {count} messages purged (target: {target})");
    }
    Ok(())
}

async fn handle_message_watch(
    peer: Option<String>,
    count: usize,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        let filter_info = match &peer {
            Some(p) => format!(" (filter peer: {p})"),
            None => String::new(),
        };
        println!("Streaming live direct message events on 127.0.0.1:{ui_port}{filter_info}...");
    }
    let mut received = 0usize;
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = val["kind"].as_str().unwrap_or("");
            if kind == "message_received" || kind == "message_sent" {
                if let Some(target_p) = &peer {
                    let ev_p = val["peer"].as_str().unwrap_or("");
                    if !ev_p.contains(target_p) {
                        continue;
                    }
                }
                received += 1;
                if json {
                    println!("{raw}");
                } else {
                    print_event(&val);
                }
                if count > 0 && received >= count {
                    break;
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_outbox_cmd(
    action: Option<OutboxCmd>,
    default_peer: Option<String>,
    default_status: Option<String>,
    default_limit: Option<usize>,
    default_clear: bool,
    default_resend: Option<String>,
    default_watch: bool,
    default_json: bool,
    default_ui_port: u16,
) -> Result<()> {
    if default_clear {
        return handle_outbox_clear(default_peer, default_status, default_json, default_ui_port)
            .await;
    }
    if let Some(resend_id) = default_resend {
        return handle_outbox_resend(&resend_id, default_json, default_ui_port).await;
    }
    if default_watch {
        return handle_outbox_watch(default_peer, default_json, default_ui_port).await;
    }
    match action {
        None => {
            handle_outbox_list(
                default_peer,
                default_status,
                default_limit,
                default_json,
                default_ui_port,
            )
            .await
        }
        Some(OutboxCmd::List {
            peer,
            status,
            limit,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let s = status.or(default_status);
            let lim = limit.or(default_limit);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_outbox_list(p, s, lim, j, port).await
        }
        Some(OutboxCmd::Clear {
            peer,
            status,
            json,
            ui_port,
        }) => {
            let p = peer.or(default_peer);
            let s = status.or(default_status);
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_outbox_clear(p, s, j, port).await
        }
        Some(OutboxCmd::Resend { id, json, ui_port }) => {
            let j = json || default_json;
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_outbox_resend(&id, j, port).await
        }
        Some(OutboxCmd::Watch { peer, ui_port }) => {
            let p = peer.or(default_peer);
            let port = if ui_port != 45716 {
                ui_port
            } else {
                default_ui_port
            };
            handle_outbox_watch(p, default_json, port).await
        }
    }
}

async fn handle_outbox_list(
    peer: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(p) = &peer {
        query_params.push(format!("peer={p}"));
    }
    if let Some(s) = &status {
        query_params.push(format!("status={s}"));
    }
    if let Some(l) = limit {
        query_params.push(format!("limit={l}"));
    }
    let qs = if query_params.is_empty() {
        String::new()
    } else {
        format!("?{}", query_params.join("&"))
    };
    let path = format!("/api/file/outbox{qs}");
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
            "failed to query outbox history (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        print_outbox_report(&val);
    }
    Ok(())
}

fn print_outbox_report(val: &serde_json::Value) {
    let total = val["total"].as_u64().unwrap_or(0);
    let returned = val["returned"].as_u64().unwrap_or(0);
    println!("Outbox & Sent Files History (total: {total}, showing: {returned}):");
    let entries = match val["entries"].as_array() {
        Some(arr) if !arr.is_empty() => arr,
        _ => {
            println!("  (no outbox entries found)");
            return;
        }
    };
    println!(
        "  {:<20} {:<24} {:<12} {:<12} {:<12} {:<12} {:<24}",
        "ID", "FILENAME", "SIZE", "PEER", "STATUS", "SPEED", "STARTED"
    );
    println!("  {}", "-".repeat(110));
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    for e in entries {
        let id = e["id"].as_str().unwrap_or("-");
        let filename = e["filename"].as_str().unwrap_or("-");
        let clean_filename: String = filename.chars().take(22).collect();
        let size = e["size"].as_u64().unwrap_or(0);
        let peer = e["peer"].as_str().unwrap_or("-");
        let short_peer = if peer.len() > 10 { &peer[..10] } else { peer };
        let status = e["status"].as_str().unwrap_or("unknown");
        let speed = e["speed_bps"].as_u64().unwrap_or(0);
        let speed_str = if speed > 0 {
            format!("{}/s", format_bytes(speed))
        } else {
            "-".to_string()
        };
        let ts = e["started_ms"].as_u64().unwrap_or(0);
        let ago_str = if now_ms >= ts {
            let diff_s = (now_ms - ts) / 1000;
            format!("{diff_s}s ago")
        } else {
            "just now".to_string()
        };
        println!(
            "  {:<20} {:<24} {:<12} {:<12} {:<12} {:<12} {:<24}",
            id,
            clean_filename,
            format_bytes(size),
            short_peer,
            status,
            speed_str,
            ago_str
        );
    }
}

async fn handle_outbox_clear(
    peer: Option<String>,
    status: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "status": status,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/outbox/clear", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to clear outbox records (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let count = val["cleared_count"].as_u64().unwrap_or(0);
        let target_p = val["target_peer"].as_str().unwrap_or("all");
        let target_s = val["target_status"].as_str().unwrap_or("all");
        println!("Outbox purged: {count} entries cleared (peer: {target_p}, status: {target_s})");
    }
    Ok(())
}

async fn handle_outbox_resend(id: &str, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "id": id,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/outbox/resend", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to resend outbox file (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let orig = val["original_id"].as_str().unwrap_or(id);
        let new_id = val["new_transfer_id"].as_str().unwrap_or("?");
        let msg = val["message"].as_str().unwrap_or("success");
        println!("File re-offered successfully!");
        println!("  Original: {orig}");
        println!("  New TxID: {new_id}");
        println!("  Status:   {msg}");
    }
    Ok(())
}

async fn handle_outbox_watch(peer: Option<String>, json: bool, ui_port: u16) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        let filter_info = match &peer {
            Some(p) => format!(" (filter peer: {p})"),
            None => String::new(),
        };
        println!("Streaming live outbox events on 127.0.0.1:{ui_port}{filter_info}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = val["kind"].as_str().unwrap_or("");
            if kind == "outbox_updated" || kind == "outbox_cleared" {
                if let Some(target_p) = &peer {
                    let ev_p = val["peer"].as_str().unwrap_or("");
                    if !ev_p.contains(target_p) {
                        continue;
                    }
                }
                if json {
                    println!("{raw}");
                } else {
                    print_event(&val);
                }
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
        "inbox_file_opened" => {
            let name = v["name"].as_str();
            match name {
                Some(n) => println!("[inbox_file_opened] name={n}"),
                None => println!("[inbox_file_opened] directory"),
            }
        }
        "notification_banner_sent" => {
            let title = v["title"].as_str().unwrap_or("?");
            let app = v["app_name"].as_str().unwrap_or("-");
            let is_otp = v["is_otp"].as_bool().unwrap_or(false);
            println!("[notification_banner_sent] title=\"{title}\" app={app} is_otp={is_otp}");
        }
        "notify_config_changed" => {
            let banners = v["banners_enabled"].as_bool().unwrap_or(true);
            let dnd = v["dnd_mode"].as_str().unwrap_or("off");
            let sound = v["sound_enabled"].as_bool().unwrap_or(true);
            let mask_otp = v["mask_otp"].as_bool().unwrap_or(false);
            let allow_otp_dnd = v["allow_otp_in_dnd"].as_bool().unwrap_or(true);
            println!(
                "[notify_config_changed] banners={banners} dnd={dnd} sound={sound} mask_otp={mask_otp} allow_otp_in_dnd={allow_otp_dnd}"
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
        "peer_discovered" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let name = v["name"].as_str().unwrap_or("?");
            let addr = v["addr"].as_str().unwrap_or("?");
            let caps = v["caps"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("[peer_discovered] {name} ({dev}) at {addr} caps=[{caps}]");
        }
        "peer_expired" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            println!("[peer_expired] peer={dev}");
        }
        "system_metrics_refreshed" => {
            let pid = v["pid"].as_u64().unwrap_or(0);
            let rss = v["rss_bytes"].as_u64().unwrap_or(0);
            let virt = v["virt_bytes"].as_u64().unwrap_or(0);
            let threads = v["thread_count"].as_u64().unwrap_or(0);
            let fds = v["open_fds"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".into());
            let cpu_ms = v["cpu_time_ms"].as_u64().unwrap_or(0);
            println!(
                "[system_metrics_refreshed] pid={pid} rss={} virt={} threads={threads} fds={fds} cpu_time={cpu_ms}ms",
                format_bytes(rss),
                format_bytes(virt)
            );
        }
        "peer_link_quality_refreshed" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let rtt = v["rtt_ms"].as_u64().unwrap_or(0);
            let avg = v["avg_rtt_ms"]
                .as_f64()
                .map(|a| format!("{a:.1}ms"))
                .unwrap_or_else(|| "-".into());
            let jitter = v["jitter_ms"]
                .as_f64()
                .map(|j| format!("{j:.1}ms"))
                .unwrap_or_else(|| "-".into());
            let loss = v["packet_loss_rate"].as_f64().unwrap_or(0.0);
            let grade = v["grade"].as_str().unwrap_or("unknown");
            println!(
                "[peer_link_quality_refreshed] peer={dev} rtt={rtt}ms avg={avg} jitter={jitter} loss={loss:.1}% grade={grade}"
            );
        }
        "heartbeat_config_changed" => {
            let interval = v["interval_secs"].as_u64().unwrap_or(0);
            let deadline = v["deadline_secs"].as_u64().unwrap_or(0);
            println!("[heartbeat_config_changed] interval={interval}s deadline={deadline}s");
        }
        "audit_event_recorded" => {
            let id = v["id"].as_u64().unwrap_or(0);
            let cat = v["category"].as_str().unwrap_or("general");
            let sev = v["severity"].as_str().unwrap_or("info");
            let action = v["action"].as_str().unwrap_or("?");
            let target = v["target"]
                .as_str()
                .map(|t| format!(" target={t}"))
                .unwrap_or_default();
            let summary = v["summary"].as_str().unwrap_or("");
            let success = v["success"].as_bool().unwrap_or(true);
            let status = if success { "ok" } else { "failed" };
            println!("[audit] #{id} [{sev}] [{cat}] {action}{target} -> {summary} ({status})");
        }
        "message_received" => {
            let id = v["id"].as_str().unwrap_or("?");
            let peer = v["peer"].as_str().unwrap_or("?");
            let len = v["len"].as_u64().unwrap_or(0);
            println!("[message_received] id={id} from={peer} (length: {len} bytes)");
        }
        "message_sent" => {
            let id = v["id"].as_str().unwrap_or("?");
            let peer = v["peer"].as_str().unwrap_or("?");
            let len = v["len"].as_u64().unwrap_or(0);
            println!("[message_sent] id={id} to={peer} (length: {len} bytes)");
        }
        "outbox_updated" => {
            let id = v["id"].as_str().unwrap_or("?");
            let tid = v["transfer_id"].as_str().unwrap_or("?");
            let peer = v["peer"].as_str().unwrap_or("?");
            let fname = v["filename"].as_str().unwrap_or("?");
            let sz = v["size"].as_u64().unwrap_or(0);
            let status = v["status"].as_str().unwrap_or("?");
            println!(
                "[outbox_updated] id={id} transfer={tid} peer={peer} file=\"{fname}\" ({}) status={status}",
                format_bytes(sz)
            );
        }
        "outbox_cleared" => {
            let count = v["count"].as_u64().unwrap_or(0);
            let peer = v["peer"]
                .as_str()
                .map(|p| format!(" peer={p}"))
                .unwrap_or_default();
            let status = v["status"]
                .as_str()
                .map(|s| format!(" status={s}"))
                .unwrap_or_default();
            println!("[outbox_cleared] purged={count} entries{peer}{status}");
        }
        "peer_profile_updated" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let dev_type = v["device_type"].as_str().unwrap_or("other");
            let grp = v["group"].as_str().unwrap_or("<none>");
            let tags = v["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("[peer_profile_updated] peer={dev} type={dev_type} group={grp} tags=[{tags}]");
        }
        "multi_dispatch_completed" => {
            let kind = v["kind"].as_str().unwrap_or("?");
            let selector = v["selector"].as_str().unwrap_or("?");
            let total = v["target_count"].as_u64().unwrap_or(0);
            let success = v["success_count"].as_u64().unwrap_or(0);
            let skipped = v["skipped_offline_count"].as_u64().unwrap_or(0);
            let failed = v["failed_count"].as_u64().unwrap_or(0);
            println!(
                "[multi_dispatch_completed] kind={kind} selector={selector} targets={total} (success={success}, skipped={skipped}, failed={failed})"
            );
        }
        "auto_accept_policy_changed" => {
            let enabled = v["enabled"].as_bool().unwrap_or(false);
            let grps = v["allowed_groups"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let tags = v["allowed_tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            println!(
                "[auto_accept_policy_changed] enabled={enabled} groups=[{grps}] tags=[{tags}]"
            );
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

fn print_system_report(v: &serde_json::Value) {
    let node_name = v["node_name"].as_str().unwrap_or("?");
    let device_id = v["device_id"].as_str().unwrap_or("?");
    let proc = &v["process"];
    let host = &v["host"];

    let pid = proc["pid"].as_u64().unwrap_or(0);
    let uptime_secs = proc["uptime_secs"].as_u64().unwrap_or(0);
    let rss = proc["rss_bytes"].as_u64().unwrap_or(0);
    let virt = proc["virt_bytes"].as_u64().unwrap_or(0);
    let threads = proc["thread_count"].as_u64().unwrap_or(0);
    let fds = proc["open_fds"]
        .as_u64()
        .map(|n| n.to_string())
        .unwrap_or_else(|| "N/A".to_string());
    let cpu_time_ms = proc["cpu_time_ms"].as_u64().unwrap_or(0);

    let os = host["os"].as_str().unwrap_or("?");
    let arch = host["arch"].as_str().unwrap_or("?");
    let num_cpus = host["num_cpus"].as_u64().unwrap_or(1);
    let total_mem = host["total_memory_bytes"].as_u64();

    println!("Tetherly Node System Telemetry & Performance Profile:");
    println!("  Node Name:        {node_name}");
    println!("  Device ID:        {device_id}");
    println!("  Host Platform:    {os} ({arch}), {num_cpus} CPU cores");
    if let Some(tm) = total_mem {
        println!("  Host Memory:      {}", format_bytes(tm));
    }
    println!("  Daemon PID:       {pid}");
    println!(
        "  Daemon Uptime:    {} ({uptime_secs}s)",
        format_uptime(uptime_secs)
    );
    println!("  Active Threads:   {threads}");
    println!("  Open Descriptors: {fds}");
    println!("  Resident Memory:  {} ({rss} B)", format_bytes(rss));
    println!("  Virtual Memory:   {} ({virt} B)", format_bytes(virt));
    println!(
        "  CPU Runtime:      {cpu_time_ms}ms ({:.2}s)",
        cpu_time_ms as f64 / 1000.0
    );
}

fn url_encode(input: &str) -> String {
    let mut out = String::new();
    for b in input.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
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
            anyhow::bail!("file offer not found: {target_id}");
        } else {
            anyhow::bail!("endpoint returned 404: {}", String::from_utf8_lossy(&resp));
        }
    }
    if code != 200 {
        anyhow::bail!(
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
        anyhow::bail!("specify a transfer ID to accept or use --all");
    }
    let body_json = serde_json::json!({
        "id": id,
        "all": if all { Some(true) } else { None },
        "peer": peer,
    });
    let body = serde_json::to_vec(&body_json)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/accept", &body)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
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
        anyhow::bail!("specify a transfer ID to reject or use --all");
    }
    let body_json = serde_json::json!({
        "id": id,
        "all": if all { Some(true) } else { None },
        "peer": peer,
    });
    let body = serde_json::to_vec(&body_json)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/reject", &body)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
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

#[allow(clippy::too_many_arguments)]
async fn do_file_auto_accept(
    ui_port: u16,
    enable: bool,
    disable: bool,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    max_size: Option<u64>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    if enable && disable {
        anyhow::bail!("cannot specify both --enable and --disable");
    }
    let has_update = enable
        || disable
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || max_size.is_some()
        || clear_filters;

    if has_update {
        let mut body_map = serde_json::Map::new();
        if enable {
            body_map.insert("enabled".to_string(), serde_json::json!(true));
        } else if disable {
            body_map.insert("enabled".to_string(), serde_json::json!(false));
        }
        if clear_filters {
            body_map.insert("allowed_groups".to_string(), serde_json::json!([]));
            body_map.insert("allowed_tags".to_string(), serde_json::json!([]));
            body_map.insert("allowed_types".to_string(), serde_json::json!([]));
            body_map.insert("max_file_size".to_string(), serde_json::Value::Null);
        } else {
            if let Some(g) = group {
                body_map.insert("allowed_groups".to_string(), serde_json::json!(g));
            }
            if let Some(t) = tag {
                body_map.insert("allowed_tags".to_string(), serde_json::json!(t));
            }
            if let Some(dt) = device_type {
                body_map.insert("allowed_types".to_string(), serde_json::json!(dt));
            }
            if let Some(ms) = max_size {
                if ms == 0 {
                    body_map.insert("max_file_size".to_string(), serde_json::Value::Null);
                } else {
                    body_map.insert("max_file_size".to_string(), serde_json::json!(ms));
                }
            }
        }
        let body = serde_json::to_vec(&serde_json::Value::Object(body_map))?;
        let (code, resp) =
            tetherly_node::loopback_post(ui_port, "/api/file/auto-accept/policy", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
        if code != 200 {
            anyhow::bail!(
                "configure auto-accept policy failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            let pol = &v["policy"];
            let enabled = pol["enabled"].as_bool().unwrap_or(false);
            let count = v
                .get("cumulative_auto_accepted_count")
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            let grps = pol["allowed_groups"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let tags = pol["allowed_tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let types = pol["allowed_types"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let max_sz = pol["max_file_size"]
                .as_u64()
                .map(format_bytes)
                .unwrap_or_else(|| "unlimited".to_string());
            println!("File auto-accept policy updated:");
            println!("  Master Enabled:      {enabled}");
            println!("  Allowed Groups:      [{grps}] (empty = all trusted)");
            println!("  Allowed Tags:        [{tags}] (empty = all trusted)");
            println!("  Allowed Types:       [{types}] (empty = all trusted)");
            println!("  Max File Size:       {max_sz}");
            println!("  Cumulative Accepted: {count} file(s)");
        }
        return Ok(());
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/file/auto-accept/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "query auto-accept policy failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&v)?);
    } else {
        let pol = &v["policy"];
        let enabled = pol["enabled"].as_bool().unwrap_or(false);
        let count = v
            .get("cumulative_auto_accepted_count")
            .and_then(|c| c.as_u64())
            .unwrap_or(0);
        let grps = pol["allowed_groups"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let tags = pol["allowed_tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let types = pol["allowed_types"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let max_sz = pol["max_file_size"]
            .as_u64()
            .map(format_bytes)
            .unwrap_or_else(|| "unlimited".to_string());
        println!("File Auto-Accept Policy:");
        println!("  Master Enabled:      {enabled}");
        println!("  Allowed Groups:      [{grps}] (empty = all trusted)");
        println!("  Allowed Tags:        [{tags}] (empty = all trusted)");
        println!("  Allowed Types:       [{types}] (empty = all trusted)");
        println!("  Max File Size:       {max_sz}");
        println!("  Cumulative Accepted: {count} file(s)");
        println!("  Target:              Trusted Noise-authenticated peers matching policy");
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

async fn handle_profile_list(
    tag: Option<String>,
    group: Option<String>,
    device_type: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut query_params = Vec::new();
    if let Some(t) = &tag {
        query_params.push(format!("tag={t}"));
    }
    if let Some(g) = &group {
        query_params.push(format!("group={g}"));
    }
    if let Some(dt) = &device_type {
        query_params.push(format!("type={dt}"));
    }
    let qs = if query_params.is_empty() {
        String::new()
    } else {
        format!("?{}", query_params.join("&"))
    };
    let path = format!("/api/peers/profiles{qs}");
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
            "failed to list profiles (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let total = val["total"].as_u64().unwrap_or(0);
        let profiles = val["profiles"].as_array();
        println!("Peer Profiles (total: {total}):");
        match profiles {
            Some(arr) if !arr.is_empty() => {
                println!(
                    "  {:<36} {:<12} {:<16} {:<24}",
                    "DEVICE ID", "TYPE", "GROUP", "TAGS"
                );
                for p in arr {
                    let dev = p["device_id"].as_str().unwrap_or("-");
                    let dev_type = p["device_type"].as_str().unwrap_or("other");
                    let group = p["group"].as_str().unwrap_or("-");
                    let tags = p["tags"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    println!("  {:<36} {:<12} {:<16} {:<24}", dev, dev_type, group, tags);
                }
            }
            _ => println!("  (no peer profiles match criteria)"),
        }
    }
    Ok(())
}

async fn handle_profile_get(peer: &str, json: bool, ui_port: u16) -> Result<()> {
    let path = format!("/api/peers/profile?peer={peer}");
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
            "failed to get peer profile (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let dev = val["device_id"].as_str().unwrap_or("-");
        let dev_type = val["device_type"].as_str().unwrap_or("other");
        let group = val["group"].as_str().unwrap_or("<none>");
        let tags = val["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let notes = val["notes"].as_str().unwrap_or("<none>");
        let updated = val["updated_ms"].as_u64().unwrap_or(0);
        println!("Peer Profile for: {dev}");
        println!("  Device Type : {dev_type}");
        println!("  Group       : {group}");
        println!("  Tags        : [{tags}]");
        println!("  Notes       : {notes}");
        println!("  Updated     : {updated} ms");
    }
    Ok(())
}

async fn handle_profile_set(
    peer: &str,
    device_type: Option<String>,
    group: Option<String>,
    notes: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "device_type": device_type,
        "group": group,
        "notes": notes,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/profile", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to update profile (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let msg = val["message"].as_str().unwrap_or("profile updated");
        let dev = val["device_id"].as_str().unwrap_or(peer);
        println!("Profile updated for {dev}: {msg}");
    }
    Ok(())
}

async fn handle_tag_add(peer: &str, tag: &str, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "tag": tag,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/tags/add", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to add tag (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let tags = val["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let dev = val["device_id"].as_str().unwrap_or(peer);
        println!("Tag '{tag}' added to {dev}. Current tags: [{tags}]");
    }
    Ok(())
}

async fn handle_tag_remove(peer: &str, tag: &str, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "tag": tag,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/tags/remove", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to remove tag (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let tags = val["tags"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let dev = val["device_id"].as_str().unwrap_or(peer);
        println!("Tag '{tag}' removed from {dev}. Current tags: [{tags}]");
    }
    Ok(())
}

async fn handle_group_set(peer: &str, group: &str, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "group": group,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/group", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to set group (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let dev = val["device_id"].as_str().unwrap_or(peer);
        println!("Group set to '{group}' for {dev}");
    }
    Ok(())
}

async fn handle_group_clear(peer: &str, json: bool, ui_port: u16) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "group": serde_json::Value::Null,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/peers/group", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to clear group (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp).trim());
    } else {
        let val: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let dev = val["device_id"].as_str().unwrap_or(peer);
        println!("Group cleared for {dev}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_clip_policy(
    ui_port: u16,
    action: Option<String>,
    mode: Option<String>,
    sensitive_guard: bool,
    no_sensitive_guard: bool,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    max_bytes: Option<usize>,
    auto_clear: Option<u64>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    if sensitive_guard && no_sensitive_guard {
        anyhow::bail!("cannot specify both --sensitive-guard and --no-sensitive-guard");
    }
    let has_set_intent = action.as_deref() == Some("set")
        || mode.is_some()
        || sensitive_guard
        || no_sensitive_guard
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || max_bytes.is_some()
        || auto_clear.is_some()
        || clear_filters;

    if has_set_intent {
        let mut payload = serde_json::Map::new();
        if let Some(m) = mode {
            payload.insert("mode".into(), serde_json::Value::String(m));
        }
        if sensitive_guard {
            payload.insert("sensitive_guard".into(), serde_json::Value::Bool(true));
        } else if no_sensitive_guard {
            payload.insert("sensitive_guard".into(), serde_json::Value::Bool(false));
        }
        if let Some(grps) = group {
            payload.insert("allowed_groups".into(), serde_json::to_value(grps)?);
        }
        if let Some(tags) = tag {
            payload.insert("allowed_tags".into(), serde_json::to_value(tags)?);
        }
        if let Some(types) = device_type {
            payload.insert("allowed_types".into(), serde_json::to_value(types)?);
        }
        if let Some(sz) = max_bytes {
            payload.insert("max_payload_bytes".into(), serde_json::json!(sz));
        }
        if let Some(secs) = auto_clear {
            payload.insert("auto_clear_timeout_secs".into(), serde_json::json!(secs));
        }
        if clear_filters {
            payload.insert("clear_filters".into(), serde_json::Value::Bool(true));
        }
        let body_bytes = serde_json::to_vec(&payload)?;
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/clip/policy", &body_bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "failed to update clipboard policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
        } else {
            let v: serde_json::Value = serde_json::from_slice(&resp)?;
            print_clipboard_policy(&v);
        }
    } else {
        let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/clip/policy")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "failed to query clipboard policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
        } else {
            let v: serde_json::Value = serde_json::from_slice(&resp)?;
            print_clipboard_policy(&v);
        }
    }
    Ok(())
}

fn print_clipboard_policy(v: &serde_json::Value) {
    let p = &v["policy"];
    let mode = p["mode"].as_str().unwrap_or("manual");
    let guard = p["sensitive_guard"].as_bool().unwrap_or(true);
    let max_bytes = p["max_payload_bytes"].as_u64().unwrap_or(65536);
    let auto_clear = p["auto_clear_timeout_secs"].as_u64();
    let groups = p["allowed_groups"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let tags = p["allowed_tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let types = p["allowed_types"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    let auto_out = v["auto_synced_out_count"].as_u64().unwrap_or(0);
    let auto_in = v["auto_synced_in_count"].as_u64().unwrap_or(0);
    let blocked = v["sensitive_blocked_count"].as_u64().unwrap_or(0);

    println!("Clipboard Policy & Governance:");
    println!("  Mode:              {mode}");
    println!(
        "  Sensitive Guard:   {}",
        if guard {
            "enabled (fail-closed, blocks keys & credentials)"
        } else {
            "disabled (unrestricted)"
        }
    );
    println!(
        "  Allowed Groups:    {}",
        if groups.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", groups.join(", "))
        }
    );
    println!(
        "  Allowed Tags:      {}",
        if tags.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Types:     {}",
        if types.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
    println!("  Max Payload Bytes: {max_bytes} bytes");
    println!(
        "  Auto-Clear Timer:  {}",
        match auto_clear {
            Some(secs) if secs > 0 => format!("{secs} seconds"),
            _ => "disabled".to_string(),
        }
    );
    println!("  Auto-Synced Out:   {auto_out}");
    println!("  Auto-Synced In:    {auto_in}");
    println!("  Sensitive Blocked: {blocked}");
}

async fn do_clip_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!(
            "Streaming live clipboard synchronization & security events on 127.0.0.1:{ui_port}..."
        );
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "clipboard_synced"
                || kind == "clipboard_policy_changed"
                || kind == "clipboard_sync_blocked"
                || kind == "clipboard_cleared"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "clipboard_synced" => {
                            let seq = v["seq"].as_u64().unwrap_or(0);
                            let dir = v["direction"].as_str().unwrap_or("");
                            let bytes = v["bytes"].as_u64().unwrap_or(0);
                            let hash = v["hash"].as_str().unwrap_or("");
                            let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
                            let src = v["source"].as_str().unwrap_or("");
                            let peers = v["sent_peers"].as_u64().unwrap_or(0);
                            if dir == "outbound" {
                                println!("[clipboard_synced] outbound seq={seq} from={src} ({bytes} bytes, sha256:{short_hash}...) to {peers} peer(s)");
                            } else {
                                println!("[clipboard_synced] inbound seq={seq} from={src} ({bytes} bytes, sha256:{short_hash}...)");
                            }
                        }
                        "clipboard_policy_changed" => {
                            let mode = v["mode"].as_str().unwrap_or("");
                            let guard = v["sensitive_guard"].as_bool().unwrap_or(true);
                            let max_bytes = v["max_payload_bytes"].as_u64().unwrap_or(0);
                            println!("[clipboard_policy_changed] mode={mode} sensitive_guard={guard} max_payload_bytes={max_bytes}");
                        }
                        "clipboard_sync_blocked" => {
                            let reason = v["reason"].as_str().unwrap_or("");
                            let bytes = v["bytes"].as_u64().unwrap_or(0);
                            let hash = v["hash"].as_str().unwrap_or("");
                            let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
                            println!("[clipboard_sync_blocked] BLOCKED reason={reason} ({bytes} bytes, sha256:{short_hash}...)");
                        }
                        "clipboard_cleared" => {
                            let ts = v["timestamp_ms"].as_u64().unwrap_or(0);
                            println!("[clipboard_cleared] timestamp_ms={ts}");
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(())
}

async fn do_clip_poll(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/clip/auto-sync/poll", b"{}")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "clipboard auto-sync poll failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value = serde_json::from_slice(&resp)?;
        let dispatched = v["dispatched_peers"].as_u64().unwrap_or(0);
        let blocked = v["stats"]["sensitive_blocked_count"].as_u64().unwrap_or(0);
        let out = v["stats"]["auto_synced_out_count"].as_u64().unwrap_or(0);
        println!("Clipboard Auto-Sync Poll Result:");
        println!("  Dispatched Peers:    {dispatched}");
        println!("  Total Auto-Synced:   {out}");
        println!("  Sensitive Blocked:   {blocked}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_handoff_send(
    ui_port: u16,
    url: &str,
    title: Option<&str>,
    peer: Option<&str>,
    tag: Option<&str>,
    group: Option<&str>,
    device_type: Option<&str>,
    all: bool,
    auto_open: bool,
    json: bool,
) -> Result<()> {
    let payload = serde_json::json!({
        "url": url,
        "title": title,
        "peer": peer,
        "tag": tag,
        "group": group,
        "device_type": device_type,
        "all": all,
        "auto_open": auto_open,
    });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/handoff/send", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "resource handoff failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        if let Some(target_cnt) = v.get("target_count").and_then(|c| c.as_u64()) {
            let succ_cnt = v["success_count"].as_u64().unwrap_or(0);
            let fail_cnt = v["failed_count"].as_u64().unwrap_or(0);
            let skip_cnt = v["skipped_offline_count"].as_u64().unwrap_or(0);
            let sel_desc = v["selector"].to_string();
            let sent_url = v["url"].as_str().unwrap_or(url);
            let sent_title = v["title"].as_str().unwrap_or("");
            println!("Multi-Peer Resource Handoff Dispatched:");
            println!("  Target Selector:      {sel_desc}");
            println!("  Targets / Dispatched: {succ_cnt} / {target_cnt} (failed: {fail_cnt}, skipped: {skip_cnt})");
            println!("  URL:                  {sent_url}");
            if !sent_title.is_empty() {
                println!("  Title:                {sent_title}");
            }
        } else {
            let id = v["id"].as_str().unwrap_or("-");
            let target_peer = v["peer"].as_str().unwrap_or("-");
            let alias_desc = v["peer_alias"]
                .as_str()
                .map(|a| format!(" ({a})"))
                .unwrap_or_default();
            let sent_url = v["url"].as_str().unwrap_or(url);
            let sent_title = v["title"].as_str().unwrap_or("");
            let auto_req = v["auto_open_requested"].as_bool().unwrap_or(false);
            println!("Resource Handoff Dispatched:");
            println!("  Handoff ID:           {id}");
            println!("  Target Peer:          {target_peer}{alias_desc}");
            println!("  URL:                  {sent_url}");
            if !sent_title.is_empty() {
                println!("  Title:                {sent_title}");
            }
            println!("  Auto-Open Requested:  {auto_req}");
        }
    }
    Ok(())
}

async fn do_handoff_list(ui_port: u16, status: Option<&str>, json: bool) -> Result<()> {
    let mut path = "/api/handoff/inbox".to_string();
    if let Some(s) = status {
        path = format!("{path}?status={s}");
    }
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
            "failed to list handoff inbox (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let entries = v["inbox"].as_array();
        match entries {
            Some(list) if !list.is_empty() => {
                println!("Resource Handoff Inbox ({} entries):", list.len());
                println!(
                    "  {:<24} {:<10} {:<24} TITLE / URL",
                    "ID", "STATUS", "SENDER"
                );
                println!("  {}", "-".repeat(84));
                for entry in list {
                    let id = entry["id"].as_str().unwrap_or("-");
                    let st = entry["status"].as_str().unwrap_or("-");
                    let sender_alias = entry["sender_alias"].as_str().unwrap_or("");
                    let sender_id = entry["sender_id"].as_str().unwrap_or("");
                    let sender_disp = if !sender_alias.is_empty() {
                        let short_id = if sender_id.len() >= 12 {
                            &sender_id[..12]
                        } else {
                            sender_id
                        };
                        format!("{sender_alias} ({short_id}...)")
                    } else if sender_id.len() >= 20 {
                        format!("{}...", &sender_id[..20])
                    } else {
                        sender_id.to_string()
                    };
                    let url = entry["url"].as_str().unwrap_or("");
                    let title = entry["title"].as_str().unwrap_or("");
                    let resource_disp = if !title.is_empty() {
                        format!("{title} ({url})")
                    } else {
                        url.to_string()
                    };
                    println!(
                        "  {:<24} {:<10} {:<24} {}",
                        id, st, sender_disp, resource_disp
                    );
                }
            }
            _ => println!("(handoff inbox is empty)"),
        }
    }
    Ok(())
}

async fn do_handoff_open(ui_port: u16, id: &str, json: bool) -> Result<()> {
    let payload = serde_json::json!({ "id": id });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/handoff/inbox/open", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to open handoff resource (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let entry = &v["entry"];
        let url = entry["url"].as_str().unwrap_or("-");
        let title = entry["title"].as_str().unwrap_or("");
        println!("Opened handoff resource '{id}':");
        if !title.is_empty() {
            println!("  Title:   {title}");
        }
        println!("  URL:     {url}");
        println!("  Status:  opened");
    }
    Ok(())
}

async fn do_handoff_dismiss(ui_port: u16, id: &str, json: bool) -> Result<()> {
    let payload = serde_json::json!({ "id": id });
    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/handoff/inbox/dismiss", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to dismiss handoff resource (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        println!("Dismissed handoff resource '{id}'.");
    }
    Ok(())
}

async fn do_handoff_clear(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_delete(ui_port, "/api/handoff/inbox")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to clear handoff inbox (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let cleared = v["cleared"].as_u64().unwrap_or(0);
        println!("Cleared {cleared} handoff entries from inbox.");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_handoff_policy(
    ui_port: u16,
    action: Option<String>,
    mode: Option<String>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    max_entries: Option<usize>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let has_set_intent = action.as_deref() == Some("set")
        || mode.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || max_entries.is_some()
        || clear_filters;

    if has_set_intent {
        let mut payload = serde_json::Map::new();
        if let Some(m) = mode {
            payload.insert("action".into(), serde_json::Value::String(m));
        }
        if let Some(grps) = group {
            payload.insert(
                "allowed_groups".into(),
                serde_json::Value::Array(grps.into_iter().map(serde_json::Value::String).collect()),
            );
        }
        if let Some(tags) = tag {
            payload.insert(
                "allowed_tags".into(),
                serde_json::Value::Array(tags.into_iter().map(serde_json::Value::String).collect()),
            );
        }
        if let Some(types) = device_type {
            payload.insert(
                "allowed_types".into(),
                serde_json::Value::Array(
                    types.into_iter().map(serde_json::Value::String).collect(),
                ),
            );
        }
        if let Some(max_e) = max_entries {
            payload.insert("max_inbox_entries".into(), serde_json::json!(max_e));
        }
        if clear_filters {
            payload.insert("clear_filters".into(), serde_json::Value::Bool(true));
        }

        let body = serde_json::to_vec(&payload)?;
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/handoff/policy", &body)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "failed to update handoff policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/handoff/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to query handoff policy (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let p = &v["policy"];
    let act = p["action"].as_str().unwrap_or("prompt_only");
    let groups = p["allowed_groups"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let tags = p["allowed_tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let types = p["allowed_types"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let max_entries = p["max_inbox_entries"].as_u64().unwrap_or(100);

    println!("Resource Handoff Ingress Policy:");
    println!("  Action Mode:       {act}");
    println!(
        "  Allowed Groups:    {}",
        if groups.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", groups.join(", "))
        }
    );
    println!(
        "  Allowed Tags:      {}",
        if tags.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Types:     {}",
        if types.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
    println!("  Max Inbox Entries: {max_entries}");
    Ok(())
}

async fn do_handoff_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!("Streaming live resource handoff & policy events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "resource_handoff_received"
                || kind == "resource_handoff_dispatched"
                || kind == "handoff_policy_changed"
                || kind == "handoff_inbox_updated"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "resource_handoff_received" => {
                            let id = v["id"].as_str().unwrap_or("");
                            let sender = v["sender_alias"].as_str().unwrap_or("unknown");
                            let host = v["host"].as_str().unwrap_or("");
                            let title = v["title"].as_str().unwrap_or("");
                            let auto_opened = v["auto_opened"].as_bool().unwrap_or(false);
                            let title_disp = if !title.is_empty() {
                                format!(" \"{title}\"")
                            } else {
                                String::new()
                            };
                            println!("[resource_handoff_received] id={id} from={sender}{title_disp} ({host}) auto_open={auto_opened}");
                        }
                        "resource_handoff_dispatched" => {
                            let id = v["id"].as_str().unwrap_or("");
                            let host = v["host"].as_str().unwrap_or("");
                            let succ = v["success_count"].as_u64().unwrap_or(0);
                            let targets = v["target_count"].as_u64().unwrap_or(0);
                            println!("[resource_handoff_dispatched] id={id} dispatched to {succ}/{targets} targets ({host})");
                        }
                        "handoff_policy_changed" => {
                            let action = v["action"].as_str().unwrap_or("");
                            println!("[handoff_policy_changed] action={action}");
                        }
                        "handoff_inbox_updated" => {
                            let total = v["total_entries"].as_u64().unwrap_or(0);
                            let pending = v["pending_count"].as_u64().unwrap_or(0);
                            println!("[handoff_inbox_updated] total={total} pending={pending}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_media_command(
    ui_port: u16,
    cmd: &str,
    volume: Option<u8>,
    offset_ms: Option<i64>,
    peer: Option<&str>,
    tag: Option<&str>,
    group: Option<&str>,
    device_type: Option<&str>,
    all: bool,
    player: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut payload = serde_json::json!({
        "command": cmd,
    });
    if let Some(vol) = volume {
        payload["volume"] = serde_json::json!(vol);
    }
    if let Some(off) = offset_ms {
        payload["offset_ms"] = serde_json::json!(off);
    }
    if let Some(p) = peer {
        payload["peer"] = serde_json::json!(p);
    }
    if let Some(t) = tag {
        payload["tag"] = serde_json::json!(t);
    }
    if let Some(g) = group {
        payload["group"] = serde_json::json!(g);
    }
    if let Some(dt) = device_type {
        payload["device_type"] = serde_json::json!(dt);
    }
    if all {
        payload["all"] = serde_json::json!(true);
    }
    if let Some(pl) = player {
        payload["target_player"] = serde_json::json!(pl);
    }

    let bytes = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/media/command", &bytes)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "media command failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        if let Some(target_cnt) = v.get("target_count").and_then(|c| c.as_u64()) {
            let succ_cnt = v["success_count"].as_u64().unwrap_or(0);
            let fail_cnt = v["failed_count"].as_u64().unwrap_or(0);
            let skip_cnt = v["skipped_offline_count"].as_u64().unwrap_or(0);
            let sel_desc = v["selector"].to_string();
            let cmd_label = v["command"].as_str().unwrap_or(cmd);
            println!("Multi-Peer Media Command Dispatched:");
            println!("  Command:              {cmd_label}");
            println!("  Target Selector:      {sel_desc}");
            println!("  Targets / Dispatched: {succ_cnt} / {target_cnt} (failed: {fail_cnt}, skipped: {skip_cnt})");
        } else {
            let id = v["id"].as_str().unwrap_or("-");
            let target_peer = v["peer"].as_str().unwrap_or("-");
            let alias_desc = v["peer_alias"]
                .as_str()
                .map(|a| format!(" ({a})"))
                .unwrap_or_default();
            let cmd_label = v["command"].as_str().unwrap_or(cmd);
            let succ = v["success"].as_bool().unwrap_or(true);
            println!("Media Command Dispatched:");
            println!("  Command ID:           {id}");
            println!("  Command:              {cmd_label}");
            println!("  Target Peer:          {target_peer}{alias_desc}");
            println!("  Delivered:            {succ}");
        }
    }
    Ok(())
}

async fn do_media_status(ui_port: u16, peer: Option<&str>, json: bool) -> Result<()> {
    let mut path = "/api/media/state".to_string();
    if let Some(p) = peer {
        path = format!("{path}?peer={p}");
    }
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
            "failed to query media status (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let target_peer = v["peer"].as_str().unwrap_or("local");
    if let Some(st) = v.get("state").filter(|s| !s.is_null()) {
        let player_id = st["player_id"].as_str().unwrap_or("default");
        let state = st["state"].as_str().unwrap_or("stopped");
        let title = st["title"].as_str().unwrap_or("");
        let artist = st["artist"].as_str().unwrap_or("");
        let album = st["album"].as_str().unwrap_or("");
        let pos_ms = st["position_ms"].as_u64().unwrap_or(0);
        let dur_ms = st["duration_ms"].as_u64().unwrap_or(0);
        let vol = st["volume_percent"].as_u64().unwrap_or(100);
        let muted = st["muted"].as_bool().unwrap_or(false);

        let pos_sec = pos_ms / 1000;
        let dur_sec = dur_ms / 1000;
        let pos_str = format!("{:02}:{:02}", pos_sec / 60, pos_sec % 60);
        let dur_str = if dur_ms > 0 {
            format!("{:02}:{:02}", dur_sec / 60, dur_sec % 60)
        } else {
            "--:--".to_string()
        };

        println!("Media Playback Status ({target_peer}):");
        println!("  Player:     {player_id}");
        println!("  State:      {state}");
        if !title.is_empty() {
            println!("  Title:      {title}");
        }
        if !artist.is_empty() {
            println!("  Artist:     {artist}");
        }
        if !album.is_empty() {
            println!("  Album:      {album}");
        }
        println!("  Position:   {pos_str} / {dur_str}");
        println!(
            "  Volume:     {vol}%{}",
            if muted { " (muted)" } else { "" }
        );
    } else {
        println!("No active playback on {target_peer}.");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_media_policy(
    ui_port: u16,
    action: Option<String>,
    mode: Option<String>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    max_volume: Option<u8>,
    allow_seek: Option<bool>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_mutation = action.as_deref() == Some("set")
        || mode.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || max_volume.is_some()
        || allow_seek.is_some()
        || clear_filters;

    if is_mutation {
        let mut payload = serde_json::json!({});
        if let Some(m) = mode {
            payload["mode"] = serde_json::json!(m);
        }
        if let Some(grps) = group {
            payload["allowed_groups"] = serde_json::json!(grps);
        }
        if let Some(tags) = tag {
            payload["allowed_tags"] = serde_json::json!(tags);
        }
        if let Some(types) = device_type {
            payload["allowed_types"] = serde_json::json!(types);
        }
        if let Some(lim) = max_volume {
            payload["max_volume_limit"] = serde_json::json!(lim);
        }
        if let Some(seek) = allow_seek {
            payload["allow_seek"] = serde_json::json!(seek);
        }
        if clear_filters {
            payload["clear_filters"] = serde_json::json!(true);
        }

        let body = serde_json::to_vec(&payload)?;
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/media/policy", &body)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "failed to update media policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/media/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to query media policy (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let p = &v["policy"];
    let mode = p["mode"].as_str().unwrap_or("allow");
    let max_vol = p["max_volume_limit"].as_u64().unwrap_or(100);
    let allow_seek = p["allow_seek"].as_bool().unwrap_or(true);
    let groups = p["allowed_groups"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let tags = p["allowed_tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let types = p["allowed_types"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    println!("Media Remote Control Ingress Policy:");
    println!("  Control Mode:      {mode}");
    println!("  Max Volume Limit:  {max_vol}%");
    println!("  Allow Remote Seek: {allow_seek}");
    println!(
        "  Allowed Groups:    {}",
        if groups.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", groups.join(", "))
        }
    );
    println!(
        "  Allowed Tags:      {}",
        if tags.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Types:     {}",
        if types.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
    Ok(())
}

async fn do_media_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!("Streaming live media playback & control events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "media_command_received"
                || kind == "media_command_dispatched"
                || kind == "media_state_changed"
                || kind == "media_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "media_command_received" => {
                            let id = v["id"].as_str().unwrap_or("");
                            let sender = v["sender_alias"].as_str().unwrap_or("unknown");
                            let cmd = v["command"].as_str().unwrap_or("");
                            println!(
                                "[media_command_received] id={id} from={sender} command={cmd}"
                            );
                        }
                        "media_command_dispatched" => {
                            let id = v["id"].as_str().unwrap_or("");
                            let target = v["target_peer"].as_str().unwrap_or("");
                            let cmd = v["command"].as_str().unwrap_or("");
                            let succ = v["success"].as_bool().unwrap_or(true);
                            println!("[media_command_dispatched] id={id} to={target} command={cmd} success={succ}");
                        }
                        "media_state_changed" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let state = v["state"].as_str().unwrap_or("");
                            let title = v["title"].as_str().unwrap_or("");
                            let vol = v["volume_percent"].as_u64().unwrap_or(0);
                            let title_disp = if !title.is_empty() {
                                format!(" \"{title}\"")
                            } else {
                                String::new()
                            };
                            println!("[media_state_changed] peer={peer} state={state}{title_disp} vol={vol}%");
                        }
                        "media_policy_changed" => {
                            let mode = v["mode"].as_str().unwrap_or("");
                            let max_vol = v["max_volume_limit"].as_u64().unwrap_or(100);
                            println!("[media_policy_changed] mode={mode} max_volume={max_vol}%");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}

async fn do_power_status(ui_port: u16, peer: Option<&str>, json: bool) -> Result<()> {
    let mut path = "/api/battery/state".to_string();
    if let Some(p) = peer {
        path = format!("{path}?peer={p}");
    }
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
            "failed to query battery status (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let target_peer = v["peer"].as_str().unwrap_or("local");
    if let Some(st) = v.get("state").filter(|s| !s.is_null()) {
        let level = st["battery_level"].as_u64().unwrap_or(0);
        let charging = st["is_charging"].as_bool().unwrap_or(false);
        let source = st["power_source"].as_str().unwrap_or("unknown");
        let saver = st["battery_saver"].as_bool().unwrap_or(false);
        let health = st.get("health_percent").and_then(|h| h.as_u64());

        let bar_len = (level as usize * 20) / 100;
        let bar = format!("[{}{}]", "=".repeat(bar_len), " ".repeat(20 - bar_len));

        println!("Battery & Power Status ({target_peer}):");
        println!("  Battery Level:     {level}% {bar}");
        println!("  Charging:          {charging}");
        println!("  Power Source:      {source}");
        println!("  Battery Saver:     {saver}");
        if let Some(hp) = health {
            println!("  Battery Health:    {hp}%");
        }
    } else {
        println!("No battery state available for {target_peer}.");
    }
    Ok(())
}

async fn do_power_report(
    ui_port: u16,
    level: Option<u8>,
    charging: Option<bool>,
    power_source: Option<&str>,
    battery_saver: Option<bool>,
    health_percent: Option<u8>,
    json: bool,
) -> Result<()> {
    let mut payload = serde_json::json!({});
    if let Some(lvl) = level {
        payload["battery_level"] = serde_json::json!(lvl);
    }
    if let Some(chg) = charging {
        payload["is_charging"] = serde_json::json!(chg);
    }
    if let Some(src) = power_source {
        payload["power_source"] = serde_json::json!(src);
    }
    if let Some(saver) = battery_saver {
        payload["battery_saver"] = serde_json::json!(saver);
    }
    if let Some(hp) = health_percent {
        payload["health_percent"] = serde_json::json!(hp);
    }

    let (code, resp) = tetherly_node::loopback_post(
        ui_port,
        "/api/battery/state",
        &serde_json::to_vec(&payload)?,
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
            "failed to update battery state (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let st = &v["state"];
    let level = st["battery_level"].as_u64().unwrap_or(0);
    let charging = st["is_charging"].as_bool().unwrap_or(false);
    let source = st["power_source"].as_str().unwrap_or("unknown");
    let saver = st["battery_saver"].as_bool().unwrap_or(false);
    let bcast_cnt = v["broadcast_count"].as_u64().unwrap_or(0);

    println!("Local Battery State Updated & Broadcasted:");
    println!("  Battery Level:     {level}%");
    println!("  Charging:          {charging}");
    println!("  Power Source:      {source}");
    println!("  Battery Saver:     {saver}");
    println!("  Broadcast To:      {bcast_cnt} online peer(s)");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_power_policy(
    ui_port: u16,
    action: Option<String>,
    low_threshold: Option<u8>,
    critical_threshold: Option<u8>,
    alert: Option<bool>,
    throttle: Option<bool>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_mutation = action.as_deref() == Some("set")
        || low_threshold.is_some()
        || critical_threshold.is_some()
        || alert.is_some()
        || throttle.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || clear_filters;

    if is_mutation {
        let mut payload = serde_json::json!({});
        if let Some(low) = low_threshold {
            payload["low_battery_threshold"] = serde_json::json!(low);
        }
        if let Some(crit) = critical_threshold {
            payload["critical_battery_threshold"] = serde_json::json!(crit);
        }
        if let Some(al) = alert {
            payload["alert_on_low_battery"] = serde_json::json!(al);
        }
        if let Some(th) = throttle {
            payload["auto_throttle_on_battery_saver"] = serde_json::json!(th);
        }
        if let Some(grps) = group {
            payload["allowed_groups"] = serde_json::json!(grps);
        }
        if let Some(tags) = tag {
            payload["allowed_tags"] = serde_json::json!(tags);
        }
        if let Some(types) = device_type {
            payload["allowed_types"] = serde_json::json!(types);
        }
        if clear_filters {
            payload["clear_filters"] = serde_json::json!(true);
        }

        let (code, resp) = tetherly_node::loopback_post(
            ui_port,
            "/api/battery/policy",
            &serde_json::to_vec(&payload)?,
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
                "failed to update battery policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/battery/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to query battery policy (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let p = &v["policy"];
    let low = p["low_battery_threshold"].as_u64().unwrap_or(20);
    let crit = p["critical_battery_threshold"].as_u64().unwrap_or(10);
    let alert_enabled = p["alert_on_low_battery"].as_bool().unwrap_or(true);
    let throttle_enabled = p["auto_throttle_on_battery_saver"]
        .as_bool()
        .unwrap_or(false);
    let groups = p["allowed_groups"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let tags = p["allowed_tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let types = p["allowed_types"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    println!("Battery & Energy Governance Policy:");
    println!("  Low Warning Threshold:      {low}%");
    println!("  Critical Alert Threshold:   {crit}%");
    println!("  Alert on Low Battery:       {alert_enabled}");
    println!("  Throttle on Battery Saver:  {throttle_enabled}");
    println!(
        "  Allowed Groups:             {}",
        if groups.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", groups.join(", "))
        }
    );
    println!(
        "  Allowed Tags:               {}",
        if tags.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Types:              {}",
        if types.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
    Ok(())
}

async fn do_power_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!("Streaming live battery & power events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "battery_state_changed"
                || kind == "battery_low_alert"
                || kind == "battery_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "battery_state_changed" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let lvl = v["battery_level"].as_u64().unwrap_or(0);
                            let chg = v["is_charging"].as_bool().unwrap_or(false);
                            let src = v["power_source"].as_str().unwrap_or("unknown");
                            let saver = v["battery_saver"].as_bool().unwrap_or(false);
                            let chg_str = if chg { " [charging]" } else { "" };
                            let saver_str = if saver { " (saver)" } else { "" };
                            println!("[battery_state_changed] peer={peer} level={lvl}%{chg_str} source={src}{saver_str}");
                        }
                        "battery_low_alert" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let lvl = v["battery_level"].as_u64().unwrap_or(0);
                            let is_crit = v["is_critical"].as_bool().unwrap_or(false);
                            let alert_tag = if is_crit { "CRITICAL" } else { "LOW" };
                            println!("[battery_low_alert] [{alert_tag}] peer={peer} battery={lvl}%! Please connect to power.");
                        }
                        "battery_policy_changed" => {
                            let low = v["low_battery_threshold"].as_u64().unwrap_or(20);
                            let crit = v["critical_battery_threshold"].as_u64().unwrap_or(10);
                            let alert = v["alert_on_low_battery"].as_bool().unwrap_or(true);
                            println!("[battery_policy_changed] low={low}% critical={crit}% alert={alert}");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}

async fn do_locate_ring(
    ui_port: u16,
    peer: &str,
    duration_secs: Option<u32>,
    pattern: Option<&str>,
    override_silent: Option<bool>,
    message: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut payload = serde_json::json!({
        "peer": peer,
    });
    if let Some(dur) = duration_secs {
        payload["duration_secs"] = serde_json::json!(dur);
    }
    if let Some(pat) = pattern {
        payload["pattern"] = serde_json::json!(pat);
    }
    if let Some(ov) = override_silent {
        payload["override_silent"] = serde_json::json!(ov);
    }
    if let Some(msg) = message {
        payload["message"] = serde_json::json!(msg);
    }

    let (code, resp) =
        tetherly_node::loopback_post(ui_port, "/api/locate/ring", &serde_json::to_vec(&payload)?)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
                )
            })?;

    if code != 200 {
        anyhow::bail!(
            "failed to trigger locate ring (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let target = v["target_peer"].as_str().unwrap_or(peer);
    let req_id = v["request_id"].as_str().unwrap_or("unknown");
    let dur = v["duration_secs"].as_u64().unwrap_or(15);
    let pat = v["pattern"].as_str().unwrap_or("chime");
    let ov = v["override_silent"].as_bool().unwrap_or(true);
    let msg = v["message"].as_str().unwrap_or("");

    println!("Locate Acoustic Alarm Triggered:");
    println!("  Target Peer:       {target}");
    println!("  Request ID:        {req_id}");
    println!("  Duration:          {dur}s");
    println!("  Pattern:           {pat}");
    println!("  Override Silent:   {ov}");
    if !msg.is_empty() {
        println!("  Message:           {msg}");
    }
    Ok(())
}

async fn do_locate_stop(
    ui_port: u16,
    peer: Option<&str>,
    request_id: Option<&str>,
    reason: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut payload = serde_json::json!({});
    if let Some(p) = peer {
        payload["peer"] = serde_json::json!(p);
    }
    if let Some(id) = request_id {
        payload["request_id"] = serde_json::json!(id);
    }
    if let Some(r) = reason {
        payload["reason"] = serde_json::json!(r);
    }

    let (code, resp) =
        tetherly_node::loopback_post(ui_port, "/api/locate/stop", &serde_json::to_vec(&payload)?)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
                )
            })?;

    if code != 200 {
        anyhow::bail!(
            "failed to stop locate ring (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let target = v["target_peer"].as_str().unwrap_or("local");
    let req_id = v["request_id"].as_str().unwrap_or("active");
    let rsn = v["reason"].as_str().unwrap_or("unspecified");

    println!("Locate Ring Stopped:");
    println!("  Target Peer:       {target}");
    println!("  Request ID:        {req_id}");
    println!("  Reason:            {rsn}");
    Ok(())
}

async fn do_locate_status(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/locate/status")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to query locate status (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let is_ringing = v["is_ringing"].as_bool().unwrap_or(false);

    println!("Device Locator Status:");
    println!("  Is Ringing:        {is_ringing}");
    if let Some(ar) = v.get("active_ring").filter(|r| !r.is_null()) {
        let req_id = ar["request_id"].as_str().unwrap_or("");
        let init_peer = ar["initiator_peer"].as_str().unwrap_or("");
        let init_alias = ar["initiator_alias"].as_str().unwrap_or("");
        let pat = ar["pattern"].as_str().unwrap_or("chime");
        let dur = ar["duration_secs"].as_u64().unwrap_or(0);
        let rem = ar["remaining_secs"].as_u64().unwrap_or(0);
        let ov = ar["override_silent"].as_bool().unwrap_or(true);
        let msg = ar["message"].as_str().unwrap_or("");

        println!("  Active Ringing Alarm:");
        println!("    Request ID:      {req_id}");
        println!("    Initiator Peer:  {init_peer} ({init_alias})");
        println!("    Pattern:         {pat}");
        println!("    Duration:        {dur}s (remaining: {rem}s)");
        println!("    Override Silent: {ov}");
        if !msg.is_empty() {
            println!("    Message:         {msg}");
        }
    }

    if let Some(p) = v.get("policy").filter(|p| !p.is_null()) {
        let enabled = p["enabled"].as_bool().unwrap_or(true);
        let allow_ov = p["allow_override_silent"].as_bool().unwrap_or(true);
        let max_dur = p["max_duration_secs"].as_u64().unwrap_or(60);
        println!("  Policy Summary:");
        println!("    Enabled:         {enabled}");
        println!("    Override Silent: {allow_ov}");
        println!("    Max Duration:    {max_dur}s");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_locate_policy(
    ui_port: u16,
    action: Option<String>,
    enabled: Option<bool>,
    override_silent: Option<bool>,
    max_duration: Option<u32>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_mutation = action.as_deref() == Some("set")
        || enabled.is_some()
        || override_silent.is_some()
        || max_duration.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || clear_filters;

    if is_mutation {
        let mut payload = serde_json::json!({});
        if let Some(en) = enabled {
            payload["enabled"] = serde_json::json!(en);
        }
        if let Some(ov) = override_silent {
            payload["allow_override_silent"] = serde_json::json!(ov);
        }
        if let Some(dur) = max_duration {
            payload["max_duration_secs"] = serde_json::json!(dur);
        }
        if let Some(grps) = group {
            payload["allowed_groups"] = serde_json::json!(grps);
        }
        if let Some(tags) = tag {
            payload["allowed_tags"] = serde_json::json!(tags);
        }
        if let Some(types) = device_type {
            payload["allowed_types"] = serde_json::json!(types);
        }
        if clear_filters {
            payload["clear_filters"] = serde_json::json!(true);
        }

        let (code, resp) = tetherly_node::loopback_post(
            ui_port,
            "/api/locate/policy",
            &serde_json::to_vec(&payload)?,
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
                "failed to update locate policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/locate/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code != 200 {
        anyhow::bail!(
            "failed to query locate policy (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let p = &v["policy"];
    let en = p["enabled"].as_bool().unwrap_or(true);
    let allow_ov = p["allow_override_silent"].as_bool().unwrap_or(true);
    let max_dur = p["max_duration_secs"].as_u64().unwrap_or(60);
    let groups = p["allowed_groups"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let tags = p["allowed_tags"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();
    let types = p["allowed_types"]
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>())
        .unwrap_or_default();

    println!("Device Locator Policy:");
    println!("  Enabled:                    {en}");
    println!("  Allow Silent Override:      {allow_ov}");
    println!("  Max Duration:               {max_dur}s");
    println!(
        "  Allowed Groups:             {}",
        if groups.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", groups.join(", "))
        }
    );
    println!(
        "  Allowed Tags:               {}",
        if tags.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Types:              {}",
        if types.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
    Ok(())
}

async fn do_locate_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!("Streaming live device locator events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "locate_ring_triggered"
                || kind == "locate_ring_stopped"
                || kind == "locate_policy_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "locate_ring_triggered" => {
                            let req_id = v["request_id"].as_str().unwrap_or("");
                            let peer = v["initiator_alias"].as_str().unwrap_or("unknown");
                            let pat = v["pattern"].as_str().unwrap_or("chime");
                            let dur = v["duration_secs"].as_u64().unwrap_or(0);
                            let ov = v["override_silent"].as_bool().unwrap_or(true);
                            let msg = v["message"].as_str().unwrap_or("");
                            let msg_str = if !msg.is_empty() {
                                format!(" msg=\"{msg}\"")
                            } else {
                                String::new()
                            };
                            println!("[locate_ring_triggered] req={req_id} peer={peer} pattern={pat} duration={dur}s override={ov}{msg_str}");
                        }
                        "locate_ring_stopped" => {
                            let req_id = v["request_id"].as_str().unwrap_or("");
                            let peer = v["stopped_by_alias"].as_str().unwrap_or("unknown");
                            let reason = v["reason"].as_str().unwrap_or("unspecified");
                            println!("[locate_ring_stopped] req={req_id} stopped_by={peer} reason={reason}");
                        }
                        "locate_policy_changed" => {
                            let en = v["enabled"].as_bool().unwrap_or(true);
                            let ov = v["allow_override_silent"].as_bool().unwrap_or(true);
                            let max_d = v["max_duration_secs"].as_u64().unwrap_or(60);
                            println!("[locate_policy_changed] enabled={en} override_silent={ov} max_duration={max_d}s");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}

async fn do_action_local_list(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/action/local")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "failed to list local actions (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let val: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let actions = if let Some(arr) = val.get("actions").and_then(|a| a.as_array()) {
        arr.clone()
    } else if let Some(arr) = val.as_array() {
        arr.clone()
    } else {
        Vec::new()
    };

    println!("Local Predefined Actions ({})", actions.len());
    if actions.is_empty() {
        println!("  (none configured)");
        return Ok(());
    }

    for a in actions {
        let id = a["id"].as_str().unwrap_or("");
        let name = a["name"].as_str().unwrap_or("");
        let cmd = a["command"].as_str().unwrap_or("");
        let args = a["args"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let timeout = a["timeout_secs"].as_u64().unwrap_or(10);
        let enabled = a["enabled"].as_bool().unwrap_or(true);
        let desc = a["description"].as_str().unwrap_or("-");
        let icon = a["icon"].as_str().unwrap_or("-");

        println!("  - ID:          {id}");
        println!("    Name:        {name}");
        println!("    Command:     {cmd} {args}");
        println!("    Timeout:     {timeout}s (enabled: {enabled})");
        println!("    Icon/Desc:   [{icon}] {desc}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_action_local_add(
    ui_port: u16,
    id: &str,
    name: &str,
    cmd: &str,
    args: Option<Vec<String>>,
    desc: Option<&str>,
    icon: Option<&str>,
    timeout: Option<u32>,
    enabled: Option<bool>,
    json: bool,
) -> Result<()> {
    let mut payload = serde_json::json!({
        "id": id,
        "name": name,
        "command": cmd,
        "args": args.unwrap_or_default(),
    });
    if let Some(d) = desc {
        payload["description"] = serde_json::json!(d);
    }
    if let Some(i) = icon {
        payload["icon"] = serde_json::json!(i);
    }
    if let Some(t) = timeout {
        payload["timeout_secs"] = serde_json::json!(t);
    }
    if let Some(en) = enabled {
        payload["enabled"] = serde_json::json!(en);
    }

    let (code, resp) =
        tetherly_node::loopback_post(ui_port, "/api/action/local", &serde_json::to_vec(&payload)?)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
                )
            })?;

    if code != 200 {
        anyhow::bail!(
            "failed to register local action (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    println!("Local Action Registered:");
    println!("  ID:      {id}");
    println!("  Name:    {name}");
    println!("  Command: {cmd}");
    Ok(())
}

async fn do_action_local_remove(ui_port: u16, id: &str, json: bool) -> Result<()> {
    let path = format!("/api/action/local?id={}", url_encode(id));
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
            "failed to remove local action (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    println!("Local Action Removed: {id}");
    Ok(())
}

async fn do_action_remote_list(ui_port: u16, peer: &str, refresh: bool, json: bool) -> Result<()> {
    if refresh {
        let refresh_path = format!("/api/action/remote/refresh?peer={}", url_encode(peer));
        let (rcode, rresp) = tetherly_node::loopback_post(ui_port, &refresh_path, &[])
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if rcode != 200 {
            anyhow::bail!(
                "failed to request remote actions refresh (HTTP {rcode}): {}",
                String::from_utf8_lossy(&rresp)
            );
        }
    }

    let path = format!("/api/action/remote?peer={}", url_encode(peer));
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
            "failed to query remote actions (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let val: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;

    println!("Remote Advertised Actions for '{peer}':");
    let actions = if let Some(arr) = val.get("actions").and_then(|a| a.as_array()) {
        arr.clone()
    } else if let Some(arr) = val.as_array() {
        arr.clone()
    } else {
        Vec::new()
    };

    if actions.is_empty() {
        println!("  (no advertised actions recorded yet; use --refresh to query peer)");
        return Ok(());
    }

    for act in actions {
        let aid = act["id"].as_str().unwrap_or("");
        let aname = act["name"].as_str().unwrap_or("");
        let adesc = act["description"].as_str().unwrap_or("-");
        let aicon = act["icon"].as_str().unwrap_or("-");
        println!("  - Action: [{aid}] \"{aname}\" (icon: {aicon})");
        if adesc != "-" {
            println!("    Description: {adesc}");
        }
    }
    Ok(())
}

async fn do_action_run(ui_port: u16, peer: &str, action_id: &str, json: bool) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "action_id": action_id,
    });
    let (code, resp) = tetherly_node::loopback_post(
        ui_port,
        "/api/action/trigger",
        &serde_json::to_vec(&payload)?,
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
            "failed to trigger remote action (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let trig_id = v["trigger_id"].as_str().unwrap_or("");
    let pid = v["peer_id"].as_str().unwrap_or(peer);
    let aid = v["action_id"].as_str().unwrap_or(action_id);
    let status = v["status"].as_str().unwrap_or("dispatched");

    println!("Action Trigger Dispatched:");
    println!("  Trigger ID: {trig_id}");
    println!("  Target:     {pid}");
    println!("  Action ID:  {aid}");
    println!("  Status:     {status}");
    println!("  (run `tetherly action watch` or check `tetherly action history` for result)");
    Ok(())
}

async fn do_action_history(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/action/history")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code != 200 {
        anyhow::bail!(
            "failed to query action history (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let val: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let history = if let Some(arr) = val.get("results").and_then(|r| r.as_array()) {
        arr.clone()
    } else if let Some(arr) = val.as_array() {
        arr.clone()
    } else {
        Vec::new()
    };

    println!("Action Execution History ({})", history.len());
    if history.is_empty() {
        println!("  (no action executions recorded)");
        return Ok(());
    }

    for item in history {
        let tid = item["trigger_id"].as_str().unwrap_or("");
        let aid = item["action_id"].as_str().unwrap_or("");
        let dir = item["direction"].as_str().unwrap_or("inbound");
        let pid = item["peer_id"].as_str().unwrap_or("");
        let alias = item["peer_alias"].as_str().unwrap_or("");
        let success = item["success"].as_bool().unwrap_or(false);
        let exit_code = item["exit_code"].as_i64();
        let dur = item["duration_ms"].as_u64().unwrap_or(0);
        let out = item["output_summary"].as_str().unwrap_or("");
        let err = item["error_message"].as_str().unwrap_or("");

        let status_str = if success { "SUCCESS" } else { "FAILED" };
        let exit_str = exit_code
            .map(|c| format!("exit: {c}"))
            .unwrap_or_else(|| "no exit code".to_string());

        println!("  - [{status_str}] ID: {tid} Action: {aid} ({dir})");
        println!("    Peer:     {alias} ({pid})");
        println!("    Metrics:  {dur}ms | {exit_str}");
        if !out.is_empty() {
            println!("    Output:   {out}");
        }
        if !err.is_empty() {
            println!("    Error:    {err}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_action_policy(
    ui_port: u16,
    action: Option<&str>,
    mode: Option<&str>,
    max_timeout_secs: Option<u32>,
    expose: Option<bool>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_mutation = action == Some("set")
        || mode.is_some()
        || max_timeout_secs.is_some()
        || expose.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || clear_filters;

    if is_mutation {
        let mut payload = serde_json::json!({});
        if let Some(m) = mode {
            payload["mode"] = serde_json::json!(m);
        }
        if let Some(t) = max_timeout_secs {
            payload["max_timeout_secs"] = serde_json::json!(t);
        }
        if let Some(exp) = expose {
            payload["expose_action_list"] = serde_json::json!(exp);
        }
        if let Some(grps) = group {
            payload["allowed_groups"] = serde_json::json!(grps);
        }
        if let Some(tags) = tag {
            payload["allowed_tags"] = serde_json::json!(tags);
        }
        if let Some(types) = device_type {
            payload["allowed_types"] = serde_json::json!(types);
        }
        if clear_filters {
            payload["clear_filters"] = serde_json::json!(true);
        }

        let (code, resp) = tetherly_node::loopback_post(
            ui_port,
            "/api/action/policy",
            &serde_json::to_vec(&payload)?,
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
                "failed to update action policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        println!("Action Policy Updated Successfully:");
        print_action_policy(&v);
        return Ok(());
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/action/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code != 200 {
        anyhow::bail!(
            "failed to query action policy (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    println!("Action Governance Policy:");
    print_action_policy(&v);
    Ok(())
}

fn print_action_policy(v: &serde_json::Value) {
    let pol = v.get("policy").unwrap_or(v);
    let mode = pol["mode"].as_str().unwrap_or("allow");
    let max_timeout = pol["max_timeout_secs"].as_u64().unwrap_or(30);
    let expose = pol["expose_action_list"].as_bool().unwrap_or(true);
    let grps = pol["allowed_groups"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tags = pol["allowed_tags"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let types = pol["allowed_types"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    println!("  Mode:                       {mode}");
    println!("  Max Timeout:                {max_timeout}s");
    println!("  Expose Action List:         {expose}");
    println!(
        "  Allowed Groups:             {}",
        if grps.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", grps.join(", "))
        }
    );
    println!(
        "  Allowed Tags:               {}",
        if tags.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Types:              {}",
        if types.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
}

async fn do_action_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!("Streaming live action events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "action_list_requested"
                || kind == "action_list_received"
                || kind == "action_triggered"
                || kind == "action_result_received"
                || kind == "action_policy_changed"
                || kind == "local_actions_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "action_list_requested" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let pid = v["peer_id"].as_str().unwrap_or("");
                            println!("[action_list_requested] peer={peer} ({pid}) requested available actions");
                        }
                        "action_list_received" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let pid = v["peer_id"].as_str().unwrap_or("");
                            let count = v["count"].as_u64().unwrap_or(0);
                            println!("[action_list_received] peer={peer} ({pid}) advertised {count} actions");
                        }
                        "action_triggered" => {
                            let tid = v["trigger_id"].as_str().unwrap_or("");
                            let aid = v["action_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let pid = v["peer_id"].as_str().unwrap_or("");
                            println!("[action_triggered] trigger_id={tid} action_id={aid} from peer={peer} ({pid})");
                        }
                        "action_result_received" => {
                            let tid = v["trigger_id"].as_str().unwrap_or("");
                            let aid = v["action_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let succ = v["success"].as_bool().unwrap_or(false);
                            let dur = v["duration_ms"].as_u64().unwrap_or(0);
                            let exit = v["exit_code"]
                                .as_i64()
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "none".to_string());
                            let status = if succ { "SUCCESS" } else { "FAILED" };
                            println!("[action_result_received] trigger_id={tid} action={aid} peer={peer} status={status} duration={dur}ms exit={exit}");
                        }
                        "action_policy_changed" => {
                            let mode = v["mode"].as_str().unwrap_or("");
                            let max_t = v["max_timeout_secs"].as_u64().unwrap_or(0);
                            println!("[action_policy_changed] mode={mode} max_timeout={max_t}s");
                        }
                        "local_actions_changed" => {
                            let count = v["count"].as_u64().unwrap_or(0);
                            println!("[local_actions_changed] local action definitions updated: {count} configured");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
