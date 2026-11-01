// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Clap surface: the top-level `Cli` and the `Cmd` tree.

use super::*;

#[derive(Parser)]
#[command(
    name = "tetherly",
    about = "Tetherly LAN node with optional EasyTier sidecar"
)]
pub(crate) struct Cli {
    #[arg(long)]
    pub(crate) data_dir: Option<PathBuf>,
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[command(subcommand)]
    pub(crate) cmd: Cmd,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Cmd {
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
        /// Outbound screen codec (ADR-079): `zstd` lossless (default) or
        /// `h264` lossy — h264 still falls back to zstd per peer when the
        /// other end did not advertise `screencodecs` containing h264.
        #[arg(long, default_value = "zstd")]
        screen_codec: String,
        /// H.264 encoder bitrate ceiling: `auto` (default) picks the tier
        /// from the peer's measured bandwidth profile (ADR-078, re-tiered
        /// hot on new benches, conservative 4 Mbps with no profile), or an
        /// explicit kbit/s number as the manual override.
        #[arg(long, default_value = "auto")]
        screen_bitrate: String,
        /// Screen streaming target frame rate (ADR-083): `auto` (default) picks
        /// target FPS from bandwidth tiers (15/24/30/60 FPS), or an explicit FPS
        /// number between 5 and 120 (clamped).
        #[arg(long, default_value = "auto")]
        screen_fps: String,
        /// Screen streaming target resolution scale percentage (ADR-084): `auto` (default)
        /// selects scale tier from bandwidth profile (50%/75%/100%), or an explicit
        /// scale percentage between 25 and 100 (clamped).
        #[arg(long, default_value = "auto")]
        screen_scale: String,
        /// Screen streaming content profile (ADR-085): `auto` (default) selects
        /// Text profile for desktop/IDE clarity, `text` enforces strict QP limits
        /// for code and text, `motion` optimizes for video and fluid animations.
        #[arg(long, default_value = "auto")]
        screen_profile: String,
        /// Screen streaming quantization parameter (QP) range (ADR-085), formatted as `min..max` (e.g. `12..35`).
        #[arg(long)]
        screen_qp: Option<String>,
        /// 4D Autonomous Adaptation Coordinator mode (ADR-086): `auto` (default)
        /// continuously coordinates bitrate, FPS, and scale; `conservative` preserves
        /// native scale; `manual` disables autonomous adaptations.
        #[arg(long, default_value = "auto")]
        screen_matrix: String,
        /// Screen Streaming Receiver Playout Buffer mode (ADR-087): `adaptive` (default)
        /// dynamically adapts target buffer delay based on EWMA jitter; `zerolatency` minimizes
        /// buffering latency for interactive tasks; `smooth` cushions jitter for video/animation.
        #[arg(long, default_value = "adaptive")]
        screen_playout: String,
        /// Screen Hardware Cursor Decoupling mode (ADR-088): `decoupled` (default)
        /// streams cursor position, shape, and visibility out-of-band at 125Hz;
        /// `embedded` renders cursor directly into video frames; `hidden` suppresses remote cursor.
        #[arg(long, default_value = "decoupled")]
        screen_cursor: String,
        /// Screen Static Frame Adaptive Squelch mode (ADR-089): `auto` (default)
        /// squelches video push after 500ms of static screen with 2.0s heartbeats;
        /// `aggressive` squelches after 150ms with 5.0s heartbeats; `disabled` pushes continuous frames.
        #[arg(long, default_value = "auto")]
        screen_squelch: String,
        /// Screen Forward Error Correction mode (ADR-090): `auto` (default)
        /// adapts Cauchy RS redundancy to loss; `conservative` (10%); `aggressive` (25%); `disabled` (0%).
        #[arg(long, default_value = "auto")]
        screen_fec: String,
        /// Screen Streaming Microsecond Packet Pacing and Burst Shaper mode (ADR-091):
        /// `auto` (default) burst 4 pkts, smooth over 80% frame interval;
        /// `aggressive` burst 2 pkts, smooth over 100% frame interval; `disabled` passthrough.
        #[arg(long, default_value = "auto")]
        screen_pacer: String,
        /// Screen Streaming Delay-Based Congestion Controller and Bufferbloat Trendline Filter mode (ADR-092):
        /// `auto` (default) dynamic RTT-scaled gamma; `aggressive` (6.0 us/frame); `conservative` (25.0 us/frame); `disabled`.
        #[arg(long, default_value = "auto")]
        screen_trendline: String,
        /// Screen Streaming Selective NACK and Hybrid ARQ (HARQ) pipeline mode (ADR-093):
        /// `auto` (default) up to 2 retries; `aggressive` (up to 3 retries); `conservative` (1 retry); `disabled`.
        #[arg(long, default_value = "auto")]
        screen_harq: String,
        /// Screen Streaming Transport-Wide Congestion Control (TWCC) feedback mode (ADR-094):
        /// `auto` (default, 10ms feedback / dynamic AIMD); `aggressive` (5ms feedback); `conservative` (25ms feedback); `disabled`.
        #[arg(long, default_value = "auto")]
        screen_twcc: String,
        /// Screen Streaming Active Bandwidth Probing & Fast Ramp-Up Pipeline mode (ADR-095):
        /// `auto` (default, 10-pkt burst cluster); `aggressive` (16-pkt burst cluster); `conservative` (6-pkt burst cluster); `disabled`.
        #[arg(long, default_value = "auto")]
        screen_probe: String,
        /// Screen & Audio Playout Lip-Sync & Clock Synchronization Pipeline mode (ADR-096):
        /// `auto` (default, ±15ms skew tolerance); `strict` (±5ms tolerance); `loose` (±40ms tolerance); `disabled`.
        #[arg(long, default_value = "auto")]
        screen_sync: String,
        /// Screen Streaming Microsecond Multi-Stage Latency Profiler mode (ADR-099):
        /// `auto` (default, periodic EWMA and percentiles); `detailed` (per-frame stage timings); `disabled`.
        #[arg(long, default_value = "auto")]
        screen_latency: String,
        /// Initial display ID for screen capture (ADR-097). Defaults to 0 (Primary).
        #[arg(long, default_value = "0")]
        screen_display: u8,
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
    /// Cross-device focus mode, Do-Not-Disturb synchronization, and quiet hours governance pipeline (v1.77 / ADR-070).
    #[command(alias = "dnd")]
    Focus {
        #[command(subcommand)]
        action: FocusCmd,
    },
    /// Cross-device virtual multi-display topology, borderless edge cursor crossing & spatial layout governance pipeline (v1.78 / ADR-071).
    #[command(alias = "topo")]
    Topology {
        #[command(subcommand)]
        action: TopologyCmd,
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
    /// Manage remote keyboard/mouse control and virtual peripheral input tunneling (v1.79 / ADR-072).
    Input {
        #[command(subcommand)]
        action: InputCmd,
    },
    /// Cross-device lightweight virtual secondary display, adaptive streaming & display governance pipeline (v1.80 / ADR-073).
    #[command(alias = "disp")]
    Display {
        #[command(subcommand)]
        action: DisplayCmd,
    },
    /// Cross-device lightweight spatial audio routing, low-latency streaming & acoustic policy governance pipeline (v1.81 / ADR-074).
    #[command(alias = "snd")]
    Audio {
        #[command(subcommand)]
        action: AudioCmd,
    },
    /// Cross-device lightweight virtual camera, low-latency video multiplexing & hardware privacy governance pipeline (v1.82 / ADR-075).
    #[command(alias = "cam")]
    Camera {
        #[command(subcommand)]
        action: CameraCmd,
    },
    /// Cross-device lightweight virtual microphone, push-to-talk intercom & acoustic privacy governance pipeline (v1.83 / ADR-076).
    #[command(alias = "microphone")]
    Mic {
        #[command(subcommand)]
        action: MicCmd,
    },
    /// Cross-device link throughput benchmark & bandwidth profiling pipeline (v1.84 / ADR-077).
    Bench {
        #[command(subcommand)]
        action: BenchCmd,
    },
    /// Bandwidth-profile-driven streaming parameter recommendation (v1.85 / ADR-078).
    Recommend {
        /// Show the per-peer detailed report for this device_id or alias.
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
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
