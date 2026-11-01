// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen tile streaming, tile dictionaries and tile batch controls.

use super::*;

pub(super) async fn get_screen_tile(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_tile_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_tile(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct TileBody {
        enabled: Option<bool>,
        tile_size: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TileBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let tile_size = parsed_body
        .as_ref()
        .and_then(|b| b.tile_size)
        .or_else(|| qmap.get("tile_size").and_then(|v| v.parse::<u16>().ok()))
        .unwrap_or(tetherly_core::SCREEN_TILE_DEFAULT_SIZE);

    match node.screen_set_tile_mode(enabled, tile_size).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen tile mode: {e}")),
    }
}

// Screen streaming tile dictionary cache (v2.31 / ADR-117)
pub(super) async fn post_screen_tile_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tile_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen tile mode: {e}")),
    }
}

pub(super) async fn get_screen_tile_cache(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_tile_cache_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_tile_cache(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct TileCacheBody {
        enabled: Option<bool>,
        max_entries: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TileCacheBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let max_entries = parsed_body
        .as_ref()
        .and_then(|b| b.max_entries)
        .or_else(|| qmap.get("max_entries").and_then(|v| v.parse::<u16>().ok()))
        .unwrap_or(tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES);

    match node.screen_set_tile_cache_mode(enabled, max_entries).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen tile cache mode: {e}")),
    }
}

pub(super) async fn post_screen_tile_cache_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tile_cache_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen tile cache mode: {e}")),
    }
}

pub(super) async fn get_screen_tile_palette(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_tile_palette_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_tile_palette(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct TilePaletteBody {
        enabled: Option<bool>,
        max_colors: Option<u8>,
        enable_rle: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TilePaletteBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let max_colors = parsed_body
        .as_ref()
        .and_then(|b| b.max_colors)
        .or_else(|| qmap.get("max_colors").and_then(|v| v.parse::<u8>().ok()));
    let enable_rle = parsed_body
        .as_ref()
        .and_then(|b| b.enable_rle)
        .or_else(|| qmap.get("enable_rle").and_then(|v| v.parse::<bool>().ok()));

    match node
        .screen_set_tile_palette_mode(enabled, max_colors, enable_rle)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen tile palette mode: {e}")),
    }
}

pub(super) async fn post_screen_tile_palette_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tile_palette_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(
            500,
            &format!("failed to reset screen tile palette mode: {e}"),
        ),
    }
}

pub(super) async fn get_screen_tile_predict(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_tile_predict_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_tile_predict(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct TilePredictBody {
        enabled: Option<bool>,
        filter_mode: Option<serde_json::Value>,
        enable_rct: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TilePredictBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let filter_mode = parsed_body
        .as_ref()
        .and_then(|b| b.filter_mode.as_ref())
        .and_then(|val| {
            if let Some(s) = val.as_str() {
                tetherly_core::ScreenTilePredictFilter::from_str_loose(s)
            } else {
                val.as_u64()
                    .map(|n| tetherly_core::ScreenTilePredictFilter::from_u8(n as u8))
            }
        })
        .or_else(|| {
            qmap.get("filter_mode")
                .or_else(|| qmap.get("mode"))
                .and_then(|s| tetherly_core::ScreenTilePredictFilter::from_str_loose(s))
        });
    let enable_rct = parsed_body.as_ref().and_then(|b| b.enable_rct).or_else(|| {
        qmap.get("enable_rct")
            .or_else(|| qmap.get("rct"))
            .and_then(|v| v.parse::<bool>().ok())
    });

    match node
        .screen_set_tile_predict_mode(enabled, filter_mode, enable_rct)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen tile predict mode: {e}")),
    }
}

pub(super) async fn post_screen_tile_predict_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tile_predict_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(
            500,
            &format!("failed to reset screen tile predict mode: {e}"),
        ),
    }
}

