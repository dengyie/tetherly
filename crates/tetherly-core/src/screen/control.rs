// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Fixed 14-byte control-message envelope (tags and payload codec).

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use crate::frame::FLAG_MUST_UNDERSTAND;

/// The only legal instructions a controller may send. There is deliberately no
/// "run" or "open" here: the screen channel carries pixels and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlMsg {
    /// Controller asks to begin receiving. Refused unless allowed locally.
    Start,
    Stop,
    /// Controller confirms it presented `seq`.
    Ack {
        seq: u64,
    },
    /// Viewer asks sender to emit an independent keyframe (IDR for H.264, v1.95 / ADR-081).
    KeyframeRequest,
    /// Viewer asks sender to dynamically switch captured monitor/display (v2.11 / ADR-097).
    SwitchDisplay {
        display_id: u8,
    },
    /// Viewer/controller asks sender to enable/disable privacy mode (curtain blanking) and input lockout (v2.12 / ADR-098).
    SetPrivacyMode {
        enabled: bool,
        lock_input: bool,
    },
    /// Viewer asks sender to reset multi-stage latency profile statistics (v2.13 / ADR-099).
    ResetLatencyProfile,
    /// Peer asks to synchronize the bottleneck-driven autonomous tuner mode (v2.14 / ADR-100).
    SetTuningMode {
        mode: u8,
    },
    /// Peer asks to reset bottleneck-driven autonomous tuning statistics and recovery stack (v2.14 / ADR-100).
    ResetTuningProfile,
    /// Receiver discloses that it started/stopped recording the session (v2.15 / ADR-101).
    SetRecordingState {
        active: bool,
    },
    /// Host or peer synchronizes dynamic security watermark state (v2.18 / ADR-104).
    SetWatermarkState {
        enabled: bool,
        host_enforced: bool,
    },
    /// Visitor/controller updates transient virtual laser pointer coordinates and state (v2.19 / ADR-105).
    SetLaserPointer {
        x: u16,
        y: u16,
        active: bool,
        color_rgb: [u8; 3],
    },
    /// Broadcast command to clear all collaborative whiteboard annotations (v2.19 / ADR-105).
    ClearAnnotations,
    /// Undo last annotation for a specific author hash, or 0 for global (v2.20 / ADR-106).
    WhiteboardUndo {
        author_hash: u64,
    },
    /// Redo last annotation for a specific author hash, or 0 for global (v2.20 / ADR-106).
    WhiteboardRedo {
        author_hash: u64,
    },
    /// Host updates whiteboard policy (v2.20 / ADR-106).
    SetWhiteboardPolicy {
        read_only: bool,
        allow_peer_clear: bool,
        max_items: u16,
    },
    /// Viewer/controller dynamically updates Region-of-Interest (ROI) viewport cropping bounds (v2.21 / ADR-107).
    SetScreenRoi {
        x: u16,
        y: u16,
        w: u16,
        h: u16,
    },
    /// Viewer/controller resets ROI to full-screen capture and streaming (v2.21 / ADR-107).
    ResetScreenRoi,
    /// Viewer/controller dynamically adjusts ROI zoom scale percentage (v2.21 / ADR-107).
    SetScreenRoiZoom {
        zoom_percent: u16,
    },
    /// Viewer/controller dynamically updates adaptive color depth mode (v2.22 / ADR-108).
    SetScreenColorMode {
        mode: u8,
        threshold: u8,
    },
    /// Viewer/controller resets color mode to FullColor32 (v2.22 / ADR-108).
    ResetScreenColorMode,
    /// Viewer/controller dynamically configures differential delta patch mode (v2.23 / ADR-109).
    SetScreenDeltaMode {
        enabled: bool,
        keyframe_interval: u16,
    },
    /// Viewer/controller resets differential delta patch mode to full-frame streaming (v2.23 / ADR-109).
    ResetScreenDeltaMode,
    /// Viewer/controller dynamically configures real-time objective visual quality metric mode (v2.24 / ADR-110).
    SetScreenQualityMetricMode {
        enabled: bool,
        sample_interval: u8,
    },
    /// Viewer/controller resets objective visual quality metric mode to default disabled (v2.24 / ADR-110).
    ResetScreenQualityMetricMode,
    /// Viewer/controller dynamically configures autonomous closed-loop QoE adaptation mode (v2.25 / ADR-111).
    SetScreenQoeMode {
        mode: u8,
        target_psnr_db: u8,
        target_ssim_pct: u8,
    },
    /// Viewer/controller resets autonomous closed-loop QoE adaptation mode to default disabled (v2.25 / ADR-111).
    ResetScreenQoeMode,
    /// Viewer/controller dynamically configures progressive intra-refresh mode (v2.26 / ADR-112).
    SetScreenPirMode {
        mode: u8,
        bands: u8,
    },
    /// Viewer/controller resets progressive intra-refresh mode to default disabled (v2.26 / ADR-112).
    ResetScreenPirMode,
    /// Viewer/controller dynamically configures long-term reference (LTR) mode (v2.27 / ADR-113).
    SetScreenLtrMode {
        enabled: bool,
        auto_interval: u16,
    },
    /// Viewer/controller resets long-term reference (LTR) mode to default disabled (v2.27 / ADR-113).
    ResetScreenLtrMode,
    /// Viewer/controller asks sender to recover stream state against acknowledged LTR anchor (v2.27 / ADR-113).
    LtrRecoveryRequest {
        ltr_seq: u64,
    },
    /// Sender confirms and synchronizes a new LTR golden reference anchor (v2.27 / ADR-113).
    LtrAnchorSync {
        ltr_seq: u64,
    },
    /// Viewer/controller dynamically configures temporal scalability (SVC-T) mode (v2.28 / ADR-114).
    SetScreenSvcMode {
        mode: u8,
        drop_t1: bool,
    },
    /// Viewer/controller resets temporal scalability (SVC-T) mode to default disabled (v2.28 / ADR-114).
    ResetScreenSvcMode,
    /// Sender or viewer synchronizes temporal scalability (SVC-T) T1 drop count (v2.28 / ADR-114).
    SvcT1DropSync {
        dropped_count: u64,
    },
    /// Viewer/controller dynamically configures 2D motion vector estimation and scroll acceleration mode (v2.29 / ADR-115).
    SetScreenMotionMode {
        enabled: bool,
        max_displacement: u16,
    },
    /// Viewer/controller resets 2D motion vector estimation and scroll acceleration mode to default disabled (v2.29 / ADR-115).
    ResetScreenMotionMode,
    /// Sender synchronizes motion scroll frames sent telemetry (v2.29 / ADR-115).
    MotionScrollSync {
        scroll_frames_sent: u64,
    },
    /// Viewer/controller dynamically configures spatial tile-grid decomposition mode (v2.30 / ADR-116).
    SetScreenTileMode {
        enabled: bool,
        tile_size: u16,
    },
    /// Viewer/controller resets spatial tile-grid decomposition mode to default disabled (v2.30 / ADR-116).
    ResetScreenTileMode,
    /// Sender synchronizes spatial tile-grid frames and tiles sent telemetry (v2.30 / ADR-116).
    TileGridSync {
        tiles_sent: u64,
    },
    /// Viewer/controller dynamically configures screen tile dictionary cache mode (v2.31 / ADR-117).
    SetScreenTileCacheMode {
        enabled: bool,
        max_entries: u16,
    },
    /// Viewer/controller resets screen tile dictionary cache mode to default disabled (v2.31 / ADR-117).
    ResetScreenTileCacheMode,
    /// Sender synchronizes tile cache hits telemetry (v2.31 / ADR-117).
    TileCacheSync {
        hits: u64,
    },
    /// Viewer/controller dynamically configures foveated focus awareness and region-priority scheduling (v2.32 / ADR-118).
    SetScreenFocusMode {
        enabled: bool,
        mode: u8,
        radius: u16,
        peripheral_skip_ratio: u8,
    },
    /// Viewer/controller dynamically updates static focus box bounds (v2.32 / ADR-118).
    SetScreenFocusBox {
        box_x: u16,
        box_y: u16,
        box_w: u16,
        box_h: u16,
    },
    /// Viewer/controller resets foveated focus awareness mode to default disabled (v2.32 / ADR-118).
    ResetScreenFocusMode,
    /// Sender synchronizes foveated focus tiles sent telemetry (v2.32 / ADR-118).
    FocusSync {
        focus_tiles_sent: u64,
    },
    /// Sender synchronizes peripheral tiles skipped telemetry (v2.32 / ADR-118).
    FocusPeripheralSync {
        peripheral_skipped: u64,
    },
    /// Viewer/controller dynamically configures tile-level adaptive palette compression (v2.33 / ADR-119).
    SetScreenTilePaletteMode {
        enabled: bool,
        max_colors: u8,
        enable_rle: bool,
    },
    /// Viewer/controller resets tile-level adaptive palette compression mode to default disabled (v2.33 / ADR-119).
    ResetScreenTilePaletteMode,
    /// Sender synchronizes tile palette compression metrics telemetry (v2.33 / ADR-119).
    TilePaletteSync {
        solid_tiles: u64,
        bilevel_tiles: u64,
        palette_tiles: u64,
        saved_bytes: u64,
    },
    /// Viewer/controller dynamically configures tile-level lossless predictive filter & reversible color transform (v2.34 / ADR-120).
    SetScreenTilePredictMode {
        enabled: bool,
        filter_mode: u8,
        enable_rct: bool,
    },
    /// Viewer/controller resets tile-level predictive filter mode to default disabled (v2.34 / ADR-120).
    ResetScreenTilePredictMode,
    /// Sender synchronizes tile predictive compression metrics telemetry (v2.34 / ADR-120).
    TilePredictSync {
        predict_tiles: u64,
        saved_bytes: u64,
    },
    /// Viewer/controller dynamically configures tile-level presentation transaction & double-buffering (v2.35 / ADR-121).
    SetScreenTilePresentMode {
        enabled: bool,
        vsync_mode: u8,
        double_buffer: bool,
        tear_prevention: bool,
    },
    /// Viewer/controller resets tile-level presentation transaction mode to default (v2.35 / ADR-121).
    ResetScreenTilePresentMode,
    /// Sender/receiver synchronizes tile presentation metrics telemetry (v2.35 / ADR-121).
    TilePresentSync {
        committed_txs: u64,
        swapped_frames: u64,
    },
    /// Viewer/controller dispatches a micro-NACK requesting fast retransmission of a missing dirty tile (v2.36 / ADR-122).
    TileNackRequest {
        seq: u32,
        tile_index: u16,
        retry_count: u8,
    },
    /// Viewer/controller resets tile-level micro-NACK recovery mode to default disabled (v2.36 / ADR-122).
    ResetScreenTileNackMode,
    /// Sender/receiver synchronizes tile-level micro-NACK recovery telemetry (v2.36 / ADR-122).
    TileNackSync {
        nack_requests: u64,
        retransmitted_tiles: u64,
        degraded_commits: u64,
    },
    /// Viewer/controller dynamically configures screen tile saliency tiering & priority protection (v2.37 / ADR-123).
    SetScreenTileSaliencyMode {
        enabled: bool,
        high_threshold: u8,
        low_threshold: u8,
        drop_background_on_congestion: bool,
    },
    /// Viewer/controller resets screen tile saliency mode to default disabled (v2.37 / ADR-123).
    ResetScreenTileSaliencyMode,
    /// Sender/receiver synchronizes tile saliency tiering and congestion skip telemetry (v2.37 / ADR-123).
    TileSaliencySync {
        critical_tiles: u64,
        normal_tiles: u64,
        background_tiles: u64,
        background_tiles_skipped: u64,
    },
    /// Viewer/controller dynamically configures screen tile dynamic multi-resolution Level-of-Detail (LoD) downsampling mode (v2.38 / ADR-124).
    SetScreenTileLodMode {
        enabled: bool,
        max_lod: u8,
        auto_lod_on_congestion: bool,
    },
    /// Viewer/controller resets screen tile LoD downsampling mode to default disabled (v2.38 / ADR-124).
    ResetScreenTileLodMode,
    /// Sender/receiver synchronizes tile LoD downsampling telemetry (v2.38 / ADR-124).
    TileLodSync {
        lod0_tiles: u64,
        lod1_tiles: u64,
        lod2_tiles: u64,
        bytes_saved: u64,
    },
    /// Viewer/controller dynamically configures screen tile batch MTU chunking and pacing (v2.39 / ADR-125).
    SetScreenTileBatchMode {
        enabled: bool,
        target_chunk_bytes: u16,
        pacing_interval_us: u16,
    },
    /// Viewer/controller resets screen tile batch chunking mode to default disabled (v2.39 / ADR-125).
    ResetScreenTileBatchMode,
    /// Sender/receiver synchronizes tile batch MTU chunking and pacing telemetry (v2.39 / ADR-125).
    TileBatchSync {
        batches_sent: u64,
        chunks_sent: u64,
        avg_chunk_bytes: u32,
        pacing_pauses: u64,
    },
    /// Viewer/controller dynamically configures screen tile local motion estimation (v2.40 / ADR-126).
    SetScreenTileMotionMode {
        enabled: bool,
        max_displacement: u16,
        min_match_ratio: u8,
    },
    /// Viewer/controller resets screen tile motion mode to default disabled (v2.40 / ADR-126).
    ResetScreenTileMotionMode,
    /// Sender/receiver synchronizes tile motion estimation and displacement compensation telemetry (v2.40 / ADR-126).
    TileMotionSync {
        motion_tiles: u64,
        fallback_tiles: u64,
        bytes_saved: u64,
    },
}

