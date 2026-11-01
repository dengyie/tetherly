// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen tile family plus its executors.

use super::util::{loopback_get, loopback_post};
use super::*;

/// `tetherly-cli screen tile` -- Tile arm of the screen dispatcher.
pub(crate) async fn screen_tile(
    action: Option<ScreenTileSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/tile")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_tile_report(&rep);
            }
        }
        Some(ScreenTileSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/tile").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_tile_report(&rep);
            }
        }
        Some(ScreenTileSubCmd::Set {
            enabled,
            tile_size,
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            }
            if let Some(ts) = tile_size {
                payload.insert(
                    "tile_size".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(ts)),
                );
            }
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/tile", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_tile_report(&rep);
            }
        }
        Some(ScreenTileSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_post(port, "/api/screen/tile/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_tile_report(&rep);
            }
        }
        Some(ScreenTileSubCmd::Cache {
            action: sub,
            json: j,
            ui_port: p,
        }) => {
            execute_screen_tile_cache_cmd(sub, json || j, if p != 45716 { p } else { ui_port })
                .await?;
        }
        Some(ScreenTileSubCmd::Focus {
            action: sub,
            json: j,
            ui_port: p,
        }) => {
            execute_screen_focus_cmd(sub, json || j, if p != 45716 { p } else { ui_port }).await?;
        }
        Some(ScreenTileSubCmd::Palette {
            action: sub,
            json: j,
            ui_port: p,
        }) => {
            execute_screen_tile_palette_cmd(sub, json || j, if p != 45716 { p } else { ui_port })
                .await?;
        }
        Some(ScreenTileSubCmd::Predict {
            action: sub,
            json: j,
            ui_port: p,
        }) => {
            execute_screen_tile_predict_cmd(sub, json || j, if p != 45716 { p } else { ui_port })
                .await?;
        }
        Some(ScreenTileSubCmd::Present {
            action: sub,
            json: j,
            ui_port: p,
        }) => {
            execute_screen_tile_present_cmd(sub, json || j, if p != 45716 { p } else { ui_port })
                .await?;
        }
        Some(ScreenTileSubCmd::Nack {
            action: sub,
            json: j,
            ui_port: p,
        }) => {
            execute_screen_tile_nack_cmd(sub, json || j, if p != 45716 { p } else { ui_port })
                .await?;
        }
        Some(ScreenTileSubCmd::Saliency {
            action: sub,
            json: j,
            ui_port: p,
        }) => {
            execute_screen_tile_saliency_cmd(sub, json || j, if p != 45716 { p } else { ui_port })
                .await?;
        }
        Some(ScreenTileSubCmd::Lod {
            action: sub,
            json: j,
            ui_port: p,
        }) => {
            execute_screen_tile_lod_cmd(sub, json || j, if p != 45716 { p } else { ui_port })
                .await?;
        }
        Some(ScreenTileSubCmd::Batch {
            action: sub,
            json: j,
            ui_port: p,
        }) => {
            execute_screen_tile_batch_cmd(sub, json || j, if p != 45716 { p } else { ui_port })
                .await?;
        }
        Some(ScreenTileSubCmd::Motion {
            action: sub,
            json: j,
            ui_port: p,
        }) => {
            execute_screen_tile_motion_cmd(sub, json || j, if p != 45716 { p } else { ui_port })
                .await?;
        }
    }
    Ok(())
}

