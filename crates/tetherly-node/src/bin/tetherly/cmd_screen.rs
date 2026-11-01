// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `tetherly screen` dispatcher: one call per `ScreenCmd` variant.

use super::*;

pub(crate) async fn screen(action: ScreenCmd) -> Result<()> {
    match action {
        ScreenCmd::Allow { json, ui_port } => {
            cmd_screen_session::screen_allow(json, ui_port).await?
        }
        ScreenCmd::Revoke { json, ui_port } => {
            cmd_screen_session::screen_revoke(json, ui_port).await?
        }
        ScreenCmd::Start {
            peer,
            json,
            ui_port,
        } => cmd_screen_session::screen_start(peer, json, ui_port).await?,
        ScreenCmd::Stop { json, ui_port } => cmd_screen_session::screen_stop(json, ui_port).await?,
        ScreenCmd::Keyframe { json, ui_port } => {
            cmd_screen_session::screen_keyframe(json, ui_port).await?
        }
        ScreenCmd::Status { json, ui_port } => {
            cmd_screen_session::screen_status(json, ui_port).await?
        }
        ScreenCmd::Bitrate {
            kbps,
            json,
            ui_port,
        } => cmd_screen_quality::screen_bitrate(kbps, json, ui_port).await?,
        ScreenCmd::Fps { fps, json, ui_port } => {
            cmd_screen_quality::screen_fps(fps, json, ui_port).await?
        }
        ScreenCmd::Scale {
            scale,
            json,
            ui_port,
        } => cmd_screen_quality::screen_scale(scale, json, ui_port).await?,
        ScreenCmd::Profile {
            profile,
            min_qp,
            max_qp,
            json,
            ui_port,
        } => cmd_screen_quality::screen_profile(profile, min_qp, max_qp, json, ui_port).await?,
        ScreenCmd::Matrix {
            mode,
            json,
            ui_port,
        } => cmd_screen_quality::screen_matrix(mode, json, ui_port).await?,
        ScreenCmd::Playout {
            mode,
            json,
            ui_port,
        } => cmd_screen_quality::screen_playout(mode, json, ui_port).await?,
        ScreenCmd::Cursor {
            mode,
            json,
            ui_port,
        } => cmd_screen_quality::screen_cursor(mode, json, ui_port).await?,
        ScreenCmd::Squelch {
            mode,
            json,
            ui_port,
        } => cmd_screen_quality::screen_squelch(mode, json, ui_port).await?,
        ScreenCmd::Fec {
            mode,
            json,
            ui_port,
        } => cmd_screen_quality::screen_fec(mode, json, ui_port).await?,
        ScreenCmd::Pacer {
            mode,
            json,
            ui_port,
        } => cmd_screen_quality::screen_pacer(mode, json, ui_port).await?,
        ScreenCmd::Trendline {
            mode,
            json,
            ui_port,
        } => cmd_screen_quality::screen_trendline(mode, json, ui_port).await?,
        ScreenCmd::Harq {
            mode,
            json,
            ui_port,
        } => cmd_screen_quality::screen_harq(mode, json, ui_port).await?,
        ScreenCmd::Twcc {
            mode,
            json,
            ui_port,
        } => cmd_screen_quality::screen_twcc(mode, json, ui_port).await?,
        ScreenCmd::Probe {
            mode,
            target_kbps,
            trigger,
            json,
            ui_port,
        } => cmd_screen_quality::screen_probe(mode, target_kbps, trigger, json, ui_port).await?,
        ScreenCmd::Sync {
            mode,
            trigger,
            json,
            ui_port,
        } => cmd_screen_quality::screen_sync(mode, trigger, json, ui_port).await?,
        ScreenCmd::Displays { json, ui_port } => {
            cmd_screen_session::screen_displays(json, ui_port).await?
        }
        ScreenCmd::Switch {
            display_id,
            json,
            ui_port,
        } => cmd_screen_session::screen_switch(display_id, json, ui_port).await?,
        ScreenCmd::Privacy {
            action,
            enable,
            disable,
            lock_input,
            json,
            ui_port,
        } => {
            cmd_screen_overlay::screen_privacy(action, enable, disable, lock_input, json, ui_port)
                .await?
        }
        ScreenCmd::Latency {
            action,
            mode,
            reset,
            json,
            ui_port,
        } => cmd_screen_replay::screen_latency(action, mode, reset, json, ui_port).await?,
        ScreenCmd::Tuning {
            action,
            mode,
            reset,
            json,
            ui_port,
        } => cmd_screen_replay::screen_tuning(action, mode, reset, json, ui_port).await?,
        ScreenCmd::Recording {
            action,
            out,
            json,
            ui_port,
        } => cmd_screen_replay::screen_recording(action, out, json, ui_port).await?,
        ScreenCmd::Playback {
            action,
            path,
            speed,
            r#loop,
            json,
            ui_port,
        } => cmd_screen_replay::screen_playback(action, path, speed, r#loop, json, ui_port).await?,
        ScreenCmd::Snapshot {
            out,
            format,
            json,
            ui_port,
        } => cmd_screen_session::screen_snapshot(out, format, json, ui_port).await?,
        ScreenCmd::Watermark {
            action,
            text,
            host_enforced,
            opacity,
            color,
            scale,
            pattern,
            spacing_x,
            spacing_y,
            json,
            ui_port,
        } => {
            cmd_screen_overlay::screen_watermark(
                action,
                text,
                host_enforced,
                opacity,
                color,
                scale,
                pattern,
                spacing_x,
                spacing_y,
                json,
                ui_port,
            )
            .await?
        }
        ScreenCmd::Annotate {
            action,
            json,
            ui_port,
        } => cmd_screen_overlay::screen_annotate(action, json, ui_port).await?,
        ScreenCmd::Laser {
            x,
            y,
            color,
            radius,
            off,
            status,
            json,
            ui_port,
        } => {
            cmd_screen_session::screen_laser(x, y, color, radius, off, status, json, ui_port)
                .await?
        }
        ScreenCmd::Whiteboard {
            action,
            json,
            ui_port,
        } => cmd_screen_overlay::screen_whiteboard(action, json, ui_port).await?,
        ScreenCmd::Roi {
            action,
            json,
            ui_port,
        } => cmd_screen_overlay::screen_roi(action, json, ui_port).await?,
        ScreenCmd::Color {
            action,
            json,
            ui_port,
        } => cmd_screen_overlay::screen_color(action, json, ui_port).await?,
        ScreenCmd::Delta {
            action,
            json,
            ui_port,
        } => cmd_screen_encoding::screen_delta(action, json, ui_port).await?,
        ScreenCmd::Metric {
            action,
            json,
            ui_port,
        } => cmd_screen_encoding::screen_metric(action, json, ui_port).await?,
        ScreenCmd::Qoe {
            action,
            json,
            ui_port,
        } => cmd_screen_encoding::screen_qoe(action, json, ui_port).await?,
        ScreenCmd::Pir {
            action,
            json,
            ui_port,
        } => cmd_screen_encoding::screen_pir(action, json, ui_port).await?,
        ScreenCmd::Ltr {
            action,
            json,
            ui_port,
        } => cmd_screen_encoding::screen_ltr(action, json, ui_port).await?,
        ScreenCmd::Svc {
            action,
            json,
            ui_port,
        } => cmd_screen_encoding::screen_svc(action, json, ui_port).await?,
        ScreenCmd::Motion {
            action,
            json,
            ui_port,
        } => cmd_screen_encoding::screen_motion(action, json, ui_port).await?,
        ScreenCmd::Tile {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile(action, json, ui_port).await?,
        ScreenCmd::TileCache {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_cache(action, json, ui_port).await?,
        ScreenCmd::Focus {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_focus(action, json, ui_port).await?,
        ScreenCmd::TilePalette {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_palette(action, json, ui_port).await?,
        ScreenCmd::TilePredict {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_predict(action, json, ui_port).await?,
        ScreenCmd::Predict {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_predict(action, json, ui_port).await?,
        ScreenCmd::TilePresent {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_present(action, json, ui_port).await?,
        ScreenCmd::Present {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_present(action, json, ui_port).await?,
        ScreenCmd::TileNack {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_nack(action, json, ui_port).await?,
        ScreenCmd::Nack {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_nack(action, json, ui_port).await?,
        ScreenCmd::TileSaliency {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_saliency(action, json, ui_port).await?,
        ScreenCmd::Saliency {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_saliency(action, json, ui_port).await?,
        ScreenCmd::TileLod {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_lod(action, json, ui_port).await?,
        ScreenCmd::Lod {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_lod(action, json, ui_port).await?,
        ScreenCmd::TileBatch {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_batch(action, json, ui_port).await?,
        ScreenCmd::Batch {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_batch(action, json, ui_port).await?,
        ScreenCmd::TileMotion {
            action,
            json,
            ui_port,
        } => cmd_screen_tile::screen_tile_motion(action, json, ui_port).await?,
    }
    Ok(())
}