/// Control payload layout: magic(4) | proto(1) | tag(1) | seq(8).
pub(super) const CONTROL_LEN: usize = 14;
pub(super) const CONTROL_TAG_AT: usize = 5;
pub(super) const CONTROL_SEQ_AT: usize = 6;

impl ControlMsg {
    pub(super) fn tag(self) -> u8 {
        match self {
            ControlMsg::Start => 0,
            ControlMsg::Stop => 1,
            ControlMsg::Ack { .. } => 2,
            ControlMsg::KeyframeRequest => 3,
            ControlMsg::SwitchDisplay { .. } => 4,
            ControlMsg::SetPrivacyMode { .. } => 5,
            ControlMsg::ResetLatencyProfile => 6,
            ControlMsg::SetTuningMode { .. } => 7,
            ControlMsg::ResetTuningProfile => 8,
            ControlMsg::SetRecordingState { .. } => 9,
            ControlMsg::SetWatermarkState { .. } => 10,
            ControlMsg::SetLaserPointer { .. } => 11,
            ControlMsg::ClearAnnotations => 12,
            ControlMsg::WhiteboardUndo { .. } => 13,
            ControlMsg::WhiteboardRedo { .. } => 14,
            ControlMsg::SetWhiteboardPolicy { .. } => 15,
            ControlMsg::SetScreenRoi { .. } => 16,
            ControlMsg::ResetScreenRoi => 17,
            ControlMsg::SetScreenRoiZoom { .. } => 18,
            ControlMsg::SetScreenColorMode { .. } => 19,
            ControlMsg::ResetScreenColorMode => 20,
            ControlMsg::SetScreenDeltaMode { .. } => 21,
            ControlMsg::ResetScreenDeltaMode => 22,
            ControlMsg::SetScreenQualityMetricMode { .. } => 23,
            ControlMsg::ResetScreenQualityMetricMode => 24,
            ControlMsg::SetScreenQoeMode { .. } => 25,
            ControlMsg::ResetScreenQoeMode => 26,
            ControlMsg::SetScreenPirMode { .. } => 27,
            ControlMsg::ResetScreenPirMode => 28,
            ControlMsg::SetScreenLtrMode { .. } => 29,
            ControlMsg::ResetScreenLtrMode => 30,
            ControlMsg::LtrRecoveryRequest { .. } => 31,
            ControlMsg::LtrAnchorSync { .. } => 32,
            ControlMsg::SetScreenSvcMode { .. } => 33,
            ControlMsg::ResetScreenSvcMode => 34,
            ControlMsg::SvcT1DropSync { .. } => 35,
            ControlMsg::SetScreenMotionMode { .. } => 36,
            ControlMsg::ResetScreenMotionMode => 37,
            ControlMsg::MotionScrollSync { .. } => 38,
            ControlMsg::SetScreenTileMode { .. } => 39,
            ControlMsg::ResetScreenTileMode => 40,
            ControlMsg::TileGridSync { .. } => 41,
            ControlMsg::SetScreenTileCacheMode { .. } => 42,
            ControlMsg::ResetScreenTileCacheMode => 43,
            ControlMsg::TileCacheSync { .. } => 44,
            ControlMsg::SetScreenFocusMode { .. } => 45,
            ControlMsg::SetScreenFocusBox { .. } => 46,
            ControlMsg::ResetScreenFocusMode => 47,
            ControlMsg::FocusSync { .. } => 48,
            ControlMsg::FocusPeripheralSync { .. } => 49,
            ControlMsg::SetScreenTilePaletteMode { .. } => 50,
            ControlMsg::ResetScreenTilePaletteMode => 51,
            ControlMsg::TilePaletteSync { .. } => 52,
            ControlMsg::SetScreenTilePredictMode { .. } => 53,
            ControlMsg::ResetScreenTilePredictMode => 54,
            ControlMsg::TilePredictSync { .. } => 55,
            ControlMsg::SetScreenTilePresentMode { .. } => 56,
            ControlMsg::ResetScreenTilePresentMode => 57,
            ControlMsg::TilePresentSync { .. } => 58,
            ControlMsg::TileNackRequest { .. } => 59,
            ControlMsg::ResetScreenTileNackMode => 60,
            ControlMsg::TileNackSync { .. } => 61,
            ControlMsg::SetScreenTileSaliencyMode { .. } => 62,
            ControlMsg::ResetScreenTileSaliencyMode => 63,
            ControlMsg::TileSaliencySync { .. } => 64,
            ControlMsg::SetScreenTileLodMode { .. } => 65,
            ControlMsg::ResetScreenTileLodMode => 66,
            ControlMsg::TileLodSync { .. } => 67,
            ControlMsg::SetScreenTileBatchMode { .. } => 68,
            ControlMsg::ResetScreenTileBatchMode => 69,
            ControlMsg::TileBatchSync { .. } => 70,
            ControlMsg::SetScreenTileMotionMode { .. } => 71,
            ControlMsg::ResetScreenTileMotionMode => 72,
            ControlMsg::TileMotionSync { .. } => 73,
        }
    }

