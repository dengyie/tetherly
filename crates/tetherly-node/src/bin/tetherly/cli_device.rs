// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Subcommand enums for input/display/audio/camera/mic/bench/ancs.

use super::*;

#[derive(Subcommand, Debug)]
pub(crate) enum InputCmd {
    /// Connect and start remote keyboard/mouse channel to a peer (legacy channel).
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
    /// Stop remote keyboard/mouse channel and return cursor locally (legacy channel).
    Stop {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Show remote keyboard/mouse channel status, active input tunnel session, and cursor seat.
    Status {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Attach virtual peripheral input tunnel to a remote target peer (v1.79 / ADR-072).
    Attach {
        /// Target peer device_id (tdev_...) or alias.
        #[arg(long)]
        peer: String,
        /// Screen edge boundary of entry (left, right, top, bottom).
        #[arg(long)]
        edge: Option<String>,
        /// Entry coordinate X on remote target screen.
        #[arg(long)]
        x: Option<i32>,
        /// Entry coordinate Y on remote target screen.
        #[arg(long)]
        y: Option<i32>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Release active virtual peripheral input tunnel and restore local cursor control (v1.79 / ADR-072).
    Release {
        /// Optional human-readable release reason.
        #[arg(long)]
        reason: Option<String>,
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update virtual peripheral sharing and input tunneling policy (v1.79 / ADR-072).
    Policy {
        #[command(subcommand)]
        action: Option<InputPolicySubCmd>,
        /// Enable or disable input tunneling entirely.
        #[arg(long)]
        enable: Option<bool>,
        /// Allow inbound virtual peripheral control from remote peers.
        #[arg(long)]
        allow_inbound: Option<bool>,
        /// Enable or disable keyboard forwarding.
        #[arg(long)]
        keyboard: Option<bool>,
        /// Enable or disable mouse cursor / click forwarding.
        #[arg(long)]
        mouse: Option<bool>,
        /// Enable or disable system hotkey forwarding.
        #[arg(long)]
        hotkey: Option<bool>,
        /// Idle session timeout in seconds (0 to disable).
        #[arg(long)]
        timeout: Option<u64>,
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
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch input tunnel session and policy events via SSE stream.
    Watch {
        /// Output raw JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum InputPolicySubCmd {
    /// Show current input tunneling governance policy.
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Update input tunneling governance policy.
    Set {
        /// Enable or disable input tunneling entirely.
        #[arg(long)]
        enable: Option<bool>,
        /// Allow inbound virtual peripheral control from remote peers.
        #[arg(long)]
        allow_inbound: Option<bool>,
        /// Enable or disable keyboard forwarding.
        #[arg(long)]
        keyboard: Option<bool>,
        /// Enable or disable mouse cursor / click forwarding.
        #[arg(long)]
        mouse: Option<bool>,
        /// Enable or disable system hotkey forwarding.
        #[arg(long)]
        hotkey: Option<bool>,
        /// Idle session timeout in seconds (0 to disable).
        #[arg(long)]
        timeout: Option<u64>,
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
pub(crate) enum DisplayCmd {
    /// Cross-device virtual secondary display, adaptive streaming & display governance (v1.80 / ADR-073).
    Virtual {
        #[command(subcommand)]
        action: VirtualDisplaySubCmd,
    },
    /// Show current virtual secondary display session status and telemetry metrics.
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Attach a remote peer device as a virtual secondary display.
    Attach {
        /// Target peer device_id (tdev_...) or alias.
        #[arg(long)]
        peer: String,
        /// Edge of the local desktop to attach the secondary display (left, right, top, bottom).
        #[arg(long)]
        edge: Option<String>,
        /// Preferred display width in pixels (320..=7680).
        #[arg(long)]
        width: Option<u32>,
        /// Preferred display height in pixels (240..=4320).
        #[arg(long)]
        height: Option<u32>,
        /// Target streaming frame rate in FPS (10..=120).
        #[arg(long)]
        fps: Option<u32>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Detach active virtual secondary display session.
    Detach {
        /// Optional reason for detachment (max 64 chars).
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update virtual secondary display governance policy.
    Policy {
        #[command(subcommand)]
        action: Option<VirtualDisplayPolicySubCmd>,
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-host")]
        allow_host: Option<bool>,
        #[arg(long = "allow-extend")]
        allow_extend: Option<bool>,
        #[arg(long = "max-width")]
        max_width: Option<u32>,
        #[arg(long = "max-height")]
        max_height: Option<u32>,
        #[arg(long = "max-fps")]
        max_fps: Option<u32>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch virtual secondary display session and policy events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum VirtualDisplaySubCmd {
    /// Show current virtual secondary display session status and telemetry metrics.
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Attach a remote peer device as a virtual secondary display.
    Attach {
        /// Target peer device_id (tdev_...) or alias.
        #[arg(long)]
        peer: String,
        /// Edge of the local desktop to attach the secondary display (left, right, top, bottom).
        #[arg(long)]
        edge: Option<String>,
        /// Preferred display width in pixels (320..=7680).
        #[arg(long)]
        width: Option<u32>,
        /// Preferred display height in pixels (240..=4320).
        #[arg(long)]
        height: Option<u32>,
        /// Target streaming frame rate in FPS (10..=120).
        #[arg(long)]
        fps: Option<u32>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Detach active virtual secondary display session.
    Detach {
        /// Optional reason for detachment (max 64 chars).
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update virtual secondary display governance policy.
    Policy {
        #[command(subcommand)]
        action: Option<VirtualDisplayPolicySubCmd>,
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-host")]
        allow_host: Option<bool>,
        #[arg(long = "allow-extend")]
        allow_extend: Option<bool>,
        #[arg(long = "max-width")]
        max_width: Option<u32>,
        #[arg(long = "max-height")]
        max_height: Option<u32>,
        #[arg(long = "max-fps")]
        max_fps: Option<u32>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch virtual secondary display session and policy events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum VirtualDisplayPolicySubCmd {
    /// Show current virtual secondary display governance policy.
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Update virtual secondary display governance policy.
    Set {
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-host")]
        allow_host: Option<bool>,
        #[arg(long = "allow-extend")]
        allow_extend: Option<bool>,
        #[arg(long = "max-width")]
        max_width: Option<u32>,
        #[arg(long = "max-height")]
        max_height: Option<u32>,
        #[arg(long = "max-fps")]
        max_fps: Option<u32>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum AudioCmd {
    /// Cross-device spatial audio routing and stream multiplexing (v1.81 / ADR-074).
    Routing {
        #[command(subcommand)]
        action: AudioRoutingSubCmd,
    },
    /// Show current spatial audio routing session status and acoustic metrics.
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Attach a remote peer device as an audio sink (route audio to peer).
    Attach {
        /// Target peer device_id (tdev_...) or alias.
        #[arg(long)]
        peer: String,
        /// Audio channel layout (mono, stereo, quad, surround5_1).
        #[arg(long)]
        layout: Option<String>,
        /// Audio sample rate in Hz (8000..=192000, default: 48000).
        #[arg(long)]
        sample_rate: Option<u32>,
        /// Number of channels (1..=8, default: layout default).
        #[arg(long)]
        channels: Option<u8>,
        /// Spatial X position (-10.0..=10.0, left to right).
        #[arg(long)]
        x: Option<f32>,
        /// Spatial Y position (-10.0..=10.0, bottom to top).
        #[arg(long)]
        y: Option<f32>,
        /// Spatial Z position (-10.0..=10.0, rear to front).
        #[arg(long)]
        z: Option<f32>,
        /// Playback volume gain multiplier (0.0..=2.0, default: 1.0).
        #[arg(long)]
        gain: Option<f32>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Detach active spatial audio routing session.
    Detach {
        /// Optional reason for detachment (max 64 chars).
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update spatial audio routing governance policy.
    Policy {
        #[command(subcommand)]
        action: Option<AudioRoutingPolicySubCmd>,
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-sink")]
        allow_sink: Option<bool>,
        #[arg(long = "allow-outbound")]
        allow_outbound: Option<bool>,
        #[arg(long = "max-volume")]
        max_volume: Option<u8>,
        #[arg(long = "allow-spatial")]
        allow_spatial: Option<bool>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch spatial audio routing session and policy events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum AudioRoutingSubCmd {
    /// Show current spatial audio routing session status and acoustic metrics.
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Attach a remote peer device as an audio sink (route audio to peer).
    Attach {
        /// Target peer device_id (tdev_...) or alias.
        #[arg(long)]
        peer: String,
        /// Audio channel layout (mono, stereo, quad, surround5_1).
        #[arg(long)]
        layout: Option<String>,
        /// Audio sample rate in Hz (8000..=192000, default: 48000).
        #[arg(long)]
        sample_rate: Option<u32>,
        /// Number of channels (1..=8, default: layout default).
        #[arg(long)]
        channels: Option<u8>,
        /// Spatial X position (-10.0..=10.0, left to right).
        #[arg(long)]
        x: Option<f32>,
        /// Spatial Y position (-10.0..=10.0, bottom to top).
        #[arg(long)]
        y: Option<f32>,
        /// Spatial Z position (-10.0..=10.0, rear to front).
        #[arg(long)]
        z: Option<f32>,
        /// Playback volume gain multiplier (0.0..=2.0, default: 1.0).
        #[arg(long)]
        gain: Option<f32>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Detach active spatial audio routing session.
    Detach {
        /// Optional reason for detachment (max 64 chars).
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update spatial audio routing governance policy.
    Policy {
        #[command(subcommand)]
        action: Option<AudioRoutingPolicySubCmd>,
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-sink")]
        allow_sink: Option<bool>,
        #[arg(long = "allow-outbound")]
        allow_outbound: Option<bool>,
        #[arg(long = "max-volume")]
        max_volume: Option<u8>,
        #[arg(long = "allow-spatial")]
        allow_spatial: Option<bool>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch spatial audio routing session and policy events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum AudioRoutingPolicySubCmd {
    /// Show current spatial audio routing governance policy.
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Update spatial audio routing governance policy.
    Set {
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-sink")]
        allow_sink: Option<bool>,
        #[arg(long = "allow-outbound")]
        allow_outbound: Option<bool>,
        #[arg(long = "max-volume")]
        max_volume: Option<u8>,
        #[arg(long = "allow-spatial")]
        allow_spatial: Option<bool>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum CameraCmd {
    /// Cross-device lightweight virtual camera stream multiplexing (v1.82 / ADR-075).
    Stream {
        #[command(subcommand)]
        action: CameraStreamSubCmd,
    },
    /// Show current virtual camera stream session status and lens metrics.
    Status {
        /// Target peer device_id or alias (optional filter).
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Attach a remote peer device lens as a virtual webcam input.
    Attach {
        /// Target peer device_id (tdev_...) or alias.
        #[arg(long)]
        peer: String,
        /// Lens ID (e.g. back_main, front_selfie, ultra_wide).
        #[arg(long)]
        lens: Option<String>,
        /// Video frame width in pixels (320..=3840, default: 1280).
        #[arg(long)]
        width: Option<u32>,
        /// Video frame height in pixels (240..=2160, default: 720).
        #[arg(long)]
        height: Option<u32>,
        /// Target stream frame rate (10..=60 FPS, default: 30).
        #[arg(long)]
        fps: Option<u32>,
        /// Video compression codec (mjpeg, zstd_rgb, raw_nv12, h264_light).
        #[arg(long)]
        codec: Option<String>,
        /// Enable flashlight/torch on remote device if supported.
        #[arg(long)]
        torch: bool,
        /// Digital zoom ratio (0.5..=20.0, default: 1.0).
        #[arg(long)]
        zoom: Option<f32>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Detach active virtual camera stream session.
    Detach {
        /// Optional reason for detachment (max 64 chars).
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update virtual camera governance policy.
    Policy {
        #[command(subcommand)]
        action: Option<CameraPolicySubCmd>,
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-inbound")]
        allow_inbound: Option<bool>,
        #[arg(long = "allow-capture")]
        allow_capture: Option<bool>,
        #[arg(long = "max-width")]
        max_width: Option<u32>,
        #[arg(long = "max-height")]
        max_height: Option<u32>,
        #[arg(long = "max-fps")]
        max_fps: Option<u32>,
        #[arg(long = "privacy-shutter")]
        privacy_shutter: Option<bool>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch virtual camera stream session and policy events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum CameraStreamSubCmd {
    /// Show current virtual camera stream session status and lens metrics.
    Status {
        #[arg(long)]
        peer: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Attach a remote peer device lens as a virtual webcam input.
    Attach {
        #[arg(long)]
        peer: String,
        #[arg(long)]
        lens: Option<String>,
        #[arg(long)]
        width: Option<u32>,
        #[arg(long)]
        height: Option<u32>,
        #[arg(long)]
        fps: Option<u32>,
        #[arg(long)]
        codec: Option<String>,
        #[arg(long)]
        torch: bool,
        #[arg(long)]
        zoom: Option<f32>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Detach active virtual camera stream session.
    Detach {
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update virtual camera governance policy.
    Policy {
        #[command(subcommand)]
        action: Option<CameraPolicySubCmd>,
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-inbound")]
        allow_inbound: Option<bool>,
        #[arg(long = "allow-capture")]
        allow_capture: Option<bool>,
        #[arg(long = "max-width")]
        max_width: Option<u32>,
        #[arg(long = "max-height")]
        max_height: Option<u32>,
        #[arg(long = "max-fps")]
        max_fps: Option<u32>,
        #[arg(long = "privacy-shutter")]
        privacy_shutter: Option<bool>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch virtual camera stream session and policy events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum CameraPolicySubCmd {
    /// Show current virtual camera governance policy.
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Update virtual camera governance policy.
    Set {
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-inbound")]
        allow_inbound: Option<bool>,
        #[arg(long = "allow-capture")]
        allow_capture: Option<bool>,
        #[arg(long = "max-width")]
        max_width: Option<u32>,
        #[arg(long = "max-height")]
        max_height: Option<u32>,
        #[arg(long = "max-fps")]
        max_fps: Option<u32>,
        #[arg(long = "privacy-shutter")]
        privacy_shutter: Option<bool>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum MicCmd {
    /// Show current virtual microphone / intercom session status and acoustic metrics.
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Attach a remote peer microphone as local virtual mic or push-to-talk intercom.
    Attach {
        /// Target peer device_id (tdev_...) or alias.
        #[arg(long)]
        peer: String,
        /// Microphone device ID on the remote source (default: default).
        #[arg(long)]
        mic: Option<String>,
        /// Session mode: virtual_mic (continuous) or intercom (push-to-talk).
        #[arg(long)]
        mode: Option<String>,
        /// Audio compression codec (opus, pcm16, aac_light).
        #[arg(long)]
        codec: Option<String>,
        /// Sample rate in Hz (8000..=192000, default: 48000).
        #[arg(long = "sample-rate")]
        sample_rate: Option<u32>,
        /// Channel count (1=mono, 2=stereo).
        #[arg(long)]
        channels: Option<u32>,
        /// Capture gain multiplier (0.0..=2.0, default: 1.0).
        #[arg(long)]
        gain: Option<f32>,
        /// Start the session muted on the capture side.
        #[arg(long)]
        muted: bool,
        /// Enable noise suppression on the capture side.
        #[arg(long = "noise-suppression")]
        noise_suppression: Option<bool>,
        /// Enable echo cancellation on the capture side.
        #[arg(long = "echo-cancellation")]
        echo_cancellation: Option<bool>,
        /// Enable automatic gain control on the capture side.
        #[arg(long = "auto-gain")]
        auto_gain: Option<bool>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Detach active virtual microphone / intercom session.
    Detach {
        /// Optional reason for detachment (max 64 chars).
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update virtual microphone governance policy.
    Policy {
        #[command(subcommand)]
        action: Option<MicPolicySubCmd>,
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-inbound")]
        allow_inbound: Option<bool>,
        #[arg(long = "allow-capture")]
        allow_capture: Option<bool>,
        #[arg(long = "allow-intercom")]
        allow_intercom: Option<bool>,
        #[arg(long = "max-rate")]
        max_rate: Option<u32>,
        #[arg(long = "max-duration")]
        max_duration: Option<u32>,
        #[arg(long = "privacy-indicator")]
        privacy_indicator: Option<bool>,
        #[arg(long = "default-muted")]
        default_muted: Option<bool>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch virtual microphone / intercom session and policy events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum MicPolicySubCmd {
    /// Show current virtual microphone governance policy.
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Update virtual microphone governance policy.
    Set {
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-inbound")]
        allow_inbound: Option<bool>,
        #[arg(long = "allow-capture")]
        allow_capture: Option<bool>,
        #[arg(long = "allow-intercom")]
        allow_intercom: Option<bool>,
        #[arg(long = "max-rate")]
        max_rate: Option<u32>,
        #[arg(long = "max-duration")]
        max_duration: Option<u32>,
        #[arg(long = "privacy-indicator")]
        privacy_indicator: Option<bool>,
        #[arg(long = "default-muted")]
        default_muted: Option<bool>,
        #[arg(long = "timeout")]
        timeout: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum BenchCmd {
    /// Show current link benchmark session status and last measurement report.
    Status {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Start a link throughput benchmark against a remote peer.
    Start {
        /// Target peer device_id (tdev_...) or alias.
        #[arg(long)]
        peer: String,
        /// Benchmark direction: push (local sends) or pull (remote sends).
        #[arg(long, default_value = "push")]
        direction: String,
        /// Test duration in seconds (1..=60, default: 5).
        #[arg(long)]
        duration: Option<u32>,
        /// Payload chunk size in bytes (1024..=32768, default: 16384).
        #[arg(long)]
        chunk: Option<u32>,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Stop the active benchmark session early.
    Stop {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Inspect or update link benchmark governance policy.
    Policy {
        #[command(subcommand)]
        action: Option<BenchPolicySubCmd>,
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-inbound")]
        allow_inbound: Option<bool>,
        #[arg(long = "max-duration")]
        max_duration: Option<u32>,
        #[arg(long = "max-bandwidth")]
        max_bandwidth: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Live watch link benchmark session and policy events via SSE stream.
    Watch {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum BenchPolicySubCmd {
    /// Show current link benchmark governance policy.
    Show {
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Update link benchmark governance policy.
    Set {
        #[arg(long)]
        enable: Option<bool>,
        #[arg(long = "allow-inbound")]
        allow_inbound: Option<bool>,
        #[arg(long = "max-duration")]
        max_duration: Option<u32>,
        #[arg(long = "max-bandwidth")]
        max_bandwidth: Option<u32>,
        #[arg(long = "group")]
        group: Option<Vec<String>>,
        #[arg(long = "tag")]
        tag: Option<Vec<String>>,
        #[arg(long = "type")]
        device_type: Option<Vec<String>>,
        #[arg(long)]
        clear_filters: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[derive(Subcommand, Debug)]
pub(crate) enum AncsCmd {
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