pub(super) async fn get_screen_tile_present(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_tile_present_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_tile_present(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct TilePresentBody {
        enabled: Option<bool>,
        vsync_mode: Option<serde_json::Value>,
        double_buffer: Option<bool>,
        tear_prevention: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TilePresentBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let vsync_mode = parsed_body
        .as_ref()
        .and_then(|b| b.vsync_mode.as_ref())
        .and_then(|val| {
            if let Some(s) = val.as_str() {
                tetherly_core::ScreenTilePresentVsyncMode::from_str_loose(s)
            } else {
                val.as_u64()
                    .map(|n| tetherly_core::ScreenTilePresentVsyncMode::from_u8(n as u8))
            }
        })
        .or_else(|| {
            qmap.get("vsync_mode")
                .or_else(|| qmap.get("vsync"))
                .or_else(|| qmap.get("mode"))
                .and_then(|s| tetherly_core::ScreenTilePresentVsyncMode::from_str_loose(s))
        });
    let double_buffer = parsed_body
        .as_ref()
        .and_then(|b| b.double_buffer)
        .or_else(|| {
            qmap.get("double_buffer")
                .or_else(|| qmap.get("double"))
                .and_then(|v| v.parse::<bool>().ok())
        });
    let tear_prevention = parsed_body
        .as_ref()
        .and_then(|b| b.tear_prevention)
        .or_else(|| {
            qmap.get("tear_prevention")
                .or_else(|| qmap.get("anti_tearing"))
                .or_else(|| qmap.get("tear"))
                .and_then(|v| v.parse::<bool>().ok())
        });

    match node
        .screen_set_tile_present_mode(enabled, vsync_mode, double_buffer, tear_prevention)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen tile present mode: {e}")),
    }
}

pub(super) async fn post_screen_tile_present_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tile_present_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(
            500,
            &format!("failed to reset screen tile present mode: {e}"),
        ),
    }
}

pub(super) async fn get_screen_tile_nack(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_tile_nack_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_tile_nack(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct TileNackBody {
        enabled: Option<bool>,
        timeout_ms: Option<u16>,
        max_retries: Option<u8>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TileNackBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let timeout_ms = parsed_body.as_ref().and_then(|b| b.timeout_ms).or_else(|| {
        qmap.get("timeout_ms")
            .or_else(|| qmap.get("timeout"))
            .and_then(|v| v.parse::<u16>().ok())
    });
    let max_retries = parsed_body
        .as_ref()
        .and_then(|b| b.max_retries)
        .or_else(|| {
            qmap.get("max_retries")
                .or_else(|| qmap.get("retries"))
                .and_then(|v| v.parse::<u8>().ok())
        });

    match node
        .screen_set_tile_nack_mode(enabled, timeout_ms, max_retries)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen tile nack mode: {e}")),
    }
}

pub(super) async fn post_screen_tile_nack_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tile_nack_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen tile nack mode: {e}")),
    }
}

pub(super) async fn get_screen_tile_saliency(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_tile_saliency_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_tile_saliency(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct TileSaliencyBody {
        enabled: Option<bool>,
        high_threshold: Option<u8>,
        low_threshold: Option<u8>,
        drop_background_on_congestion: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TileSaliencyBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let high_threshold = parsed_body
        .as_ref()
        .and_then(|b| b.high_threshold)
        .or_else(|| {
            qmap.get("high_threshold")
                .or_else(|| qmap.get("high"))
                .and_then(|v| v.parse::<u8>().ok())
        });
    let low_threshold = parsed_body
        .as_ref()
        .and_then(|b| b.low_threshold)
        .or_else(|| {
            qmap.get("low_threshold")
                .or_else(|| qmap.get("low"))
                .and_then(|v| v.parse::<u8>().ok())
        });
    let drop_background_on_congestion = parsed_body
        .as_ref()
        .and_then(|b| b.drop_background_on_congestion)
        .or_else(|| {
            qmap.get("drop_background_on_congestion")
                .or_else(|| qmap.get("drop_background"))
                .and_then(|v| v.parse::<bool>().ok())
        });

    match node
        .screen_set_tile_saliency_mode(
            enabled,
            high_threshold,
            low_threshold,
            drop_background_on_congestion,
        )
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(
            500,
            &format!("failed to set screen tile saliency mode: {e}"),
        ),
    }
}

pub(super) async fn post_screen_tile_saliency_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tile_saliency_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(
            500,
            &format!("failed to reset screen tile saliency mode: {e}"),
        ),
    }
}