    pub fn encode_payload(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(CONTROL_LEN);
        out.extend_from_slice(&SCREEN_MAGIC);
        out.push(SCREEN_PROTO);
        out.push(self.tag());
        match self {
            ControlMsg::Ack { seq } => out.extend_from_slice(&seq.to_be_bytes()),
            ControlMsg::SwitchDisplay { display_id } => {
                out.extend_from_slice(&(*display_id as u64).to_be_bytes())
            }
            ControlMsg::SetPrivacyMode {
                enabled,
                lock_input,
            } => {
                let val: u64 = (*enabled as u64) | ((*lock_input as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::SetTuningMode { mode } => {
                out.extend_from_slice(&(*mode as u64).to_be_bytes())
            }
            ControlMsg::SetRecordingState { active } => {
                out.extend_from_slice(&(*active as u64).to_be_bytes())
            }
            ControlMsg::SetWatermarkState {
                enabled,
                host_enforced,
            } => {
                let val: u64 = (*enabled as u64) | ((*host_enforced as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::SetLaserPointer {
                x,
                y,
                active,
                color_rgb,
            } => {
                let val: u64 = (*x as u64)
                    | ((*y as u64) << 16)
                    | ((*active as u64) << 32)
                    | ((color_rgb[0] as u64) << 40)
                    | ((color_rgb[1] as u64) << 48)
                    | ((color_rgb[2] as u64) << 56);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ClearAnnotations => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::WhiteboardUndo { author_hash } => {
                out.extend_from_slice(&author_hash.to_be_bytes())
            }
            ControlMsg::WhiteboardRedo { author_hash } => {
                out.extend_from_slice(&author_hash.to_be_bytes())
            }
            ControlMsg::SetWhiteboardPolicy {
                read_only,
                allow_peer_clear,
                max_items,
            } => {
                let val: u64 = (*read_only as u64)
                    | ((*allow_peer_clear as u64) << 1)
                    | ((*max_items as u64) << 16);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::SetScreenRoi { x, y, w, h } => {
                let val: u64 =
                    (*x as u64) | ((*y as u64) << 16) | ((*w as u64) << 32) | ((*h as u64) << 48);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenRoi => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenRoiZoom { zoom_percent } => {
                out.extend_from_slice(&(*zoom_percent as u64).to_be_bytes())
            }
            ControlMsg::SetScreenColorMode { mode, threshold } => {
                let val: u64 = (*mode as u64) | ((*threshold as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenColorMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenDeltaMode {
                enabled,
                keyframe_interval,
            } => {
                let val: u64 =
                    (if *enabled { 1u64 } else { 0u64 }) | ((*keyframe_interval as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenDeltaMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenQualityMetricMode {
                enabled,
                sample_interval,
            } => {
                let val: u64 =
                    (if *enabled { 1u64 } else { 0u64 }) | ((*sample_interval as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenQualityMetricMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenQoeMode {
                mode,
                target_psnr_db,
                target_ssim_pct,
            } => {
                let val: u64 = (*mode as u64)
                    | ((*target_psnr_db as u64) << 8)
                    | ((*target_ssim_pct as u64) << 16);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenQoeMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenPirMode { mode, bands } => {
                let val: u64 = (*mode as u64) | ((*bands as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenPirMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SetScreenLtrMode {
                enabled,
                auto_interval,
            } => {
                let val: u64 =
                    (if *enabled { 1u64 } else { 0u64 }) | ((*auto_interval as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenLtrMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::LtrRecoveryRequest { ltr_seq } => {
                out.extend_from_slice(&ltr_seq.to_be_bytes())
            }
            ControlMsg::LtrAnchorSync { ltr_seq } => out.extend_from_slice(&ltr_seq.to_be_bytes()),
            ControlMsg::SetScreenSvcMode { mode, drop_t1 } => {
                let val: u64 = (*mode as u64) | (if *drop_t1 { 1u64 << 8 } else { 0u64 });
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenSvcMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::SvcT1DropSync { dropped_count } => {
                out.extend_from_slice(&dropped_count.to_be_bytes())
            }
            ControlMsg::SetScreenMotionMode {
                enabled,
                max_displacement,
            } => {
                let val: u64 =
                    (if *enabled { 1u64 } else { 0u64 }) | ((*max_displacement as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenMotionMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::MotionScrollSync { scroll_frames_sent } => {
                out.extend_from_slice(&scroll_frames_sent.to_be_bytes())
            }
            ControlMsg::SetScreenTileMode { enabled, tile_size } => {
                let val: u64 = (if *enabled { 1u64 } else { 0u64 }) | ((*tile_size as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileGridSync { tiles_sent } => {
                out.extend_from_slice(&tiles_sent.to_be_bytes())
            }
            ControlMsg::SetScreenTileCacheMode {
                enabled,
                max_entries,
            } => {
                let val: u64 = (if *enabled { 1u64 } else { 0u64 }) | ((*max_entries as u64) << 8);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileCacheMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileCacheSync { hits } => out.extend_from_slice(&hits.to_be_bytes()),
            ControlMsg::SetScreenFocusMode {
                enabled,
                mode,
                radius,
                peripheral_skip_ratio,
            } => {
                let val: u64 = (if *enabled { 1u64 } else { 0u64 })
                    | ((*mode as u64) << 8)
                    | ((*radius as u64) << 16)
                    | ((*peripheral_skip_ratio as u64) << 32);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::SetScreenFocusBox {
                box_x,
                box_y,
                box_w,
                box_h,
            } => {
                let val: u64 = (*box_x as u64)
                    | ((*box_y as u64) << 16)
                    | ((*box_w as u64) << 32)
                    | ((*box_h as u64) << 48);
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenFocusMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::FocusSync { focus_tiles_sent } => {
                out.extend_from_slice(&focus_tiles_sent.to_be_bytes())
            }
            ControlMsg::FocusPeripheralSync { peripheral_skipped } => {
                out.extend_from_slice(&peripheral_skipped.to_be_bytes())
            }
            ControlMsg::SetScreenTilePaletteMode {
                enabled,
                max_colors,
                enable_rle,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*max_colors as u64) << 8;
                if *enable_rle {
                    val |= 1u64 << 16;
                }
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTilePaletteMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TilePaletteSync { saved_bytes, .. } => {
                out.extend_from_slice(&saved_bytes.to_be_bytes())
            }
            ControlMsg::SetScreenTilePredictMode {
                enabled,
                filter_mode,
                enable_rct,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*filter_mode as u64) << 8;
                if *enable_rct {
                    val |= 1u64 << 16;
                }
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTilePredictMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TilePredictSync { saved_bytes, .. } => {
                out.extend_from_slice(&saved_bytes.to_be_bytes())
            }
            ControlMsg::SetScreenTilePresentMode {
                enabled,
                vsync_mode,
                double_buffer,
                tear_prevention,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*vsync_mode as u64) << 8;
                if *double_buffer {
                    val |= 1u64 << 16;
                }
                if *tear_prevention {
                    val |= 1u64 << 17;
                }
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTilePresentMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TilePresentSync { committed_txs, .. } => {
                out.extend_from_slice(&committed_txs.to_be_bytes())
            }
            ControlMsg::TileNackRequest {
                seq,
                tile_index,
                retry_count,
            } => {
                let mut val: u64 = (*seq as u64) & 0xFFFF_FFFF;
                val |= (*tile_index as u64) << 32;
                val |= (*retry_count as u64) << 48;
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileNackMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileNackSync { nack_requests, .. } => {
                out.extend_from_slice(&nack_requests.to_be_bytes())
            }
            ControlMsg::SetScreenTileSaliencyMode {
                enabled,
                high_threshold,
                low_threshold,
                drop_background_on_congestion,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*high_threshold as u64) << 8;
                val |= (*low_threshold as u64) << 16;
                if *drop_background_on_congestion {
                    val |= 1u64 << 24;
                }
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileSaliencyMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileSaliencySync { critical_tiles, .. } => {
                out.extend_from_slice(&critical_tiles.to_be_bytes())
            }
            ControlMsg::SetScreenTileLodMode {
                enabled,
                max_lod,
                auto_lod_on_congestion,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*max_lod as u64) << 8;
                if *auto_lod_on_congestion {
                    val |= 1u64 << 16;
                }
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileLodMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileLodSync { bytes_saved, .. } => {
                out.extend_from_slice(&bytes_saved.to_be_bytes())
            }
            ControlMsg::SetScreenTileBatchMode {
                enabled,
                target_chunk_bytes,
                pacing_interval_us,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*target_chunk_bytes as u64) << 16;
                val |= (*pacing_interval_us as u64) << 32;
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileBatchMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileBatchSync { batches_sent, .. } => {
                out.extend_from_slice(&batches_sent.to_be_bytes())
            }
            ControlMsg::SetScreenTileMotionMode {
                enabled,
                max_displacement,
                min_match_ratio,
            } => {
                let mut val: u64 = if *enabled { 1u64 } else { 0u64 };
                val |= (*max_displacement as u64) << 16;
                val |= (*min_match_ratio as u64) << 32;
                out.extend_from_slice(&val.to_be_bytes())
            }
            ControlMsg::ResetScreenTileMotionMode => out.extend_from_slice(&0u64.to_be_bytes()),
            ControlMsg::TileMotionSync { motion_tiles, .. } => {
                out.extend_from_slice(&motion_tiles.to_be_bytes())
            }
            _ => out.extend_from_slice(&0u64.to_be_bytes()),
        }
        out
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        Ok(InnerFrame {
            ty: TYPE_SCREEN_CONTROL,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 0,
            payload: self.encode_payload(),
        })
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_CONTROL {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode_payload(&frame.payload)
    }

    pub fn decode_payload(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() != CONTROL_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if payload[0..4] != SCREEN_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[4] != SCREEN_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        let seq = u64::from_be_bytes(payload[CONTROL_SEQ_AT..CONTROL_LEN].try_into().unwrap());
        match payload[CONTROL_TAG_AT] {
            0 => Ok(ControlMsg::Start),
            1 => Ok(ControlMsg::Stop),
            2 => Ok(ControlMsg::Ack { seq }),
            3 => Ok(ControlMsg::KeyframeRequest),
            4 => Ok(ControlMsg::SwitchDisplay {
                display_id: seq as u8,
            }),
            5 => {
                let enabled = (seq & 0xFF) != 0;
                let lock_input = ((seq >> 8) & 0xFF) != 0;
                Ok(ControlMsg::SetPrivacyMode {
                    enabled,
                    lock_input,
                })
            }
            6 => Ok(ControlMsg::ResetLatencyProfile),
            7 => Ok(ControlMsg::SetTuningMode {
                mode: (seq & 0xFF) as u8,
            }),
            8 => Ok(ControlMsg::ResetTuningProfile),
            9 => Ok(ControlMsg::SetRecordingState {
                active: (seq & 0xFF) != 0,
            }),
            10 => {
                let enabled = (seq & 0xFF) != 0;
                let host_enforced = ((seq >> 8) & 0xFF) != 0;
                Ok(ControlMsg::SetWatermarkState {
                    enabled,
                    host_enforced,
                })
            }
            11 => {
                let x = (seq & 0xFFFF) as u16;
                let y = ((seq >> 16) & 0xFFFF) as u16;
                let active = ((seq >> 32) & 0xFF) != 0;
                let color_rgb = [
                    ((seq >> 40) & 0xFF) as u8,
                    ((seq >> 48) & 0xFF) as u8,
                    ((seq >> 56) & 0xFF) as u8,
                ];
                Ok(ControlMsg::SetLaserPointer {
                    x,
                    y,
                    active,
                    color_rgb,
                })
            }
            12 => Ok(ControlMsg::ClearAnnotations),
            13 => Ok(ControlMsg::WhiteboardUndo { author_hash: seq }),
            14 => Ok(ControlMsg::WhiteboardRedo { author_hash: seq }),
            15 => {
                let read_only = (seq & 0x1) != 0;
                let allow_peer_clear = (seq & 0x2) != 0;
                let max_items = ((seq >> 16) & 0xFFFF) as u16;
                Ok(ControlMsg::SetWhiteboardPolicy {
                    read_only,
                    allow_peer_clear,
                    max_items,
                })
            }
            16 => {
                let x = (seq & 0xFFFF) as u16;
                let y = ((seq >> 16) & 0xFFFF) as u16;
                let w = ((seq >> 32) & 0xFFFF) as u16;
                let h = ((seq >> 48) & 0xFFFF) as u16;
                Ok(ControlMsg::SetScreenRoi { x, y, w, h })
            }
            17 => Ok(ControlMsg::ResetScreenRoi),
            18 => {
                let zoom_percent = (seq & 0xFFFF) as u16;
                Ok(ControlMsg::SetScreenRoiZoom { zoom_percent })
            }
            19 => {
                let mode = (seq & 0xFF) as u8;
                let threshold = ((seq >> 8) & 0xFF) as u8;
                Ok(ControlMsg::SetScreenColorMode { mode, threshold })
            }
            20 => Ok(ControlMsg::ResetScreenColorMode),
            21 => {
                let enabled = (seq & 0x1) != 0;
                let keyframe_interval = ((seq >> 8) & 0xFFFF) as u16;
                Ok(ControlMsg::SetScreenDeltaMode {
                    enabled,
                    keyframe_interval,
                })
            }
            22 => Ok(ControlMsg::ResetScreenDeltaMode),
            23 => {
                let enabled = (seq & 0x1) != 0;
                let sample_interval = ((seq >> 8) & 0xFF) as u8;
                Ok(ControlMsg::SetScreenQualityMetricMode {
                    enabled,
                    sample_interval: if sample_interval == 0 {
                        SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL
                    } else {
                        sample_interval.clamp(1, 120)
                    },
                })
            }
            24 => Ok(ControlMsg::ResetScreenQualityMetricMode),
            25 => {
                let mode = (seq & 0xFF) as u8;
                let target_psnr_db = ((seq >> 8) & 0xFF) as u8;
                let target_ssim_pct = ((seq >> 16) & 0xFF) as u8;
                Ok(ControlMsg::SetScreenQoeMode {
                    mode,
                    target_psnr_db: if target_psnr_db == 0 {
                        SCREEN_QOE_DEFAULT_TARGET_PSNR_DB
                    } else {
                        target_psnr_db.clamp(10, 60)
                    },
                    target_ssim_pct: if target_ssim_pct == 0 {
                        SCREEN_QOE_DEFAULT_TARGET_SSIM_PCT
                    } else {
                        target_ssim_pct.clamp(10, 100)
                    },
                })
            }
            26 => Ok(ControlMsg::ResetScreenQoeMode),
            27 => {
                let mode = (seq & 0xFF) as u8;
                let bands = ((seq >> 8) & 0xFF) as u8;
                Ok(ControlMsg::SetScreenPirMode {
                    mode,
                    bands: if bands == 0 {
                        SCREEN_PIR_DEFAULT_BANDS
                    } else {
                        bands.clamp(SCREEN_PIR_MIN_BANDS, SCREEN_PIR_MAX_BANDS)
                    },
                })
            }
            28 => Ok(ControlMsg::ResetScreenPirMode),
            29 => {
                let enabled = (seq & 0x1) != 0;
                let auto_interval = ((seq >> 8) & 0xFFFF) as u16;
                Ok(ControlMsg::SetScreenLtrMode {
                    enabled,
                    auto_interval: if auto_interval == 0 {
                        SCREEN_LTR_DEFAULT_AUTO_INTERVAL
                    } else {
                        auto_interval.clamp(SCREEN_LTR_MIN_INTERVAL, SCREEN_LTR_MAX_INTERVAL)
                    },
                })
            }
            30 => Ok(ControlMsg::ResetScreenLtrMode),
            31 => Ok(ControlMsg::LtrRecoveryRequest { ltr_seq: seq }),
            32 => Ok(ControlMsg::LtrAnchorSync { ltr_seq: seq }),
            33 => {
                let mode = (seq & 0xFF) as u8;
                let drop_t1 = ((seq >> 8) & 0x1) != 0;
                Ok(ControlMsg::SetScreenSvcMode { mode, drop_t1 })
            }
            34 => Ok(ControlMsg::ResetScreenSvcMode),
            35 => Ok(ControlMsg::SvcT1DropSync { dropped_count: seq }),
            36 => {
                let enabled = (seq & 0x1) != 0;
                let max_displacement = ((seq >> 8) & 0xFFFF) as u16;
                let max_displacement = if max_displacement == 0 {
                    SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT
                } else {
                    max_displacement.clamp(
                        SCREEN_MOTION_MIN_DISPLACEMENT,
                        SCREEN_MOTION_MAX_DISPLACEMENT,
                    )
                };
                Ok(ControlMsg::SetScreenMotionMode {
                    enabled,
                    max_displacement,
                })
            }
            37 => Ok(ControlMsg::ResetScreenMotionMode),
            38 => Ok(ControlMsg::MotionScrollSync {
                scroll_frames_sent: seq,
            }),
            39 => {
                let enabled = (seq & 0x1) != 0;
                let tile_size = ((seq >> 8) & 0xFFFF) as u16;
                let tile_size = if tile_size == 0 {
                    SCREEN_TILE_DEFAULT_SIZE
                } else {
                    tile_size.clamp(SCREEN_TILE_MIN_SIZE, SCREEN_TILE_MAX_SIZE)
                };
                Ok(ControlMsg::SetScreenTileMode { enabled, tile_size })
            }
            40 => Ok(ControlMsg::ResetScreenTileMode),
            41 => Ok(ControlMsg::TileGridSync { tiles_sent: seq }),
            42 => {
                let enabled = (seq & 0x1) != 0;
                let max_entries = ((seq >> 8) & 0xFFFF) as u16;
                let max_entries = if max_entries == 0 {
                    SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
                } else {
                    max_entries.clamp(SCREEN_TILE_CACHE_MIN_ENTRIES, SCREEN_TILE_CACHE_MAX_ENTRIES)
                };
                Ok(ControlMsg::SetScreenTileCacheMode {
                    enabled,
                    max_entries,
                })
            }
            43 => Ok(ControlMsg::ResetScreenTileCacheMode),
            44 => Ok(ControlMsg::TileCacheSync { hits: seq }),
            45 => {
                let enabled = (seq & 0x1) != 0;
                let mode = ((seq >> 8) & 0xFF) as u8;
                let radius = ((seq >> 16) & 0xFFFF) as u16;
                let peripheral_skip_ratio = ((seq >> 32) & 0xFF) as u8;
                let radius = if radius == 0 {
                    SCREEN_FOCUS_DEFAULT_RADIUS
                } else {
                    radius.clamp(SCREEN_FOCUS_MIN_RADIUS, SCREEN_FOCUS_MAX_RADIUS)
                };
                let peripheral_skip_ratio = peripheral_skip_ratio.min(SCREEN_FOCUS_MAX_SKIP_RATIO);
                Ok(ControlMsg::SetScreenFocusMode {
                    enabled,
                    mode,
                    radius,
                    peripheral_skip_ratio,
                })
            }
            46 => {
                let box_x = (seq & 0xFFFF) as u16;
                let box_y = ((seq >> 16) & 0xFFFF) as u16;
                let box_w = ((seq >> 32) & 0xFFFF) as u16;
                let box_h = ((seq >> 48) & 0xFFFF) as u16;
                Ok(ControlMsg::SetScreenFocusBox {
                    box_x,
                    box_y,
                    box_w,
                    box_h,
                })
            }
            47 => Ok(ControlMsg::ResetScreenFocusMode),
            48 => Ok(ControlMsg::FocusSync {
                focus_tiles_sent: seq,
            }),
            49 => Ok(ControlMsg::FocusPeripheralSync {
                peripheral_skipped: seq,
            }),
            50 => {
                let enabled = (seq & 0x1) != 0;
                let max_colors = ((seq >> 8) & 0xFF) as u8;
                let enable_rle = ((seq >> 16) & 0x1) != 0;
                let max_colors = if max_colors == 0 {
                    SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS
                } else {
                    max_colors.clamp(2, 16)
                };
                Ok(ControlMsg::SetScreenTilePaletteMode {
                    enabled,
                    max_colors,
                    enable_rle,
                })
            }
            51 => Ok(ControlMsg::ResetScreenTilePaletteMode),
            52 => Ok(ControlMsg::TilePaletteSync {
                solid_tiles: 0,
                bilevel_tiles: 0,
                palette_tiles: 0,
                saved_bytes: seq,
            }),
            53 => {
                let enabled = (seq & 0x1) != 0;
                let filter_mode = ((seq >> 8) & 0xFF) as u8;
                let enable_rct = ((seq >> 16) & 0x1) != 0;
                Ok(ControlMsg::SetScreenTilePredictMode {
                    enabled,
                    filter_mode: filter_mode.min(5),
                    enable_rct,
                })
            }
            54 => Ok(ControlMsg::ResetScreenTilePredictMode),
            55 => Ok(ControlMsg::TilePredictSync {
                predict_tiles: 0,
                saved_bytes: seq,
            }),
            56 => {
                let enabled = (seq & 0x1) != 0;
                let vsync_mode = ((seq >> 8) & 0xFF) as u8;
                let double_buffer = ((seq >> 16) & 0x1) != 0;
                let tear_prevention = ((seq >> 17) & 0x1) != 0;
                Ok(ControlMsg::SetScreenTilePresentMode {
                    enabled,
                    vsync_mode: vsync_mode.min(3),
                    double_buffer,
                    tear_prevention,
                })
            }
            57 => Ok(ControlMsg::ResetScreenTilePresentMode),
            58 => Ok(ControlMsg::TilePresentSync {
                committed_txs: seq,
                swapped_frames: 0,
            }),
            59 => {
                let frame_seq = (seq & 0xFFFF_FFFF) as u32;
                let tile_index = ((seq >> 32) & 0xFFFF) as u16;
                let retry_count = ((seq >> 48) & 0xFF) as u8;
                Ok(ControlMsg::TileNackRequest {
                    seq: frame_seq,
                    tile_index,
                    retry_count,
                })
            }
            60 => Ok(ControlMsg::ResetScreenTileNackMode),
            61 => Ok(ControlMsg::TileNackSync {
                nack_requests: seq,
                retransmitted_tiles: 0,
                degraded_commits: 0,
            }),
            62 => {
                let enabled = (seq & 0x1) != 0;
                let high_threshold = ((seq >> 8) & 0xFF) as u8;
                let low_threshold = ((seq >> 16) & 0xFF) as u8;
                let drop_background_on_congestion = ((seq >> 24) & 0x1) != 0;
                let high = if high_threshold == 0 {
                    120
                } else {
                    high_threshold
                };
                let low = if low_threshold == 0 {
                    40
                } else {
                    low_threshold
                };
                Ok(ControlMsg::SetScreenTileSaliencyMode {
                    enabled,
                    high_threshold: high,
                    low_threshold: low,
                    drop_background_on_congestion,
                })
            }
            63 => Ok(ControlMsg::ResetScreenTileSaliencyMode),
            64 => Ok(ControlMsg::TileSaliencySync {
                critical_tiles: seq,
                normal_tiles: 0,
                background_tiles: 0,
                background_tiles_skipped: 0,
            }),
            65 => {
                let enabled = (seq & 0x1) != 0;
                let max_lod = ((seq >> 8) & 0xFF) as u8;
                let auto_lod_on_congestion = ((seq >> 16) & 0x1) != 0;
                let max_lod = if max_lod == 0 { 1 } else { max_lod.clamp(1, 2) };
                Ok(ControlMsg::SetScreenTileLodMode {
                    enabled,
                    max_lod,
                    auto_lod_on_congestion,
                })
            }
            66 => Ok(ControlMsg::ResetScreenTileLodMode),
            67 => Ok(ControlMsg::TileLodSync {
                lod0_tiles: 0,
                lod1_tiles: 0,
                lod2_tiles: 0,
                bytes_saved: seq,
            }),
            68 => {
                let enabled = (seq & 0x1) != 0;
                let target_chunk_bytes = ((seq >> 16) & 0xFFFF) as u16;
                let pacing_interval_us = ((seq >> 32) & 0xFFFF) as u16;
                let target = if target_chunk_bytes == 0 {
                    SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES
                } else {
                    target_chunk_bytes.clamp(
                        SCREEN_TILE_BATCH_MIN_CHUNK_BYTES,
                        SCREEN_TILE_BATCH_MAX_CHUNK_BYTES,
                    )
                };
                let pacing = pacing_interval_us.min(SCREEN_TILE_BATCH_MAX_PACING_US);
                Ok(ControlMsg::SetScreenTileBatchMode {
                    enabled,
                    target_chunk_bytes: target,
                    pacing_interval_us: pacing,
                })
            }
            69 => Ok(ControlMsg::ResetScreenTileBatchMode),
            70 => Ok(ControlMsg::TileBatchSync {
                batches_sent: seq,
                chunks_sent: 0,
                avg_chunk_bytes: 0,
                pacing_pauses: 0,
            }),
            71 => {
                let enabled = (seq & 0xFF) != 0;
                let max_disp = ((seq >> 16) & 0xFFFF) as u16;
                let min_ratio = ((seq >> 32) & 0xFF) as u8;
                let disp = if max_disp == 0 {
                    SCREEN_TILE_MOTION_DEFAULT_MAX_DISPLACEMENT
                } else {
                    max_disp.clamp(
                        SCREEN_TILE_MOTION_MIN_DISPLACEMENT,
                        SCREEN_TILE_MOTION_MAX_DISPLACEMENT,
                    )
                };
                let ratio = if min_ratio == 0 {
                    SCREEN_TILE_MOTION_DEFAULT_MIN_MATCH_RATIO
                } else {
                    min_ratio.min(100)
                };
                Ok(ControlMsg::SetScreenTileMotionMode {
                    enabled,
                    max_displacement: disp,
                    min_match_ratio: ratio,
                })
            }
            72 => Ok(ControlMsg::ResetScreenTileMotionMode),
            73 => Ok(ControlMsg::TileMotionSync {
                motion_tiles: seq,
                fallback_tiles: 0,
                bytes_saved: 0,
            }),
            _ => Err(CoreError::ScreenFrame),
        }
    }
}
