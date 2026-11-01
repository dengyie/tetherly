// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Subcommand enums for storage/inbox/link/audit/message/outbox/banner/profile/tag/group/route.

use super::*;

#[derive(Subcommand, Debug)]
pub(crate) enum StorageCmd {
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
pub(crate) enum InboxCmd {
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
pub(crate) enum LinkCmd {
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
pub(crate) enum AuditCmd {
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
pub(crate) enum MessageCmd {
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
pub(crate) enum OutboxCmd {
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
pub(crate) enum BannerCmd {
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
pub(crate) enum ProfileCmd {
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
pub(crate) enum TagCmd {
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
pub(crate) enum GroupCmd {
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
pub(crate) enum RouteCmd {
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