/// `tetherly-cli screen tile cache` -- TileCache arm of the screen dispatcher.
pub(crate) async fn screen_tile_cache(
    action: Option<ScreenTileCacheSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    execute_screen_tile_cache_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli screen focus` -- Focus arm of the screen dispatcher.
pub(crate) async fn screen_focus(
    action: Option<ScreenFocusSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    execute_screen_focus_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli screen tile palette` -- TilePalette arm of the screen dispatcher.
pub(crate) async fn screen_tile_palette(
    action: Option<ScreenTilePaletteSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    execute_screen_tile_palette_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli screen tile predict` -- TilePredict arm of the screen dispatcher.
pub(crate) async fn screen_tile_predict(
    action: Option<ScreenTilePredictSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    execute_screen_tile_predict_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli screen tile present` -- TilePresent arm of the screen dispatcher.
pub(crate) async fn screen_tile_present(
    action: Option<ScreenTilePresentSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    execute_screen_tile_present_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli screen tile nack` -- TileNack arm of the screen dispatcher.
pub(crate) async fn screen_tile_nack(
    action: Option<ScreenTileNackSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    execute_screen_tile_nack_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli screen tile saliency` -- TileSaliency arm of the screen dispatcher.
pub(crate) async fn screen_tile_saliency(
    action: Option<ScreenTileSaliencySubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    execute_screen_tile_saliency_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli screen tile lod` -- TileLod arm of the screen dispatcher.
pub(crate) async fn screen_tile_lod(
    action: Option<ScreenTileLodSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    execute_screen_tile_lod_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli screen tile batch` -- TileBatch arm of the screen dispatcher.
pub(crate) async fn screen_tile_batch(
    action: Option<ScreenTileBatchSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    execute_screen_tile_batch_cmd(action, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli screen tile motion` -- TileMotion arm of the screen dispatcher.
pub(crate) async fn screen_tile_motion(
    action: Option<ScreenTileMotionSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    execute_screen_tile_motion_cmd(action, json, ui_port).await?;
    Ok(())
}

pub(crate) fn print_screen_tile_report(rep: &serde_json::Value) {
    println!("Screen Spatial Tile-Grid Decomposition & Streaming Pipeline (v2.30 / ADR-116):");
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let tile_size = rep.get("tile_size").and_then(|v| v.as_u64()).unwrap_or(64);
    let grid_frames_sent = rep
        .get("grid_frames_sent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let dirty_tiles_sent = rep
        .get("dirty_tiles_sent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let skipped_tiles_count = rep
        .get("skipped_tiles_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let saved_bandwidth_bytes = rep
        .get("saved_bandwidth_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let status_str = if enabled {
        format!("Enabled (tile_size: {tile_size}px)")
    } else {
        "Disabled".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Tile Size:             {tile_size} px");
    println!("  Grid Frames Sent:      {grid_frames_sent}");
    println!("  Dirty Tiles Sent:      {dirty_tiles_sent}");
    println!("  Skipped Tiles Count:   {skipped_tiles_count}");
    println!("  Saved Bandwidth:       {saved_bandwidth_bytes} bytes");
}

pub(crate) fn print_screen_tile_cache_report(rep: &serde_json::Value) {
    println!("Screen Hash-Based Tile Deduplication & LRU Cache Pipeline (v2.31 / ADR-117):");
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_entries = rep
        .get("max_entries")
        .and_then(|v| v.as_u64())
        .unwrap_or(512);
    let current_entries = rep
        .get("current_entries")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache_hits = rep.get("cache_hits").and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_misses = rep
        .get("cache_misses")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let hit_ratio_pct = rep
        .get("hit_ratio_pct")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let saved_bandwidth_bytes = rep
        .get("saved_bandwidth_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let status_str = if enabled {
        format!("Enabled (max_entries: {max_entries})")
    } else {
        "Disabled".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Cache Dictionary Cap:  {max_entries} slots");
    println!("  Active Cache Entries:  {current_entries} entries");
    println!("  Tile Cache Hits:       {cache_hits}");
    println!("  Tile Cache Misses:     {cache_misses}");
    println!("  Cache Hit Ratio:       {hit_ratio_pct:.2}%");
    println!("  Saved Bandwidth:       {saved_bandwidth_bytes} bytes");
}

pub(crate) async fn execute_screen_tile_cache_cmd(
    action: Option<ScreenTileCacheSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/tile/cache")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile cache query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_tile_cache_report(&rep);
            }
        }
        Some(ScreenTileCacheSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/tile/cache")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile cache query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_tile_cache_report(&rep);
            }
        }
        Some(ScreenTileCacheSubCmd::Set {
            enabled,
            max_entries,
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            }
            if let Some(me) = max_entries {
                payload.insert(
                    "max_entries".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(me)),
                );
            }
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/tile/cache", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile cache set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_tile_cache_report(&rep);
            }
        }
        Some(ScreenTileCacheSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_post(port, "/api/screen/tile/cache/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile cache reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_tile_cache_report(&rep);
            }
        }
    }
    Ok(())
}

pub(crate) fn print_screen_focus_report(rep: &serde_json::Value) {
    println!("Screen Foveated Focus Awareness & Spatial Priority Scheduling (v2.32 / ADR-118):");
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mode = rep.get("mode").and_then(|v| v.as_str()).unwrap_or("cursor");
    let radius = rep.get("radius").and_then(|v| v.as_u64()).unwrap_or(256);
    let skip_ratio = rep
        .get("peripheral_skip_ratio")
        .and_then(|v| v.as_u64())
        .unwrap_or(2);
    let box_x = rep.get("box_x").and_then(|v| v.as_u64()).unwrap_or(0);
    let box_y = rep.get("box_y").and_then(|v| v.as_u64()).unwrap_or(0);
    let box_w = rep.get("box_w").and_then(|v| v.as_u64()).unwrap_or(512);
    let box_h = rep.get("box_h").and_then(|v| v.as_u64()).unwrap_or(512);
    let focus_tiles_sent = rep
        .get("focus_tiles_sent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let peripheral_tiles_sent = rep
        .get("peripheral_tiles_sent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let peripheral_tiles_skipped = rep
        .get("peripheral_tiles_skipped")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let saved_bandwidth_bytes = rep
        .get("saved_bandwidth_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let status_str = if enabled {
        format!("Enabled (mode: {mode}, radius: {radius}px, skip_ratio: {skip_ratio})")
    } else {
        "Disabled".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Focus Tracking Mode:   {mode}");
    println!("  Focus Zone Radius:     {radius} px");
    println!(
        "  Peripheral Skip Ratio: {skip_ratio} (cadence: every {} frames)",
        skip_ratio + 1
    );
    if mode == "box" {
        println!(
            "  Static Focus Box:      ({box_x}, {box_y}) [{}x{}]",
            box_w, box_h
        );
    }
    println!("  Focus Tiles Sent:      {focus_tiles_sent}");
    println!("  Peripheral Tiles Sent: {peripheral_tiles_sent}");
    println!("  Peripheral Skipped:    {peripheral_tiles_skipped}");
    println!("  Saved Bandwidth:       {saved_bandwidth_bytes} bytes");
}

pub(crate) async fn execute_screen_focus_cmd(
    action: Option<ScreenFocusSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/focus")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen focus query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_focus_report(&rep);
            }
            Ok(())
        }
        Some(ScreenFocusSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/focus").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "screen focus query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_focus_report(&rep);
            }
            Ok(())
        }
        Some(ScreenFocusSubCmd::Set {
            enabled,
            mode,
            radius,
            skip_ratio,
            box_x,
            box_y,
            box_w,
            box_h,
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            }
            if let Some(m) = mode {
                payload.insert("mode".to_string(), serde_json::Value::String(m));
            }
            if let Some(r) = radius {
                payload.insert(
                    "radius".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(r)),
                );
            }
            if let Some(s) = skip_ratio {
                payload.insert(
                    "peripheral_skip_ratio".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(s)),
                );
            }
            if let Some(bx) = box_x {
                payload.insert(
                    "box_x".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(bx)),
                );
            }
            if let Some(by) = box_y {
                payload.insert(
                    "box_y".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(by)),
                );
            }
            if let Some(bw) = box_w {
                payload.insert(
                    "box_w".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(bw)),
                );
            }
            if let Some(bh) = box_h {
                payload.insert(
                    "box_h".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(bh)),
                );
            }
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/focus", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen focus set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_focus_report(&rep);
            }
            Ok(())
        }
        Some(ScreenFocusSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_post(port, "/api/screen/focus/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen focus reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_focus_report(&rep);
            }
            Ok(())
        }
    }
}

pub(crate) fn print_screen_tile_palette_report(rep: &serde_json::Value) {
    println!("Screen Tile-Level Adaptive Palette & Hybrid Lossless Compression (v2.33 / ADR-119):");
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_colors = rep.get("max_colors").and_then(|v| v.as_u64()).unwrap_or(16);
    let enable_rle = rep
        .get("enable_rle")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let solid_tiles = rep
        .get("solid_tiles_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let bilevel_tiles = rep
        .get("bilevel_tiles_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let palette_tiles = rep
        .get("palette_tiles_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let truecolor_tiles = rep
        .get("truecolor_tiles_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let saved_bytes = rep
        .get("saved_bandwidth_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let status_str = if enabled {
        format!("Enabled (max_colors: {max_colors}, rle: {enable_rle})")
    } else {
        "Disabled".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Max Palette Colors:    {max_colors} (2..=16)");
    println!(
        "  Run-Length Encoding:   {}",
        if enable_rle { "Enabled" } else { "Disabled" }
    );
    println!("  Solid Tiles (K=1):     {solid_tiles} (5 bytes/tile, 99.97% saving)");
    println!("  Bi-Level Tiles (K=2):  {bilevel_tiles} (521 bytes/tile, 96.8% saving)");
    println!("  Indexed Palette Tiles: {palette_tiles} (nibble-packed, 87%+ saving)");
    println!("  TrueColor Fallback:    {truecolor_tiles} (Zstd / raw)");
    println!("  Saved Bandwidth:       {saved_bytes} bytes");
}

pub(crate) async fn execute_screen_tile_palette_cmd(
    action: Option<ScreenTilePaletteSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/tile/palette")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile palette query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_palette_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTilePaletteSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/tile/palette")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile palette query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json || j {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_palette_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTilePaletteSubCmd::Set {
            enabled,
            max_colors,
            enable_rle,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            } else {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(true));
            }
            if let Some(mc) = max_colors {
                payload.insert("max_colors".to_string(), serde_json::json!(mc));
            }
            if let Some(r) = enable_rle {
                payload.insert("enable_rle".to_string(), serde_json::Value::Bool(r));
            }
            let body = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/tile/palette", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile palette set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_palette_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTilePaletteSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let (code, resp) = loopback_post(port, "/api/screen/tile/palette/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile palette reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_palette_report(&rep);
            }
            Ok(())
        }
    }
}

pub(crate) fn print_screen_tile_predict_report(rep: &serde_json::Value) {
    println!("Screen Tile-Level Predictive Filter & Reversible Color Transform (v2.34 / ADR-120):");
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let filter_mode = rep
        .get("filter_mode")
        .and_then(|v| v.as_str())
        .unwrap_or("auto");
    let enable_rct = rep
        .get("enable_rct")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let predict_tiles = rep
        .get("predict_tiles_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let saved_bytes = rep
        .get("saved_bandwidth_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let status_str = if enabled {
        format!("Enabled (filter: {filter_mode}, rct: {enable_rct})")
    } else {
        "Disabled (default)".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Spatial Filter Mode:   {filter_mode} (None, Sub, Up, Average, Paeth, Auto)");
    println!(
        "  Reversible Color (RCT):{}",
        if enable_rct {
            "Enabled (Y=G, U=R-G, V=B-G, A=A)"
        } else {
            "Disabled"
        }
    );
    println!("  Predict-Filtered Tiles:{predict_tiles} (TILE_SUBCODEC_PREDICT_ZSTD)");
    println!("  Saved Bandwidth:       {saved_bytes} bytes");
}

pub(crate) async fn execute_screen_tile_predict_cmd(
    action: Option<ScreenTilePredictSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/tile/predict")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile predict query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_predict_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTilePredictSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/tile/predict")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile predict query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json || j {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_predict_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTilePredictSubCmd::Set {
            enabled,
            filter_mode,
            enable_rct,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            } else {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(true));
            }
            if let Some(fm) = filter_mode {
                payload.insert("filter_mode".to_string(), serde_json::Value::String(fm));
            }
            if let Some(rct) = enable_rct {
                payload.insert("enable_rct".to_string(), serde_json::Value::Bool(rct));
            }
            let body = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/tile/predict", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile predict set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_predict_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTilePredictSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let (code, resp) = loopback_post(port, "/api/screen/tile/predict/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile predict reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_predict_report(&rep);
            }
            Ok(())
        }
    }
}

pub(crate) fn print_screen_tile_present_report(v: &serde_json::Value) {
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    let vsync_mode = v
        .get("vsync_mode")
        .and_then(|x| x.as_str())
        .unwrap_or("off");
    let double_buffer = v
        .get("double_buffer")
        .and_then(|x| x.as_bool())
        .unwrap_or(true);
    let tear_prevention = v
        .get("tear_prevention")
        .and_then(|x| x.as_bool())
        .unwrap_or(true);
    let committed_tx = v
        .get("committed_transactions")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let swapped = v
        .get("swapped_frames")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let torn_prevented = v
        .get("torn_frames_prevented")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let avg_latency = v
        .get("avg_swap_latency_us")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);

    println!("Screen Tile Presentation Transaction Report:");
    let status_str = if enabled {
        "Enabled (Double-Buffered Staged Atomic Swap)".to_string()
    } else {
        "Disabled (default)".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  V-Sync Pacing Mode:    {vsync_mode} (Off, Vsync60, Vsync120, Adaptive)");
    println!(
        "  Double Buffer Staging: {}",
        if double_buffer { "Enabled" } else { "Disabled" }
    );
    println!(
        "  Anti-Tearing Guard:    {}",
        if tear_prevention {
            "Enabled"
        } else {
            "Disabled"
        }
    );
    println!("  Committed Transactions:{committed_tx}");
    println!("  Swapped Front Frames:  {swapped}");
    println!("  Torn Frames Prevented: {torn_prevented}");
    println!("  Avg Front-Swap Latency:{avg_latency} µs");
}

pub(crate) async fn execute_screen_tile_present_cmd(
    action: Option<ScreenTilePresentSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/tile/present")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile present query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_present_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTilePresentSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/tile/present")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile present query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json || j {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_present_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTilePresentSubCmd::Set {
            enabled,
            vsync_mode,
            double_buffer,
            tear_prevention,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            } else {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(true));
            }
            if let Some(vm) = vsync_mode {
                payload.insert("vsync_mode".to_string(), serde_json::Value::String(vm));
            }
            if let Some(db) = double_buffer {
                payload.insert("double_buffer".to_string(), serde_json::Value::Bool(db));
            }
            if let Some(tp) = tear_prevention {
                payload.insert("tear_prevention".to_string(), serde_json::Value::Bool(tp));
            }
            let body = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/tile/present", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile present set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_present_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTilePresentSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let (code, resp) = loopback_post(port, "/api/screen/tile/present/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile present reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_present_report(&rep);
            }
            Ok(())
        }
    }
}

pub(crate) fn print_screen_tile_nack_report(v: &serde_json::Value) {
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    let timeout_ms = v.get("timeout_ms").and_then(|x| x.as_u64()).unwrap_or(20);
    let max_retries = v.get("max_retries").and_then(|x| x.as_u64()).unwrap_or(2);
    let nack_requests_sent = v
        .get("nack_requests_sent")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let tiles_retransmitted = v
        .get("tiles_retransmitted")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let degraded_commits = v
        .get("degraded_commits")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);

    println!("Screen Tile Micro-NACK Fast Retransmission Report:");
    let status_str = if enabled {
        "Enabled (Semantic Micro-NACK & Ring Buffer Replay)".to_string()
    } else {
        "Disabled (default)".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Timeout Window:        {timeout_ms} ms");
    println!("  Max Retries:           {max_retries}");
    println!("  Micro-NACK Requests:   {nack_requests_sent}");
    println!("  Tiles Retransmitted:   {tiles_retransmitted}");
    println!("  Degraded Commits:      {degraded_commits}");
}

pub(crate) async fn execute_screen_tile_nack_cmd(
    action: Option<ScreenTileNackSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/tile/nack")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile nack query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_nack_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileNackSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/tile/nack")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile nack query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json || j {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_nack_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileNackSubCmd::Set {
            enabled,
            timeout_ms,
            max_retries,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            } else {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(true));
            }
            if let Some(t) = timeout_ms {
                payload.insert(
                    "timeout_ms".to_string(),
                    serde_json::Value::Number(t.into()),
                );
            }
            if let Some(r) = max_retries {
                payload.insert(
                    "max_retries".to_string(),
                    serde_json::Value::Number(r.into()),
                );
            }
            let body = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/tile/nack", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile nack set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_nack_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileNackSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let (code, resp) = loopback_post(port, "/api/screen/tile/nack/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile nack reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_nack_report(&rep);
            }
            Ok(())
        }
    }
}

pub(crate) fn print_screen_tile_saliency_report(v: &serde_json::Value) {
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    let high = v
        .get("high_threshold")
        .and_then(|x| x.as_u64())
        .unwrap_or(120);
    let low = v
        .get("low_threshold")
        .and_then(|x| x.as_u64())
        .unwrap_or(40);
    let drop_bg = v
        .get("drop_background_on_congestion")
        .and_then(|x| x.as_bool())
        .unwrap_or(true);
    let critical_tiles = v
        .get("critical_tiles_sent")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let normal_tiles = v
        .get("normal_tiles_sent")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let background_tiles = v
        .get("background_tiles_sent")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let bg_skipped = v
        .get("background_tiles_skipped")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);

    println!("Screen Tile Saliency & Priority Scheduling Report (ADR-123):");
    let status_str = if enabled {
        "Enabled (Spatial Gradient Energy & Focus Weighted)".to_string()
    } else {
        "Disabled (default)".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  High Threshold:        {high} (Critical tier: score >= {high})");
    println!("  Low Threshold:         {low} (Normal tier: {low} <= score < {high}; Background: score < {low})");
    println!("  Drop BG on Congestion: {drop_bg}");
    println!("  Critical Tiles Sent:   {critical_tiles}");
    println!("  Normal Tiles Sent:     {normal_tiles}");
    println!("  Background Tiles Sent: {background_tiles}");
    println!("  Background Skipped:    {bg_skipped}");
}

pub(crate) async fn execute_screen_tile_saliency_cmd(
    action: Option<ScreenTileSaliencySubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/tile/saliency")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile saliency query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_saliency_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileSaliencySubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/tile/saliency")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile saliency query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json || j {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_saliency_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileSaliencySubCmd::Set {
            enabled,
            high_threshold,
            low_threshold,
            drop_background,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            }
            if let Some(h) = high_threshold {
                payload.insert(
                    "high_threshold".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(h)),
                );
            }
            if let Some(l) = low_threshold {
                payload.insert(
                    "low_threshold".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(l)),
                );
            }
            if let Some(d) = drop_background {
                payload.insert(
                    "drop_background_on_congestion".to_string(),
                    serde_json::Value::Bool(d),
                );
            }
            let body = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/tile/saliency", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile saliency set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_saliency_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileSaliencySubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let (code, resp) = loopback_post(port, "/api/screen/tile/saliency/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile saliency reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_saliency_report(&rep);
            }
            Ok(())
        }
    }
}

pub(crate) fn print_screen_tile_lod_report(v: &serde_json::Value) {
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    let max_lod = v.get("max_lod").and_then(|x| x.as_u64()).unwrap_or(1);
    let auto_lod = v
        .get("auto_lod_on_congestion")
        .and_then(|x| x.as_bool())
        .unwrap_or(true);
    let lod0 = v
        .get("lod0_tiles_sent")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let lod1 = v
        .get("lod1_tiles_sent")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let lod2 = v
        .get("lod2_tiles_sent")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let bytes_saved = v.get("bytes_saved").and_then(|x| x.as_u64()).unwrap_or(0);

    println!("Screen Tile Level-of-Detail (LoD) Downsampling Report (ADR-124):");
    let status_str = if enabled {
        "Enabled (Dynamic Box-Filter Downsampling & Bilinear Reconstruction)".to_string()
    } else {
        "Disabled (default)".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Max LoD Level:         Lod{max_lod} (Lod1: 32x32, Lod2: 16x16)");
    println!("  Auto LoD on Congest:   {auto_lod}");
    println!("  Lod0 Tiles (64x64):    {lod0}");
    println!("  Lod1 Tiles (32x32):    {lod1}");
    println!("  Lod2 Tiles (16x16):    {lod2}");
    println!("  Bandwidth Bytes Saved: {bytes_saved} B");
}

pub(crate) async fn execute_screen_tile_lod_cmd(
    action: Option<ScreenTileLodSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/tile/lod")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile lod query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_lod_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileLodSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/tile/lod")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile lod query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json || j {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_lod_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileLodSubCmd::Set {
            enabled,
            max_lod,
            auto_lod,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            }
            if let Some(m) = max_lod {
                payload.insert("max_lod".to_string(), serde_json::Value::from(m));
            }
            if let Some(a) = auto_lod {
                payload.insert(
                    "auto_lod_on_congestion".to_string(),
                    serde_json::Value::Bool(a),
                );
            }
            let body = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/tile/lod", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile lod set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_lod_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileLodSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let (code, resp) = loopback_post(port, "/api/screen/tile/lod/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile lod reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_lod_report(&rep);
            }
            Ok(())
        }
    }
}

pub(crate) fn print_screen_tile_batch_report(v: &serde_json::Value) {
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    let target_chunk_bytes = v
        .get("target_chunk_bytes")
        .and_then(|x| x.as_u64())
        .unwrap_or(1200);
    let pacing_interval_us = v
        .get("pacing_interval_us")
        .and_then(|x| x.as_u64())
        .unwrap_or(200);
    let batches_sent = v.get("batches_sent").and_then(|x| x.as_u64()).unwrap_or(0);
    let chunks_sent = v.get("chunks_sent").and_then(|x| x.as_u64()).unwrap_or(0);
    let avg_chunk_bytes = v
        .get("avg_chunk_bytes")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let pacing_pauses = v.get("pacing_pauses").and_then(|x| x.as_u64()).unwrap_or(0);

    println!("Screen Tile Dynamic MTU Batching & Pacing Report (ADR-125):");
    let status_str = if enabled {
        "Enabled (Dynamic MTU Chunking & Coalesced Pacing)".to_string()
    } else {
        "Disabled (default)".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Target Chunk Bytes:    {target_chunk_bytes} B (512..=64000)");
    println!("  Pacing Interval:       {pacing_interval_us} µs");
    println!("  Batches Sent:          {batches_sent}");
    println!("  Chunks Sent:           {chunks_sent}");
    println!("  Avg Chunk Bytes:       {avg_chunk_bytes} B");
    println!("  Pacing Pauses:         {pacing_pauses}");
}

pub(crate) async fn execute_screen_tile_batch_cmd(
    action: Option<ScreenTileBatchSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/tile/batch")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile batch query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_batch_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileBatchSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/tile/batch")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile batch query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json || j {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_batch_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileBatchSubCmd::Set {
            enabled,
            target_chunk_bytes,
            pacing_interval_us,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            }
            if let Some(b) = target_chunk_bytes {
                payload.insert("target_chunk_bytes".to_string(), serde_json::Value::from(b));
            }
            if let Some(us) = pacing_interval_us {
                payload.insert(
                    "pacing_interval_us".to_string(),
                    serde_json::Value::from(us),
                );
            }
            let body = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/tile/batch", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile batch set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_batch_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileBatchSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let (code, resp) = loopback_post(port, "/api/screen/tile/batch/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile batch reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_batch_report(&rep);
            }
            Ok(())
        }
    }
}

pub(crate) fn print_screen_tile_motion_report(v: &serde_json::Value) {
    let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    let max_displacement = v
        .get("max_displacement")
        .and_then(|x| x.as_u64())
        .unwrap_or(32);
    let min_match_ratio = v
        .get("min_match_ratio")
        .and_then(|x| x.as_u64())
        .unwrap_or(90);
    let motion_tiles_sent = v
        .get("motion_tiles_sent")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let fallback_tiles_sent = v
        .get("fallback_tiles_sent")
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let bytes_saved = v.get("bytes_saved").and_then(|x| x.as_u64()).unwrap_or(0);

    println!("Screen Tile Dynamic Local Motion Vector Estimation & Displacement Compensation Report (ADR-126):");
    let status_str = if enabled {
        "Enabled (Local Motion Vector Search & Cross-Frame Displacement Blit)".to_string()
    } else {
        "Disabled (default)".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Max Displacement:      {max_displacement} px (4..=128)");
    println!("  Min Match Ratio:       {min_match_ratio}% (0.50..=1.00)");
    println!("  Motion Tiles Sent:     {motion_tiles_sent}");
    println!("  Fallback Tiles Sent:   {fallback_tiles_sent}");
    println!("  Bytes Saved:           {bytes_saved} B");
}

pub(crate) async fn execute_screen_tile_motion_cmd(
    action: Option<ScreenTileMotionSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/tile/motion")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile motion query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_motion_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileMotionSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/tile/motion")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile motion query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if json || j {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_motion_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileMotionSubCmd::Set {
            enabled,
            max_displacement,
            min_match_ratio,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            }
            if let Some(d) = max_displacement {
                payload.insert("max_displacement".to_string(), serde_json::Value::from(d));
            }
            if let Some(r) = min_match_ratio {
                payload.insert("min_match_ratio".to_string(), serde_json::Value::from(r));
            }
            let body = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/tile/motion", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile motion set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_motion_report(&rep);
            }
            Ok(())
        }
        Some(ScreenTileMotionSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let effective_json = json || j;
            let (code, resp) = loopback_post(port, "/api/screen/tile/motion/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen tile motion reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let rep: serde_json::Value = serde_json::from_slice(&resp)?;
            if effective_json {
                println!("{}", serde_json::to_string_pretty(&rep)?);
            } else {
                print_screen_tile_motion_report(&rep);
            }
            Ok(())
        }
    }
}
