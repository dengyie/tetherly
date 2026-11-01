// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `ScreenCmd` and every `screen` sub-subcommand enum.

use super::*;

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenCmd {
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
    /// Request an immediate on-demand keyframe (IDR) from the screen host (v1.95 / ADR-081).
    Keyframe {
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
    /// Inspect or override the dynamic screen streaming bitrate (v1.96 / ADR-082).
    Bitrate {
        /// Target bitrate in kbps (e.g. 2000 for 2 Mbps). If omitted, displays current bitrate and limits.
        kbps: Option<u32>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or override the dynamic screen streaming frame rate / FPS (v1.97 / ADR-083).
    Fps {
        /// Target FPS (e.g. 15, 30, 60). If omitted, displays current FPS and pacing interval.
        fps: Option<u32>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or override the dynamic screen streaming resolution scale percentage (v1.98 / ADR-084).
    Scale {
        /// Target scale percentage (25..=100, e.g. 50, 75, 100). If omitted, displays current scale and limits.
        scale: Option<u32>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or override dynamic screen streaming content profile and QP bounds (v1.99 / ADR-085).
    Profile {
        /// Target profile: `auto`, `text`, or `motion`. If omitted, displays current profile and QP range.
        profile: Option<String>,
        /// Optional minimum Quantization Parameter bound (0..=51).
        #[arg(long)]
        min_qp: Option<u8>,
        /// Optional maximum Quantization Parameter bound (0..=51).
        #[arg(long)]
        max_qp: Option<u8>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or override 4D autonomous adaptation matrix coordinator mode (v2.00 / ADR-086).
    Matrix {
        /// Target coordinator mode: `auto`, `conservative`, or `manual`. If omitted, displays current 4D matrix status.
        mode: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure screen streaming receiver playout buffer and adaptive dejitterizer (v2.01 / ADR-087).
    Playout {
        /// Target playout dejitter mode: `adaptive`, `zerolatency`, or `smooth`. If omitted, displays current playout buffer status.
        mode: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure screen streaming hardware cursor decoupling (v2.02 / ADR-088).
    Cursor {
        /// Target cursor mode: `decoupled`, `embedded`, or `hidden`. If omitted, displays current cursor status.
        mode: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure screen streaming damage detection and static frame adaptive squelch (v2.03 / ADR-089).
    Squelch {
        /// Target squelch mode: `auto`, `aggressive`, or `disabled`. If omitted, displays current squelch status.
        mode: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure screen streaming forward error correction (FEC) and adaptive redundancy (v2.04 / ADR-090).
    Fec {
        /// Target FEC mode: `auto`, `conservative`, `aggressive`, or `disabled`. If omitted, displays current FEC status.
        mode: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure screen streaming microsecond packet pacing and burst shaper (v2.05 / ADR-091).
    Pacer {
        /// Target pacer mode: `auto`, `aggressive`, or `disabled`. If omitted, displays current pacer status.
        mode: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure screen streaming delay-based congestion controller & trendline filter (v2.06 / ADR-092).
    Trendline {
        /// Target trendline mode: `auto`, `aggressive`, `conservative`, or `disabled`. If omitted, displays current trendline status.
        mode: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure screen streaming selective NACK and Hybrid ARQ (HARQ) pipeline (v2.07 / ADR-093).
    Harq {
        /// Target HARQ mode: `auto`, `aggressive`, `conservative`, or `disabled`. If omitted, displays current HARQ status.
        mode: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure screen streaming receiver-driven transport-wide congestion control (TWCC) feedback pipeline (v2.08 / ADR-094).
    Twcc {
        /// Target TWCC mode: `auto`, `aggressive`, `conservative`, or `disabled`. If omitted, displays current TWCC status.
        mode: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect, configure, or manually trigger screen active bandwidth probing and fast ramp-up (v2.09 / ADR-095).
    Probe {
        /// Target probe mode: `auto`, `aggressive`, `conservative`, or `disabled`. If omitted, displays current probe status.
        mode: Option<String>,
        /// Target bandwidth in kbps to probe (e.g. 15000 for 15 Mbps). Triggers immediate probe cluster.
        #[arg(long)]
        target_kbps: Option<u32>,
        /// Manually trigger probe cluster immediately (using default or next ladder step).
        #[arg(long)]
        trigger: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect, configure, or manually trigger screen & audio playout lip-sync & clock synchronization pipeline (v2.10 / ADR-096).
    Sync {
        /// Target sync mode: `auto`, `strict`, `loose`, or `disabled`. If omitted, displays current sync status.
        mode: Option<String>,
        /// Manually trigger immediate sync beacon transmission.
        #[arg(long)]
        trigger: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// List enumerated displays available for screen streaming (v2.11 / ADR-097).
    Displays {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Dynamically switch active display during screen streaming session (v2.11 / ADR-097).
    Switch {
        /// Target display ID to switch to (e.g. 0 for primary, 1 for secondary).
        display_id: u8,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure screen streaming privacy mode, display blanking & host input lockout (v2.12 / ADR-098).
    Privacy {
        #[command(subcommand)]
        action: Option<ScreenPrivacySubCmd>,
        /// Enable privacy mode (curtain blanking).
        #[arg(long)]
        enable: bool,
        /// Disable privacy mode.
        #[arg(long)]
        disable: bool,
        /// Lock physical keyboard/mouse on host machine while privacy mode is active.
        #[arg(long)]
        lock_input: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure screen streaming microsecond latency breakdown, profiler mode & stage telemetry (v2.13 / ADR-099).
    Latency {
        #[command(subcommand)]
        action: Option<ScreenLatencySubCmd>,
        /// Configure profiler mode: 'auto', 'always', or 'disabled'.
        #[arg(long)]
        mode: Option<String>,
        /// Reset profiler telemetry history and ring buffer counters.
        #[arg(long)]
        reset: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure the bottleneck-driven autonomous tuning closed loop (v2.14 / ADR-100).
    Tuning {
        #[command(subcommand)]
        action: Option<ScreenTuningSubCmd>,
        /// Configure tuner mode: 'standard', 'aggressive', or 'disabled'.
        #[arg(long)]
        mode: Option<String>,
        /// Reset tuning counters, hysteresis windows and the recovery stack.
        #[arg(long)]
        reset: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Start/stop/inspect the screen session recorder (v2.15 / ADR-101).
    Recording {
        #[command(subcommand)]
        action: Option<ScreenRecordingSubCmd>,
        /// Output path for the TREC container (default: <data_dir>/recordings/rec-<ts>.trec).
        #[arg(long)]
        out: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Control in-container screen session playback from a recorded TREC container (v2.16 / ADR-102).
    Playback {
        #[command(subcommand)]
        action: Option<ScreenPlaybackSubCmd>,
        /// Path to the .trec container file to play.
        #[arg(long)]
        path: Option<String>,
        /// Playback speed multiplier (e.g. 1.0 = real-time, 0.0 = zero wait test mode).
        #[arg(long)]
        speed: Option<f32>,
        /// Automatically rewind and loop when end of recording is reached.
        #[arg(long)]
        r#loop: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Capture a live screen snapshot as standard BMP image or raw data (v2.17 / ADR-103).
    Snapshot {
        /// Optional output file path for the captured snapshot (e.g. screenshot.bmp).
        #[arg(long)]
        out: Option<String>,
        /// Snapshot format: 'bmp' (standard 32-bit Windows DIB) or 'raw' (default: bmp).
        #[arg(long, default_value = "bmp")]
        format: String,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Dynamic security watermark and forensic attribution overlay (v2.18 / ADR-104).
    Watermark {
        #[command(subcommand)]
        action: Option<ScreenWatermarkSubCmd>,
        /// Text template with dynamic placeholders ({peer}, {host}, {time}).
        #[arg(long)]
        text: Option<String>,
        /// Host-enforced lockdown: visitors cannot disable watermark overlays.
        #[arg(long)]
        host_enforced: bool,
        /// Watermark opacity alpha (0..=255, default: 64 / 25%).
        #[arg(long)]
        opacity: Option<u8>,
        /// Watermark text color in hex format (e.g. #DCDCDC, #FF8000).
        #[arg(long)]
        color: Option<String>,
        /// Font scaling multiplier (1 or 2).
        #[arg(long)]
        scale: Option<u8>,
        /// Layout pattern: 'grid' (tiled), 'single' (centered), or 'banner'.
        #[arg(long)]
        pattern: Option<String>,
        /// Horizontal spacing between tiles in pixels (min: 40, default: 240).
        #[arg(long)]
        spacing_x: Option<u16>,
        /// Vertical spacing between tiles in pixels (min: 30, default: 160).
        #[arg(long)]
        spacing_y: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Collaborative whiteboard annotation and geometric overlays (v2.19 / ADR-105).
    Annotate {
        #[command(subcommand)]
        action: Option<ScreenAnnotateSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Transient virtual laser pointer spotlight (v2.19 / ADR-105).
    Laser {
        /// X pixel coordinate.
        #[arg(value_name = "X")]
        x: Option<u16>,
        /// Y pixel coordinate.
        #[arg(value_name = "Y")]
        y: Option<u16>,
        /// Laser pointer color in hex format (e.g. #FF0000).
        #[arg(long)]
        color: Option<String>,
        /// Laser spotlight radius in pixels (default: 12).
        #[arg(long)]
        radius: Option<u8>,
        /// Turn laser pointer off.
        #[arg(long)]
        off: bool,
        /// Show laser status only.
        #[arg(long)]
        status: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Multi-party collaborative shared whiteboard synchronization, chronological undo/redo & snapshot import/export (v2.20 / ADR-106).
    Whiteboard {
        #[command(subcommand)]
        action: Option<ScreenWhiteboardSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Dynamic Region-of-Interest (ROI) viewport cropping, sub-rectangle zoom & pan (v2.21 / ADR-107).
    Roi {
        #[command(subcommand)]
        action: Option<ScreenRoiSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Adaptive color depth, grayscale economy & dynamic monochrome binarization pipeline (v2.22 / ADR-108).
    Color {
        #[command(subcommand)]
        action: Option<ScreenColorSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Dynamic differential delta patch, inter-frame temporal compression & macroblock cache pipeline (v2.23 / ADR-109).
    Delta {
        #[command(subcommand)]
        action: Option<ScreenDeltaSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Real-time objective visual quality metric, PSNR/SSIM distortion profiling & QA pipeline (v2.24 / ADR-110).
    Metric {
        #[command(subcommand)]
        action: Option<ScreenMetricSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Autonomous closed-loop QoE adaptation, perceptual visual quality stream coordination pipeline (v2.25 / ADR-111).
    Qoe {
        #[command(subcommand)]
        action: Option<ScreenQoeSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Progressive Intra-Refresh (PIR), anti-burst band slice smoothing stream pipeline (v2.26 / ADR-112).
    Pir {
        #[command(subcommand)]
        action: Option<ScreenPirSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Long-Term Reference (LTR) frame anchoring, gap recovery, and reference picture selection pipeline (v2.27 / ADR-113).
    Ltr {
        #[command(subcommand)]
        action: Option<ScreenLtrSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// 2-Layer Temporal Scalability (SVC-T) hierarchical reference, leaf-layer degradation, and congestion resilience pipeline (v2.28 / ADR-114).
    Svc {
        #[command(subcommand)]
        action: Option<ScreenSvcSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// 2D Motion Vector Estimation, Displacement Compensation and Scroll Acceleration Pipeline (v2.29 / ADR-115).
    Motion {
        #[command(subcommand)]
        action: Option<ScreenMotionSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Spatial Tile-Grid Decomposition and Independent Asynchronous Tile Streaming Pipeline (v2.30 / ADR-116).
    Tile {
        #[command(subcommand)]
        action: Option<ScreenTileSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Screen Streaming Hash-Based Tile Deduplication & LRU Cache Pipeline (v2.31 / ADR-117).
    TileCache {
        #[command(subcommand)]
        action: Option<ScreenTileCacheSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Screen Streaming Foveated Focus Awareness & Spatial Region-Priority Scheduling Pipeline (v2.32 / ADR-118).
    Focus {
        #[command(subcommand)]
        action: Option<ScreenFocusSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Screen Streaming Tile-Level Adaptive Palette & Hybrid Lossless Compression Pipeline (v2.33 / ADR-119).
    TilePalette {
        #[command(subcommand)]
        action: Option<ScreenTilePaletteSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Screen Streaming Tile-Level Predictive Filter & Reversible Color Transform Pipeline (v2.34 / ADR-120).
    TilePredict {
        #[command(subcommand)]
        action: Option<ScreenTilePredictSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage tile-level predictive filter & reversible color transform pipeline (alias for screen tile predict) (v2.34 / ADR-120).
    Predict {
        #[command(subcommand)]
        action: Option<ScreenTilePredictSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Screen Streaming Tile Presentation Transaction & Anti-Tearing Pipeline (v2.35 / ADR-121).
    TilePresent {
        #[command(subcommand)]
        action: Option<ScreenTilePresentSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage tile presentation transaction and double-buffered swapping (alias for screen tile present) (v2.35 / ADR-121).
    Present {
        #[command(subcommand)]
        action: Option<ScreenTilePresentSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Screen Streaming Tile Fast Retransmission & Micro-NACK Pipeline (v2.36 / ADR-122).
    TileNack {
        #[command(subcommand)]
        action: Option<ScreenTileNackSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage tile fast retransmission and micro-NACK self-healing pipeline (alias for screen tile nack) (v2.36 / ADR-122).
    Nack {
        #[command(subcommand)]
        action: Option<ScreenTileNackSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Screen Streaming Dynamic Tile Saliency & Priority Scheduling Pipeline (v2.37 / ADR-123).
    TileSaliency {
        #[command(subcommand)]
        action: Option<ScreenTileSaliencySubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage dynamic tile saliency & priority scheduling (alias for screen tile saliency) (v2.37 / ADR-123).
    Saliency {
        #[command(subcommand)]
        action: Option<ScreenTileSaliencySubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Screen Streaming Dynamic Tile Level-of-Detail Downsampling & Progressive Reconstruction Pipeline (v2.38 / ADR-124).
    TileLod {
        #[command(subcommand)]
        action: Option<ScreenTileLodSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Screen Streaming Dynamic Tile Batching, MTU Chunking & Coalesced Pacing Pipeline (v2.39 / ADR-125).
    TileBatch {
        #[command(subcommand)]
        action: Option<ScreenTileBatchSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Screen Streaming Dynamic Tile Local Motion Vector Estimation & Inter-Tile Displacement Compensation Pipeline (v2.40 / ADR-126).
    TileMotion {
        #[command(subcommand)]
        action: Option<ScreenTileMotionSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage dynamic tile batching and MTU chunking pipeline (alias for screen tile batch) (v2.39 / ADR-125).
    Batch {
        #[command(subcommand)]
        action: Option<ScreenTileBatchSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage dynamic tile Level-of-Detail downsampling (alias for screen tile lod) (v2.38 / ADR-124).
    Lod {
        #[command(subcommand)]
        action: Option<ScreenTileLodSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTileSubCmd {
    /// Show current screen spatial tile-grid mode and streaming statistics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen spatial tile-grid decomposition mode and tile size.
    Set {
        /// Enable spatial tile-grid streaming (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Tile grid dimension size in pixels (32..=256, default 64).
        #[arg(long)]
        tile_size: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen spatial tile-grid streaming back to default disabled state and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage tile dictionary cache (status, set, reset) (v2.31 / ADR-117).
    Cache {
        #[command(subcommand)]
        action: Option<ScreenTileCacheSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage foveated focus awareness and spatial priority scheduling (v2.32 / ADR-118).
    Focus {
        #[command(subcommand)]
        action: Option<ScreenFocusSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage tile-level adaptive palette compression (v2.33 / ADR-119).
    Palette {
        #[command(subcommand)]
        action: Option<ScreenTilePaletteSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage tile-level predictive filter & reversible color transform pipeline (v2.34 / ADR-120).
    Predict {
        #[command(subcommand)]
        action: Option<ScreenTilePredictSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage tile presentation transaction and double-buffered swapping (v2.35 / ADR-121).
    Present {
        #[command(subcommand)]
        action: Option<ScreenTilePresentSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage tile fast retransmission and micro-NACK self-healing pipeline (v2.36 / ADR-122).
    Nack {
        #[command(subcommand)]
        action: Option<ScreenTileNackSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage dynamic tile saliency awareness & priority scheduling (v2.37 / ADR-123).
    Saliency {
        #[command(subcommand)]
        action: Option<ScreenTileSaliencySubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage dynamic tile Level-of-Detail downsampling & reconstruction pipeline (v2.38 / ADR-124).
    Lod {
        #[command(subcommand)]
        action: Option<ScreenTileLodSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage dynamic tile batching, MTU chunking and pacing pipeline (v2.39 / ADR-125).
    Batch {
        #[command(subcommand)]
        action: Option<ScreenTileBatchSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manage dynamic tile local motion vector estimation & inter-tile displacement compensation pipeline (v2.40 / ADR-126).
    Motion {
        #[command(subcommand)]
        action: Option<ScreenTileMotionSubCmd>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTileMotionSubCmd {
    /// Show current screen dynamic tile local motion vector estimation & displacement compensation status and metrics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen dynamic tile local motion vector estimation, search displacement, and match ratio.
    Set {
        /// Enable dynamic tile local motion vector estimation (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Maximum local motion search window displacement in pixels (4..=128, default 32).
        #[arg(long)]
        max_displacement: Option<u16>,
        /// Minimum pixel match ratio threshold percentage (50..=100, default 90).
        #[arg(long)]
        min_match_ratio: Option<u8>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen dynamic tile local motion vector estimation configuration back to default and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTileBatchSubCmd {
    /// Show current screen dynamic tile batching & MTU chunking status and metrics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen dynamic tile batching, target chunk bytes, and pacing interval.
    Set {
        /// Enable dynamic tile batching and MTU chunking (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Target maximum chunk payload bytes (512..=64000, default 1200).
        #[arg(long)]
        target_chunk_bytes: Option<u16>,
        /// Pacing interval in microseconds between chunk transmissions (0..=50000, default 200).
        #[arg(long)]
        pacing_interval_us: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen dynamic tile batching configuration back to default and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTileLodSubCmd {
    /// Show current screen tile Level-of-Detail downsampling status and metrics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen tile Level-of-Detail downsampling mode and policies.
    Set {
        /// Enable tile Level-of-Detail downsampling (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Maximum downsampling LoD level: 1 (Lod1, 32x32) or 2 (Lod2, 16x16).
        #[arg(long)]
        max_lod: Option<u8>,
        /// Automatically downsample low-priority tiles during network congestion (true/false, default true).
        #[arg(long)]
        auto_lod: Option<bool>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen tile Level-of-Detail mode to default disabled state and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTileSaliencySubCmd {
    /// Show current screen tile saliency awareness & tiered priority scheduling status and metrics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen tile saliency thresholds and congestion dropping policy.
    Set {
        /// Enable tile saliency awareness & tiered priority scheduling (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// High saliency threshold for Critical tier (0..=255, default 120).
        #[arg(long)]
        high_threshold: Option<u8>,
        /// Low saliency threshold for Normal tier (0..=255, default 40).
        #[arg(long)]
        low_threshold: Option<u8>,
        /// Drop low-saliency background tiles during network congestion (true/false, default true).
        #[arg(long)]
        drop_background: Option<bool>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen tile saliency mode to default disabled state and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTileNackSubCmd {
    /// Show current screen tile micro-NACK fast retransmission status and metrics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen tile micro-NACK fast retransmission and self-healing parameters.
    Set {
        /// Enable tile micro-NACK fast retransmission (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Retransmission timeout window in milliseconds (default 20ms).
        #[arg(long)]
        timeout_ms: Option<u16>,
        /// Maximum retransmission retries before degraded commit (default 2).
        #[arg(long)]
        max_retries: Option<u8>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen tile micro-NACK mode to default disabled state and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTilePresentSubCmd {
    /// Show current screen tile presentation transaction, double-buffer & anti-tearing status and metrics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen tile presentation transaction and anti-tearing v-sync mode.
    Set {
        /// Enable tile presentation transaction pipeline (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Vertical sync pacing mode (off, vsync60, vsync120, adaptive / 0..=3).
        #[arg(long)]
        vsync_mode: Option<String>,
        /// Enable double-buffered back-staging (true/false).
        #[arg(long)]
        double_buffer: Option<bool>,
        /// Enable anti-tearing multi-tile swap protection (true/false).
        #[arg(long)]
        tear_prevention: Option<bool>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen tile presentation mode to default disabled state and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTilePredictSubCmd {
    /// Show current screen tile-level predictive filter & RCT status and metrics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen tile-level predictive filter and reversible color transform mode.
    Set {
        /// Enable tile-level predictive filter pipeline (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Directional spatial filter mode (none, sub, up, average, paeth, auto / 0..=5).
        #[arg(long)]
        filter_mode: Option<String>,
        /// Enable Reversible Color Transform (RCT) for inter-channel decorrelation (true/false).
        #[arg(long)]
        enable_rct: Option<bool>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen tile-level predictive filter & RCT to default disabled state and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTilePaletteSubCmd {
    /// Show current screen tile-level adaptive palette compression status and metrics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen tile-level adaptive palette compression mode and parameters.
    Set {
        /// Enable tile-level adaptive palette compression (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Maximum distinct colors for palette mode (2..=16, default 16).
        #[arg(long)]
        max_colors: Option<u8>,
        /// Enable run-length encoding for bi-level and palette bitmaps.
        #[arg(long)]
        enable_rle: Option<bool>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen tile-level adaptive palette compression to default disabled state and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenFocusSubCmd {
    /// Show current screen foveated focus mode, radius, decimation ratio, and streaming metrics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen foveated focus awareness mode, radius, and peripheral decimation skip ratio.
    Set {
        /// Enable foveated focus scheduling (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Focus tracking mode: 'cursor' (hardware mouse) or 'box' (static rectangle).
        #[arg(long)]
        mode: Option<String>,
        /// Focus zone radius in pixels (64..=2048, default 256).
        #[arg(long)]
        radius: Option<u16>,
        /// Peripheral decimation skip ratio (0..=10, default 2: update every 3 frames).
        #[arg(long)]
        skip_ratio: Option<u8>,
        /// Static focus box X offset (Box mode).
        #[arg(long)]
        box_x: Option<u16>,
        /// Static focus box Y offset (Box mode).
        #[arg(long)]
        box_y: Option<u16>,
        /// Static focus box width (Box mode).
        #[arg(long)]
        box_w: Option<u16>,
        /// Static focus box height (Box mode).
        #[arg(long)]
        box_h: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen foveated focus streaming back to default disabled state and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTileCacheSubCmd {
    /// Show current screen tile dictionary cache mode, hit ratio, and bandwidth savings.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen tile dictionary cache mode and capacity.
    Set {
        /// Enable tile dictionary cache (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Maximum cache dictionary entries (64..=4096, default 512).
        #[arg(long)]
        max_entries: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen tile dictionary cache back to default disabled state and purge entries.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenMotionSubCmd {
    /// Show current screen 2D motion estimation mode and scroll acceleration statistics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen 2D motion estimation mode and maximum displacement search radius.
    Set {
        /// Enable motion estimation acceleration (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Maximum displacement search radius in pixels (8..=256, default 64).
        #[arg(long)]
        max_displacement: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen 2D motion estimation back to default disabled state and reset metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenSvcSubCmd {
    /// Show current screen Temporal Scalability (SVC-T) configuration, mode, and frame statistics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen Temporal Scalability (SVC-T) mode and T1 frame drop policy.
    Set {
        /// SVC operation mode: disabled, adaptive, or base_only.
        #[arg(long)]
        mode: Option<String>,
        /// Explicitly drop enhancement layer T1 frames (true/false).
        #[arg(long)]
        drop_t1: Option<bool>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Toggle or set enhancement layer T1 frame dropping for instantaneous congestion relief.
    DropT1 {
        /// Drop T1 frames flag (true/false, default true).
        #[arg(long)]
        drop: Option<bool>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen SVC configuration back to default disabled state and reset telemetry.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenLtrSubCmd {
    /// Show current screen Long-Term Reference (LTR) configuration and metrics.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen Long-Term Reference (LTR) mode and auto-anchor interval.
    Set {
        /// Enable or disable LTR frame anchoring pipeline (true/false).
        #[arg(long)]
        enabled: Option<bool>,
        /// Periodic auto-anchor interval in frames (5..=600, default 60).
        #[arg(long)]
        interval: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Manually trigger an explicit golden reference frame anchor snapshot.
    Anchor {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Trigger an instant LTR differential recovery request against current golden reference.
    Recover {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen LTR configuration back to default disabled state and purge golden reference canvases.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenPirSubCmd {
    /// Show current screen Progressive Intra-Refresh (PIR) mode, band count and slice telemetry.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen Progressive Intra-Refresh (PIR) mode and band slices.
    Set {
        /// Operating mode (disabled, continuous, on-demand).
        #[arg(long)]
        mode: String,
        /// Number of horizontal band slices (2..=32, default 8).
        #[arg(long)]
        bands: Option<u8>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen Progressive Intra-Refresh (PIR) back to default disabled state and clear sweep progress.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenQoeSubCmd {
    /// Show current screen autonomous QoE closed-loop adaptation mode, targets and action decisions.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen autonomous QoE closed-loop policy mode and target distortion thresholds.
    Set {
        /// Policy mode (conservative, balanced, economy, off).
        #[arg(long)]
        mode: String,
        /// Target PSNR threshold in dB (e.g. 30 dB).
        #[arg(long)]
        target_psnr: Option<u8>,
        /// Target SSIM threshold in percentage (e.g. 85 for 0.85).
        #[arg(long)]
        target_ssim: Option<u8>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen autonomous QoE closed-loop adaptation back to default disabled state and clear history.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenMetricSubCmd {
    /// Show current screen quality metric mode, sample interval and PSNR/SSIM telemetry.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen objective quality metric mode and sample interval.
    Set {
        /// Enable or disable real-time objective visual quality metric profiling.
        #[arg(long)]
        enabled: bool,
        /// Sampling frame interval for distortion computation (1..120, default 10).
        #[arg(long)]
        interval: Option<u8>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen quality metric pipeline back to default disabled state and clear metrics.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenDeltaSubCmd {
    /// Show current screen delta patch mode, keyframe interval and bandwidth savings telemetry.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure screen differential delta patch mode and keyframe interval.
    Set {
        /// Enable or disable dynamic differential delta patch mode.
        #[arg(long)]
        enabled: bool,
        /// Keyframe refresh interval in frames (1..600, default 60).
        #[arg(long)]
        interval: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen delta patch pipeline back to default full-frame transmission.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenColorSubCmd {
    /// Show current screen adaptive color depth mode, threshold and telemetry.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Set screen adaptive color mode and optional binarization threshold.
    Set {
        /// Color depth mode: full (32-bit), reduced (16-bit RGB565), grayscale (8-bit), monochrome (1-bit B&W).
        #[arg(long)]
        mode: String,
        /// Optional binarization threshold for monochrome mode (1..254, default 128).
        #[arg(long)]
        threshold: Option<u8>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset screen color mode back to native 32-bit full true color.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenRoiSubCmd {
    /// Show current Screen ROI viewport status, dimensions and zoom scale.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Set dynamic Region-of-Interest viewport bounds and optional zoom scale.
    Set {
        /// X coordinate of top-left viewport corner.
        #[arg(long)]
        x: u16,
        /// Y coordinate of top-left viewport corner.
        #[arg(long)]
        y: u16,
        /// Width of viewport in pixels.
        #[arg(long)]
        w: u16,
        /// Height of viewport in pixels.
        #[arg(long)]
        h: u16,
        /// Optional zoom percentage (50%~800%).
        #[arg(long)]
        zoom: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset Region-of-Interest to full screen capture and streaming.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Dynamically adjust Region-of-Interest zoom percentage (50%~800%).
    Zoom {
        /// Zoom percentage (e.g. 100, 150, 200).
        percent: u16,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenWhiteboardSubCmd {
    /// Show current whiteboard status, active item count and policy.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Chronologically undo the most recent whiteboard annotation (optionally targeted by author).
    Undo {
        /// Optional author filter (e.g. "alice"). If omitted, undos the globally latest annotation.
        #[arg(long)]
        author: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Chronologically redo the most recently undone annotation (optionally targeted by author).
    Redo {
        /// Optional author filter (e.g. "alice"). If omitted, redos the globally latest undone item.
        #[arg(long)]
        author: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Export active whiteboard annotations to a structured JSON snapshot file or stdout.
    Export {
        /// Optional destination file path to save snapshot (.twhiteboard or .json).
        #[arg(long)]
        out: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Import whiteboard annotations from a snapshot file.
    Import {
        /// Source file path (.twhiteboard or .json).
        path: String,
        /// Replace active canvas items instead of appending.
        #[arg(long)]
        replace: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or configure host whiteboard moderation policy (read-only, clear permissions, max items).
    Policy {
        /// Set read-only policy for guests/peers (true/false).
        #[arg(long)]
        read_only: Option<bool>,
        /// Allow peers/guests to clear the whiteboard canvas (true/false).
        #[arg(long)]
        allow_clear: Option<bool>,
        /// Maximum number of active whiteboard items allowed.
        #[arg(long)]
        max_items: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Clear all active annotations from the whiteboard canvas.
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
pub(crate) enum ScreenAnnotateSubCmd {
    /// List all active screen whiteboard annotations.
    List {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Add a new annotation (stroke, rect, arrow, text).
    Add {
        #[command(subcommand)]
        shape: ScreenAnnotateShapeSubCmd,
    },
    /// Clear all active screen whiteboard annotations.
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
pub(crate) enum ScreenAnnotateShapeSubCmd {
    /// Freehand polyline stroke: --points x1,y1;x2,y2;...
    Stroke {
        /// Points in x1,y1;x2,y2;... format.
        #[arg(long)]
        points: String,
        /// Author / collaborator identifier (e.g. "alice").
        #[arg(long)]
        author: Option<String>,
        /// Stroke color in hex format (default: #FF0000).
        #[arg(long)]
        color: Option<String>,
        /// Stroke thickness in pixels (default: 3).
        #[arg(long)]
        thickness: Option<u8>,
        /// Alpha opacity (0..=255, default: 255).
        #[arg(long)]
        alpha: Option<u8>,
        /// Auto-fadeout TTL in milliseconds (0: persistent).
        #[arg(long)]
        ttl: Option<u32>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Rectangle bounding box: --x <x> --y <y> --w <w> --h <h>
    Rect {
        #[arg(long)]
        x: u16,
        #[arg(long)]
        y: u16,
        #[arg(long)]
        w: u16,
        #[arg(long)]
        h: u16,
        /// Author / collaborator identifier (e.g. "alice").
        #[arg(long)]
        author: Option<String>,
        /// Box border color in hex format (default: #00FF00).
        #[arg(long)]
        color: Option<String>,
        /// Border thickness in pixels (default: 2).
        #[arg(long)]
        thickness: Option<u8>,
        /// Alpha opacity (0..=255, default: 255).
        #[arg(long)]
        alpha: Option<u8>,
        /// Auto-fadeout TTL in milliseconds (0: persistent).
        #[arg(long)]
        ttl: Option<u32>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Guiding arrow pointer: --from x1,y1 --to x2,y2
    Arrow {
        /// Origin coordinate in x,y format.
        #[arg(long)]
        from: String,
        /// Arrow target coordinate in x,y format.
        #[arg(long)]
        to: String,
        /// Author / collaborator identifier (e.g. "alice").
        #[arg(long)]
        author: Option<String>,
        /// Arrow color in hex format (default: #FFCC00).
        #[arg(long)]
        color: Option<String>,
        /// Shaft thickness in pixels (default: 3).
        #[arg(long)]
        thickness: Option<u8>,
        /// Alpha opacity (0..=255, default: 255).
        #[arg(long)]
        alpha: Option<u8>,
        /// Auto-fadeout TTL in milliseconds (0: persistent).
        #[arg(long)]
        ttl: Option<u32>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Text note overlay: <text> --x <x> --y <y> [--scale 1|2]
    Text {
        /// Text message to render.
        #[arg(value_name = "TEXT")]
        text: String,
        #[arg(long)]
        x: u16,
        #[arg(long)]
        y: u16,
        /// Author / collaborator identifier (e.g. "alice").
        #[arg(long)]
        author: Option<String>,
        /// Text scale factor (1: 8x8 px, 2: 16x16 px, default: 1).
        #[arg(long)]
        scale: Option<u8>,
        /// Text color in hex format (default: #FFFFFF).
        #[arg(long)]
        color: Option<String>,
        /// Alpha opacity (0..=255, default: 255).
        #[arg(long)]
        alpha: Option<u8>,
        /// Auto-fadeout TTL in milliseconds (0: persistent).
        #[arg(long)]
        ttl: Option<u32>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenWatermarkSubCmd {
    /// Show current watermark configuration and telemetry status.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Enable dynamic security watermark overlay.
    Enable {
        /// Enforce watermark overlay: visitor cannot disable it.
        #[arg(long)]
        host_enforced: bool,
        /// Text template with placeholders ({peer}, {host}, {time}).
        #[arg(long)]
        text: Option<String>,
        /// Watermark opacity (0..=255).
        #[arg(long)]
        opacity: Option<u8>,
        /// Layout pattern ('grid', 'single', 'banner').
        #[arg(long)]
        pattern: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Disable dynamic security watermark overlay (blocked if host-enforced).
    Disable {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Configure watermark parameters (text, opacity, color, scale, pattern, spacing).
    Set {
        /// Text template with placeholders ({peer}, {host}, {time}).
        #[arg(long)]
        text: Option<String>,
        /// Watermark opacity alpha (0..=255).
        #[arg(long)]
        opacity: Option<u8>,
        /// Watermark hex color (e.g. #FF8000).
        #[arg(long)]
        color: Option<String>,
        /// Font scaling multiplier (1 or 2).
        #[arg(long)]
        scale: Option<u8>,
        /// Layout pattern ('grid', 'single', 'banner').
        #[arg(long)]
        pattern: Option<String>,
        /// Horizontal tile spacing in pixels.
        #[arg(long)]
        spacing_x: Option<u16>,
        /// Vertical tile spacing in pixels.
        #[arg(long)]
        spacing_y: Option<u16>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenPlaybackSubCmd {
    /// Start in-container playback from a recorded TREC container.
    Start {
        /// Path to the .trec container file to play.
        path: String,
        /// Playback speed multiplier (default: 1.0, 0.0 for max test throughput).
        #[arg(long)]
        speed: Option<f32>,
        /// Loop continuously when reaching the end of the recording.
        #[arg(long)]
        r#loop: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Pause the active playback session.
    Pause {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Resume the paused playback session.
    Resume {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stop the active playback session.
    Stop {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Show current playback status and statistics.
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
pub(crate) enum ScreenRecordingSubCmd {
    /// Start recording the locally presented screen stream into a TREC container.
    Start {
        /// Output path for the TREC container (default: <data_dir>/recordings/rec-<ts>.trec).
        #[arg(long)]
        out: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stop the active recording, flush the container and show final statistics.
    Stop {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Show the recorder status (local recording + peer recording disclosure).
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect a recorded TREC container: header metadata + frame record summaries.
    Inspect {
        /// Path to the .trec container file.
        path: String,
        /// Max frame summaries to return (1-512).
        #[arg(long, default_value_t = 32)]
        limit: usize,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Extract a single frame from a recorded TREC container and export as BMP (v2.17 / ADR-103).
    Snapshot {
        /// Path to the .trec container file.
        path: String,
        /// Target frame sequence number to extract.
        #[arg(long)]
        frame: Option<u64>,
        /// Target presentation timestamp (PTS in microseconds) to extract.
        #[arg(long)]
        pts: Option<u64>,
        /// Output file path for the extracted frame (e.g. frame.bmp).
        #[arg(long)]
        out: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenTuningSubCmd {
    /// Show current bottleneck-driven autonomous tuning mode, actions and last decision.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Set tuner mode ('standard', 'aggressive', 'disabled').
    Set {
        /// Tuner mode: standard, aggressive, disabled.
        mode: String,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset autonomous tuning counters, hysteresis windows and the recovery stack.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenLatencySubCmd {
    /// Show current screen latency breakdown, EWMA microsecond stages, percentiles and dominant bottleneck.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Set profiler mode ('auto', 'always', 'disabled').
    Set {
        /// Profiler mode: auto, always, disabled.
        mode: String,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Reset latency profiler metrics, EWMA state, and ring buffer percentiles.
    Reset {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum ScreenPrivacySubCmd {
    /// Show current screen privacy mode and input lockout status.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Enable screen privacy mode (curtain blanking) and optional host input lockout.
    Enable {
        /// Also lock physical keyboard/mouse on host machine while privacy mode is active.
        #[arg(long)]
        lock_input: bool,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Disable screen privacy mode and restore host display and physical inputs.
    Disable {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}
