// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Subcommand enums for handoff/media/power/locate/action/focus/topology/clip.

use super::*;

#[derive(Subcommand, Debug)]
pub(crate) enum HandoffCmd {
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
pub(crate) enum MediaCmd {
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
pub(crate) enum PowerCmd {
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
pub(crate) enum LocateCmd {
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
pub(crate) enum ActionCmd {
    /// Manage locally registered quick actions and executable commands.
    Local {
        #[command(subcommand)]
        sub: ActionLocalSubCmd,
    },
    /// Query or list predefined actions exposed by a remote peer device.
    Remote {
        /// Peer ID or alias to inspect.
        #[arg(long, alias = "target")]
        peer: String,
        /// Force refreshing action list from peer over wire protocol.
        #[arg(long)]
        refresh: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Trigger execution of a predefined quick action on a remote peer device.
    #[command(alias = "trigger")]
    Run {
        /// Peer ID or alias on which to trigger the action.
        #[arg(long, alias = "target")]
        peer: String,
        /// Action identifier to trigger (e.g. "lock_screen", "mute_mic").
        #[arg(long, alias = "id")]
        action_id: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// View recent action execution results and execution history.
    History {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update action execution policy, admission mode, timeout ceilings, and peer filters.
    Policy {
        /// Policy subcommand: show or set.
        action: Option<String>,
        /// Access mode: allow, prompt_only, disabled.
        #[arg(long)]
        mode: Option<String>,
        /// Maximum execution timeout ceiling in seconds (1..=30).
        #[arg(long = "timeout", alias = "max-timeout")]
        max_timeout_secs: Option<u32>,
        /// Whether to expose local action list metadata to connected peers.
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
    /// Live watch action events (requested, received, triggered, result, policy changed) via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum FocusCmd {
    /// Turn on focus mode (e.g. dnd, work, meeting, sleep) with optional duration and reason.
    On {
        /// Focus mode: dnd, work, meeting, sleep (default: dnd or policy default).
        #[arg(long, default_value = "dnd")]
        mode: String,
        /// Duration in minutes before automatically expiring (1..=1440).
        #[arg(long, alias = "duration-mins", alias = "mins")]
        duration: Option<u32>,
        /// Optional reason or memo for entering focus mode (max 64 bytes).
        #[arg(long)]
        reason: Option<String>,
        /// Do not broadcast focus state to mesh peers.
        #[arg(long)]
        no_broadcast: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Turn off focus mode immediately.
    Off {
        /// Do not broadcast focus state to mesh peers.
        #[arg(long)]
        no_broadcast: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Check current local focus mode status, remaining countdown, and quiet hours state.
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// List known peer devices and their advertised focus states.
    Peers {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Request an immediate focus state synchronization from online mesh peers.
    Sync {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update focus mode governance policy, mirror settings, emergency breakthrough, and quiet hours schedule.
    Policy {
        /// Policy subcommand: show or set.
        action: Option<String>,
        /// Enable or disable focus mode policy entirely.
        #[arg(long)]
        enable: Option<bool>,
        /// Default focus mode when activated without specifying mode.
        #[arg(long)]
        default_mode: Option<String>,
        /// Mirror inbound active focus states from eligible peers.
        #[arg(long)]
        mirror: Option<bool>,
        /// Allow critical / OTP notifications to break through active focus mode.
        #[arg(long)]
        breakthrough: Option<bool>,
        /// Enable or disable automated quiet hours schedule.
        #[arg(long = "quiet-hours")]
        quiet_hours: Option<bool>,
        /// Quiet hours start time (HH:MM e.g. "23:00" or minute of day).
        #[arg(long)]
        start: Option<String>,
        /// Quiet hours end time (HH:MM e.g. "07:00" or minute of day).
        #[arg(long)]
        end: Option<String>,
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
    /// Live watch focus mode events (FocusStateChanged, PeerFocusStateChanged, FocusPolicyChanged) via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum TopologyCmd {
    /// Show current local screen geometry, active edge neighbors, and crossing parameters.
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Link a local display edge to a peer device.
    Link {
        /// Edge of the local display to link (left, right, top, bottom).
        #[arg(long)]
        edge: String,
        /// Peer device ID or alias to attach to this edge.
        #[arg(long)]
        peer: String,
        /// Edge on the remote display where the cursor will enter (defaults to opposite edge).
        #[arg(long)]
        peer_edge: Option<String>,
        /// Pixel offset alignment along the common edge boundary.
        #[arg(long)]
        offset: Option<i32>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Unlink a neighbor from a local display edge.
    Unlink {
        /// Edge of the local display to unlink (left, right, top, bottom).
        #[arg(long)]
        edge: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Update display geometry dimensions or edge crossing parameters.
    Config {
        /// Edge resistance in pixels required to initiate screen crossing.
        #[arg(long)]
        resistance: Option<u32>,
        /// Dwell delay in milliseconds required at the edge before crossing.
        #[arg(long)]
        delay: Option<u32>,
        /// Size in pixels of the anti-misclick exclusion zone in the four screen corners.
        #[arg(long)]
        corner_lock: Option<u32>,
        /// Enable or disable edge wrap-around crossing.
        #[arg(long)]
        wrap: Option<bool>,
        /// Override local display width in pixels.
        #[arg(long)]
        width: Option<u32>,
        /// Override local display height in pixels.
        #[arg(long)]
        height: Option<u32>,
        /// Override local display UI scale factor.
        #[arg(long)]
        scale: Option<f32>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// List cached peer display geometries and advertised topologies.
    Peers {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Trigger immediate multi-display spatial topology broadcast and sync with peers.
    Sync {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update topology coordination and cursor crossing policy.
    Policy {
        #[command(subcommand)]
        action: Option<TopologyPolicySubCmd>,
        /// Enable or disable topology coordination entirely.
        #[arg(long)]
        enable: Option<bool>,
        /// Allow inbound edge cursor crossing from remote peers.
        #[arg(long)]
        allow_inbound: Option<bool>,
        /// Automatically link paired mesh peers with reciprocal spatial layout.
        #[arg(long)]
        auto_link: Option<bool>,
        /// Add allowed group to filter.
        #[arg(long)]
        group: Option<Vec<String>>,
        /// Add allowed tag to filter.
        #[arg(long)]
        tag: Option<Vec<String>>,
        /// Add allowed device type to filter (e.g. desktop, laptop, tablet, phone).
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        /// Clear all group, tag, and type filters.
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch multi-display spatial topology and cursor crossing events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum TopologyPolicySubCmd {
    /// Show current topology governance policy.
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Update topology governance policy.
    Set {
        /// Enable or disable topology coordination entirely.
        #[arg(long)]
        enable: Option<bool>,
        /// Allow inbound edge cursor crossing from remote peers.
        #[arg(long)]
        allow_inbound: Option<bool>,
        /// Automatically link paired mesh peers with reciprocal spatial layout.
        #[arg(long)]
        auto_link: Option<bool>,
        /// Add allowed group to filter.
        #[arg(long)]
        group: Option<Vec<String>>,
        /// Add allowed tag to filter.
        #[arg(long)]
        tag: Option<Vec<String>>,
        /// Add allowed device type to filter (e.g. desktop, laptop, tablet, phone).
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        /// Clear all group, tag, and type filters.
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ActionLocalSubCmd {
    /// List all locally registered quick action definitions.
    List {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Register or update a local quick action definition.
    Add {
        /// Unique action identifier (e.g. "lock_screen", alphanumeric + underscores/hyphens).
        #[arg(long)]
        id: String,
        /// Human-readable action name (e.g. "Lock Screen").
        #[arg(long)]
        name: String,
        /// Local executable command path or binary (e.g. "loginctl", "pmset").
        #[arg(long, alias = "command")]
        cmd: String,
        /// Predefined arguments for the command (comma-separated, e.g. "displaysleepnow").
        #[arg(long, value_delimiter = ',')]
        args: Option<Vec<String>>,
        /// Optional description of the action.
        #[arg(long, alias = "description")]
        desc: Option<String>,
        /// Optional icon identifier or name (e.g. "lock", "moon").
        #[arg(long)]
        icon: Option<String>,
        /// Execution timeout ceiling in seconds (1..=30, default: 10).
        #[arg(long, default_value_t = 10)]
        timeout: u32,
        /// Whether the action is enabled for peer invocation (default: true).
        #[arg(long, default_value_t = true)]
        enabled: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Remove a registered local quick action by ID.
    Remove {
        /// Action identifier to remove.
        #[arg(long, alias = "action_id")]
        id: String,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ClipCmd {
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