pub(super) async fn get_screen_tile_lod(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.screen_tile_lod_report())
}

pub(super) async fn post_screen_tile_lod(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct TileLodBody {
        enabled: Option<bool>,
        max_lod: Option<u8>,
        auto_lod_on_congestion: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TileLodBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let max_lod = parsed_body.as_ref().and_then(|b| b.max_lod).or_else(|| {
        qmap.get("max_lod")
            .or_else(|| qmap.get("max"))
            .or_else(|| qmap.get("lod"))
            .and_then(|v| v.parse::<u8>().ok())
    });
    let auto_lod_on_congestion = parsed_body
        .as_ref()
        .and_then(|b| b.auto_lod_on_congestion)
        .or_else(|| {
            qmap.get("auto_lod_on_congestion")
                .or_else(|| qmap.get("auto_lod"))
                .or_else(|| qmap.get("auto"))
                .and_then(|v| v.parse::<bool>().ok())
        });

    match node
        .screen_set_tile_lod_mode(enabled, max_lod, auto_lod_on_congestion)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen tile lod mode: {e}")),
    }
}

pub(super) async fn post_screen_tile_lod_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tile_lod_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen tile lod mode: {e}")),
    }
}

pub(super) async fn get_screen_tile_batch(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.screen_tile_batch_report())
}

pub(super) async fn post_screen_tile_batch(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct TileBatchBody {
        enabled: Option<bool>,
        target_chunk_bytes: Option<u16>,
        pacing_interval_us: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TileBatchBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let target_chunk_bytes = parsed_body
        .as_ref()
        .and_then(|b| b.target_chunk_bytes)
        .or_else(|| {
            qmap.get("target_chunk_bytes")
                .or_else(|| qmap.get("chunk_bytes"))
                .or_else(|| qmap.get("target"))
                .or_else(|| qmap.get("bytes"))
                .and_then(|v| v.parse::<u16>().ok())
        });
    let pacing_interval_us = parsed_body
        .as_ref()
        .and_then(|b| b.pacing_interval_us)
        .or_else(|| {
            qmap.get("pacing_interval_us")
                .or_else(|| qmap.get("pacing_us"))
                .or_else(|| qmap.get("pacing"))
                .or_else(|| qmap.get("interval"))
                .and_then(|v| v.parse::<u16>().ok())
        });

    match node
        .screen_set_tile_batch_mode(enabled, target_chunk_bytes, pacing_interval_us)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen tile batch mode: {e}")),
    }
}

pub(super) async fn post_screen_tile_batch_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tile_batch_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen tile batch mode: {e}")),
    }
}

pub(super) async fn get_screen_tile_motion(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.screen_tile_motion_report())
}

pub(super) async fn post_screen_tile_motion(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct TileMotionBody {
        enabled: Option<bool>,
        max_displacement: Option<u16>,
        min_match_ratio: Option<u8>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TileMotionBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let max_displacement = parsed_body
        .as_ref()
        .and_then(|b| b.max_displacement)
        .or_else(|| {
            qmap.get("max_displacement")
                .or_else(|| qmap.get("max_disp"))
                .or_else(|| qmap.get("displacement"))
                .or_else(|| qmap.get("disp"))
                .and_then(|v| v.parse::<u16>().ok())
        });
    let min_match_ratio = parsed_body
        .as_ref()
        .and_then(|b| b.min_match_ratio)
        .or_else(|| {
            qmap.get("min_match_ratio")
                .or_else(|| qmap.get("min_ratio"))
                .or_else(|| qmap.get("ratio"))
                .and_then(|v| v.parse::<u8>().ok())
        });

    match node
        .screen_set_tile_motion_mode(enabled, max_displacement, min_match_ratio)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen tile motion mode: {e}")),
    }
}

// Local input sharing (controller → controlled). Dialling the 45719
// channel is what lets `drive_input*` actually deliver; the established
// capture hooks are inert until then.
pub(super) async fn post_screen_tile_motion_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tile_motion_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(
            500,
            &format!("failed to reset screen tile motion mode: {e}"),
        ),
    }
}
