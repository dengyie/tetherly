// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Tile level-of-detail, MTU batch chunking and local tile motion vectors.

use super::*;
use crate::error::CoreError;

/// Runtime configuration for screen tile dynamic LoD downsampling (v2.38 / ADR-124).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileLodConfig {
    pub enabled: bool,
    pub max_lod: u8, // 1 (Lod1) or 2 (Lod2)
    pub auto_lod_on_congestion: bool,
}

impl Default for ScreenTileLodConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_lod: 1,
            auto_lod_on_congestion: true,
        }
    }
}

/// Comprehensive telemetry report for screen tile dynamic LoD downsampling (v2.38 / ADR-124).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileLodReport {
    pub ok: bool,
    pub enabled: bool,
    pub max_lod: u8,
    pub auto_lod_on_congestion: bool,
    pub lod0_tiles_sent: u64,
    pub lod1_tiles_sent: u64,
    pub lod2_tiles_sent: u64,
    pub bytes_saved: u64,
}

impl Default for ScreenTileLodReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            max_lod: 1,
            auto_lod_on_congestion: true,
            lod0_tiles_sent: 0,
            lod1_tiles_sent: 0,
            lod2_tiles_sent: 0,
            bytes_saved: 0,
        }
    }
}

/// Downsamples an RGBA tile buffer into a smaller resolution according to TileLodLevel using box-filter averaging.
pub fn downsample_tile_rgba(
    raw_pixels: &[u8],
    src_w: u16,
    src_h: u16,
    lod: TileLodLevel,
) -> (Vec<u8>, u16, u16) {
    if lod == TileLodLevel::Lod0 || src_w == 0 || src_h == 0 {
        return (raw_pixels.to_vec(), src_w, src_h);
    }
    let factor = lod.scale_factor() as usize;
    let dst_w = (src_w as usize / factor).max(1);
    let dst_h = (src_h as usize / factor).max(1);
    let mut out = vec![0u8; dst_w * dst_h * 4];
    let src_stride = src_w as usize * 4;

    for dy in 0..dst_h {
        let sy_start = dy * factor;
        let sy_end = (sy_start + factor).min(src_h as usize);
        let h_count = (sy_end - sy_start).max(1);

        for dx in 0..dst_w {
            let sx_start = dx * factor;
            let sx_end = (sx_start + factor).min(src_w as usize);
            let w_count = (sx_end - sx_start).max(1);
            let pixel_count = (h_count * w_count) as u32;

            let mut r_acc = 0u32;
            let mut g_acc = 0u32;
            let mut b_acc = 0u32;
            let mut a_acc = 0u32;

            for sy in sy_start..sy_end {
                let row_off = sy * src_stride;
                for sx in sx_start..sx_end {
                    let off = row_off + sx * 4;
                    if off + 3 < raw_pixels.len() {
                        r_acc += raw_pixels[off] as u32;
                        g_acc += raw_pixels[off + 1] as u32;
                        b_acc += raw_pixels[off + 2] as u32;
                        a_acc += raw_pixels[off + 3] as u32;
                    }
                }
            }

            let dst_off = (dy * dst_w + dx) * 4;
            out[dst_off] = (r_acc / pixel_count) as u8;
            out[dst_off + 1] = (g_acc / pixel_count) as u8;
            out[dst_off + 2] = (b_acc / pixel_count) as u8;
            out[dst_off + 3] = (a_acc / pixel_count) as u8;
        }
    }

    (out, dst_w as u16, dst_h as u16)
}

/// Upsamples a sub-resolution RGBA tile buffer back to full dimensions (dst_w, dst_h) using bilinear interpolation.
pub fn upsample_tile_rgba(
    sub_pixels: &[u8],
    sub_w: u16,
    sub_h: u16,
    dst_w: u16,
    dst_h: u16,
) -> Vec<u8> {
    if sub_w == 0 || sub_h == 0 || dst_w == 0 || dst_h == 0 {
        return vec![0u8; dst_w as usize * dst_h as usize * 4];
    }
    if sub_w == dst_w && sub_h == dst_h {
        return sub_pixels.to_vec();
    }
    let sub_w_us = sub_w as usize;
    let sub_h_us = sub_h as usize;
    let dst_w_us = dst_w as usize;
    let dst_h_us = dst_h as usize;
    let mut out = vec![0u8; dst_w_us * dst_h_us * 4];
    let sub_stride = sub_w_us * 4;

    let scale_x = ((sub_w_us as u32) << 16) / dst_w_us as u32;
    let scale_y = ((sub_h_us as u32) << 16) / dst_h_us as u32;

    for dy in 0..dst_h_us {
        let sy_fp = (dy as u32 * scale_y).saturating_sub(1 << 15);
        let sy0 = (sy_fp >> 16) as usize;
        let sy1 = (sy0 + 1).min(sub_h_us - 1);
        let fy = sy_fp & 0xFFFF;
        let ify = 0x10000 - fy;

        let dst_row = dy * dst_w_us * 4;
        let row0 = sy0.min(sub_h_us - 1) * sub_stride;
        let row1 = sy1.min(sub_h_us - 1) * sub_stride;

        for dx in 0..dst_w_us {
            let sx_fp = (dx as u32 * scale_x).saturating_sub(1 << 15);
            let sx0 = (sx_fp >> 16) as usize;
            let sx1 = (sx0 + 1).min(sub_w_us - 1);
            let fx = sx_fp & 0xFFFF;
            let ifx = 0x10000 - fx;

            let c00 = row0 + sx0.min(sub_w_us - 1) * 4;
            let c10 = row0 + sx1.min(sub_w_us - 1) * 4;
            let c01 = row1 + sx0.min(sub_w_us - 1) * 4;
            let c11 = row1 + sx1.min(sub_w_us - 1) * 4;

            let dst_off = dst_row + dx * 4;
            for c in 0..4 {
                let p00 = sub_pixels.get(c00 + c).copied().unwrap_or(0) as u32;
                let p10 = sub_pixels.get(c10 + c).copied().unwrap_or(0) as u32;
                let p01 = sub_pixels.get(c01 + c).copied().unwrap_or(0) as u32;
                let p11 = sub_pixels.get(c11 + c).copied().unwrap_or(0) as u32;

                let top = (p00 * ifx + p10 * fx) >> 16;
                let bottom = (p01 * ifx + p11 * fx) >> 16;
                let val = (top * ify + bottom * fy) >> 16;

                out[dst_off + c] = val.min(255) as u8;
            }
        }
    }

    out
}

/// Compresses an RGBA tile using downsampling (LoD) followed by secondary compression.
/// Returns (TILE_SUBCODEC_LOD, payload).
pub fn compress_tile_lod(
    raw_pixels: &[u8],
    w: u16,
    h: u16,
    lod: TileLodLevel,
    codec: &dyn crate::ScreenCodec,
) -> (u8, Vec<u8>) {
    if lod == TileLodLevel::Lod0 {
        return (TILE_SUBCODEC_RAW, raw_pixels.to_vec());
    }
    let (sub_pixels, sub_w, sub_h) = downsample_tile_rgba(raw_pixels, w, h, lod);
    let pal_cfg = ScreenTilePaletteConfig {
        enabled: true,
        max_colors: 16,
        enable_rle: true,
    };
    let (inner_codec, inner_payload) = compress_tile_palette(&sub_pixels, sub_w, sub_h, &pal_cfg);
    let (final_codec, final_payload) = if inner_codec != TILE_SUBCODEC_RAW {
        (inner_codec, inner_payload)
    } else {
        match codec.compress_frame(&sub_pixels, sub_w, sub_h) {
            Ok(comp) if comp.len() < sub_pixels.len() => (TILE_SUBCODEC_ZSTD, comp),
            _ => (TILE_SUBCODEC_RAW, sub_pixels),
        }
    };

    let mut payload = Vec::with_capacity(3 + final_payload.len());
    payload.push(TILE_SUBCODEC_LOD);
    payload.push(lod as u8);
    payload.push(final_codec);
    payload.extend_from_slice(&final_payload);
    (TILE_SUBCODEC_LOD, payload)
}

/// Decompresses an LoD subcodec tile payload and upsamples it back to (dst_w, dst_h).
pub fn decompress_tile_lod(
    payload: &[u8],
    dst_w: u16,
    dst_h: u16,
    _raw_len: usize,
    codec: &dyn crate::ScreenCodec,
) -> Result<Vec<u8>, CoreError> {
    if payload.len() < 3 || payload[0] != TILE_SUBCODEC_LOD {
        return Err(CoreError::ScreenFrame);
    }
    let lod = TileLodLevel::from_u8(payload[1]);
    let inner_codec = payload[2];
    let inner_data = &payload[3..];

    let factor = lod.scale_factor();
    let sub_w = (dst_w / factor).max(1);
    let sub_h = (dst_h / factor).max(1);
    let sub_raw_len = sub_w as usize * sub_h as usize * 4;

    let decompressed_sub = match inner_codec {
        TILE_SUBCODEC_SOLID | TILE_SUBCODEC_BILEVEL | TILE_SUBCODEC_PALETTE16 => {
            decompress_tile_palette(inner_codec, inner_data, sub_w, sub_h, sub_raw_len)?
        }
        TILE_SUBCODEC_ZSTD => codec
            .decompress_frame(inner_data, sub_w, sub_h, sub_raw_len)
            .map_err(|_| CoreError::ScreenFrame)?,
        TILE_SUBCODEC_RAW => {
            if inner_data.len() < sub_raw_len {
                return Err(CoreError::ScreenFrame);
            }
            inner_data[..sub_raw_len].to_vec()
        }
        _ => return Err(CoreError::ScreenFrame),
    };

    let upsampled = upsample_tile_rgba(&decompressed_sub, sub_w, sub_h, dst_w, dst_h);
    Ok(upsampled)
}

/// Runtime configuration for screen tile dynamic MTU batch chunking and pacing (v2.39 / ADR-125).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileBatchConfig {
    pub enabled: bool,
    pub target_chunk_bytes: u16, // Default 1200 bytes, range 512..=64000
    pub pacing_interval_us: u16, // Default 200 us, range 0..=50000
}

impl Default for ScreenTileBatchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            target_chunk_bytes: SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES,
            pacing_interval_us: SCREEN_TILE_BATCH_DEFAULT_PACING_US,
        }
    }
}

impl ScreenTileBatchConfig {
    pub fn new(
        enabled: bool,
        target_chunk_bytes: Option<u16>,
        pacing_interval_us: Option<u16>,
    ) -> Self {
        let chunk_bytes = target_chunk_bytes
            .unwrap_or(SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES)
            .clamp(
                SCREEN_TILE_BATCH_MIN_CHUNK_BYTES,
                SCREEN_TILE_BATCH_MAX_CHUNK_BYTES,
            );
        let pacing = pacing_interval_us
            .unwrap_or(SCREEN_TILE_BATCH_DEFAULT_PACING_US)
            .min(SCREEN_TILE_BATCH_MAX_PACING_US);
        Self {
            enabled,
            target_chunk_bytes: chunk_bytes,
            pacing_interval_us: pacing,
        }
    }
}

/// Comprehensive telemetry report for screen tile dynamic MTU batch chunking and pacing (v2.39 / ADR-125).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileBatchReport {
    pub ok: bool,
    pub enabled: bool,
    pub target_chunk_bytes: u16,
    pub pacing_interval_us: u16,
    pub batches_sent: u64,
    pub chunks_sent: u64,
    pub avg_chunk_bytes: u32,
    pub pacing_pauses: u64,
}

impl Default for ScreenTileBatchReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            target_chunk_bytes: SCREEN_TILE_BATCH_DEFAULT_CHUNK_BYTES,
            pacing_interval_us: SCREEN_TILE_BATCH_DEFAULT_PACING_US,
            batches_sent: 0,
            chunks_sent: 0,
            avg_chunk_bytes: 0,
            pacing_pauses: 0,
        }
    }
}

/// Chunks a collection of dirty tiles into multiple MTU-safe ScreenFrame chunks (v2.39 / ADR-125).
#[allow(clippy::too_many_arguments)]
pub fn chunk_dirty_tiles(
    seq: u64,
    screen_rect: ScreenRect,
    format: PixelFormat,
    tiles: Vec<DirtyTile>,
    target_chunk_bytes: usize,
    tile_size: u16,
    total_tiles: u16,
    batch_id: u32,
) -> Vec<ScreenFrame> {
    if tiles.is_empty() {
        return vec![ScreenFrame::encode_tile_batch_chunk(
            seq,
            screen_rect,
            format,
            batch_id,
            0,
            1,
            tile_size,
            total_tiles,
            &[],
        )];
    }

    let target = target_chunk_bytes.clamp(
        SCREEN_TILE_BATCH_MIN_CHUNK_BYTES as usize,
        SCREEN_TILE_BATCH_MAX_CHUNK_BYTES as usize,
    );

    let mut chunk_groups: Vec<Vec<DirtyTile>> = Vec::new();
    let mut current_group: Vec<DirtyTile> = Vec::new();
    let mut current_bytes = 14usize; // 14-byte batch chunk header

    for tile in tiles {
        let tile_bytes = 14usize + tile.data.len();
        if !current_group.is_empty() && current_bytes + tile_bytes > target {
            chunk_groups.push(std::mem::take(&mut current_group));
            current_bytes = 14usize;
        }
        current_bytes += tile_bytes;
        current_group.push(tile);
    }

    if !current_group.is_empty() {
        chunk_groups.push(current_group);
    }

    let total_chunks = chunk_groups.len() as u16;
    let mut frames = Vec::with_capacity(chunk_groups.len());
    for (idx, group) in chunk_groups.into_iter().enumerate() {
        frames.push(ScreenFrame::encode_tile_batch_chunk(
            seq,
            screen_rect,
            format,
            batch_id,
            idx as u16,
            total_chunks,
            tile_size,
            total_tiles,
            &group,
        ));
    }
    frames
}

/// Runtime configuration for screen tile local motion vector estimation & displacement compensation (v2.40 / ADR-126).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileMotionConfig {
    pub enabled: bool,
    pub max_displacement: u16, // Default 32, range 4..=128
    pub min_match_ratio: u8,   // Default 90, representing 90% (0.90)
}

impl Default for ScreenTileMotionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_displacement: SCREEN_TILE_MOTION_DEFAULT_MAX_DISPLACEMENT,
            min_match_ratio: SCREEN_TILE_MOTION_DEFAULT_MIN_MATCH_RATIO,
        }
    }
}

/// Telemetry report for screen tile motion vector compensation (v2.40 / ADR-126).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileMotionReport {
    pub ok: bool,
    pub enabled: bool,
    pub max_displacement: u16,
    pub min_match_ratio: u8,
    pub motion_tiles_sent: u64,
    pub fallback_tiles_sent: u64,
    pub bytes_saved: u64,
}

/// Estimates the best local motion vector (dx, dy) for a single dirty tile against a reference canvas (v2.40 / ADR-126).
/// Returns `Some((dx, dy, match_ratio))` if `match_ratio >= min_match_ratio`.
#[allow(clippy::too_many_arguments)]
pub fn estimate_tile_motion(
    tile_raw: &[u8],
    tile_x: u16,
    tile_y: u16,
    tile_w: u16,
    tile_h: u16,
    ref_canvas: &ScreenFrame,
    max_displacement: u16,
    min_match_ratio: f32,
) -> Option<(i16, i16, f32)> {
    if tile_w == 0 || tile_h == 0 || ref_canvas.bytes.is_empty() || ref_canvas.compressed() {
        return None;
    }
    let bpp = ref_canvas.format.bpp();
    let canvas_w = ref_canvas.rect.w as i32;
    let canvas_h = ref_canvas.rect.h as i32;
    let tile_stride = (tile_w as usize) * bpp;
    let canvas_stride = ref_canvas.effective_stride() as usize;

    if tile_raw.len() < (tile_h as usize) * tile_stride {
        return None;
    }

    let d_max = (max_displacement as i32).clamp(
        SCREEN_TILE_MOTION_MIN_DISPLACEMENT as i32,
        SCREEN_TILE_MOTION_MAX_DISPLACEMENT as i32,
    );

    let cur_x = tile_x as i32;
    let cur_y = tile_y as i32;
    let w_i32 = tile_w as i32;
    let h_i32 = tile_h as i32;

    // Sample points inside the tile to speed up matching (every 4th row, every 4th pixel)
    let row_step = 4.min(tile_h as usize);
    let col_step = 4.min(tile_w as usize);
    let sample_pixels = (tile_h as usize).div_ceil(row_step) * (tile_w as usize).div_ceil(col_step);

    if sample_pixels == 0 {
        return None;
    }

    let mut best_dx = 0i16;
    let mut best_dy = 0i16;
    let mut best_ratio = 0.0f32;

    let check_candidate = |dx: i32, dy: i32| -> Option<f32> {
        let ref_x = cur_x - dx;
        let ref_y = cur_y - dy;
        if ref_x < 0 || ref_y < 0 || ref_x + w_i32 > canvas_w || ref_y + h_i32 > canvas_h {
            return None;
        }

        let mut matched = 0usize;
        for r in (0..tile_h as usize).step_by(row_step) {
            let cur_row_off = r * tile_stride;
            let ref_row_off = (ref_y as usize + r) * canvas_stride + (ref_x as usize) * bpp;
            for c in (0..tile_w as usize).step_by(col_step) {
                let cur_p = cur_row_off + c * bpp;
                let ref_p = ref_row_off + c * bpp;
                if tile_raw[cur_p..cur_p + bpp] == ref_canvas.bytes[ref_p..ref_p + bpp] {
                    matched += 1;
                }
            }
        }
        Some((matched as f32) / (sample_pixels as f32))
    };

    // 1. Search vertical displacements: dx = 0, dy in [-d_max, d_max] \ {0}
    for dy in (-d_max..=d_max).filter(|&d| d != 0) {
        if let Some(ratio) = check_candidate(0, dy) {
            if ratio > best_ratio {
                best_ratio = ratio;
                best_dx = 0;
                best_dy = dy as i16;
                if best_ratio >= 0.98 {
                    break;
                }
            }
        }
    }

    // 2. If vertical didn't find high confidence match, search horizontal displacements: dy = 0, dx in [-d_max, d_max] \ {0}
    if best_ratio < min_match_ratio {
        for dx in (-d_max..=d_max).filter(|&d| d != 0) {
            if let Some(ratio) = check_candidate(dx, 0) {
                if ratio > best_ratio {
                    best_ratio = ratio;
                    best_dx = dx as i16;
                    best_dy = 0;
                    if best_ratio >= 0.98 {
                        break;
                    }
                }
            }
        }
    }

    // 3. If still below threshold, search coarse 2D diagonals (step 2)
    if best_ratio < min_match_ratio {
        for dy in (-d_max..=d_max).step_by(2).filter(|&d| d != 0) {
            for dx in (-d_max..=d_max).step_by(2).filter(|&d| d != 0) {
                if let Some(ratio) = check_candidate(dx, dy) {
                    if ratio > best_ratio {
                        best_ratio = ratio;
                        best_dx = dx as i16;
                        best_dy = dy as i16;
                        if best_ratio >= 0.98 {
                            break;
                        }
                    }
                }
            }
            if best_ratio >= 0.98 {
                break;
            }
        }
    }

    if best_ratio >= min_match_ratio {
        Some((best_dx, best_dy, best_ratio))
    } else {
        None
    }
}

/// Compresses a dirty tile using local motion estimation against reference canvas (v2.40 / ADR-126).
/// Returns `Some((TILE_SUBCODEC_MOTION, payload))` where payload is 5 bytes: [subcodec(1) | dx(2) | dy(2)].
pub fn compress_tile_motion(
    tile_raw: &[u8],
    tile_x: u16,
    tile_y: u16,
    tile_w: u16,
    tile_h: u16,
    ref_canvas: &ScreenFrame,
    config: &ScreenTileMotionConfig,
) -> Option<(u8, Vec<u8>)> {
    if !config.enabled {
        return None;
    }
    let min_ratio = (config.min_match_ratio as f32) / 100.0;
    let (dx, dy, _) = estimate_tile_motion(
        tile_raw,
        tile_x,
        tile_y,
        tile_w,
        tile_h,
        ref_canvas,
        config.max_displacement,
        min_ratio,
    )?;

    let mut payload = Vec::with_capacity(5);
    payload.push(TILE_SUBCODEC_MOTION);
    payload.extend_from_slice(&dx.to_be_bytes());
    payload.extend_from_slice(&dy.to_be_bytes());
    Some((TILE_SUBCODEC_MOTION, payload))
}

/// Decompresses a motion-compressed dirty tile by extracting shifted pixels from reference canvas (v2.40 / ADR-126).
pub fn decompress_tile_motion(
    payload: &[u8],
    tile_x: u16,
    tile_y: u16,
    tile_w: u16,
    tile_h: u16,
    ref_canvas: &ScreenFrame,
) -> Result<Vec<u8>, CoreError> {
    if payload.len() < 5 || payload[0] != TILE_SUBCODEC_MOTION {
        return Err(CoreError::ScreenFrame);
    }
    if ref_canvas.bytes.is_empty() || ref_canvas.compressed() {
        return Err(CoreError::ScreenFrame);
    }
    let dx = i16::from_be_bytes(payload[1..3].try_into().unwrap()) as i32;
    let dy = i16::from_be_bytes(payload[3..5].try_into().unwrap()) as i32;

    let bpp = ref_canvas.format.bpp();
    let canvas_w = ref_canvas.rect.w as i32;
    let canvas_h = ref_canvas.rect.h as i32;
    let canvas_stride = ref_canvas.effective_stride() as usize;

    let ref_x = (tile_x as i32) - dx;
    let ref_y = (tile_y as i32) - dy;
    let w_i32 = tile_w as i32;
    let h_i32 = tile_h as i32;

    if ref_x < 0 || ref_y < 0 || ref_x + w_i32 > canvas_w || ref_y + h_i32 > canvas_h {
        return Err(CoreError::ScreenFrame);
    }

    let tile_stride = (tile_w as usize) * bpp;
    let mut decomp = vec![0u8; (tile_h as usize) * tile_stride];

    for r in 0..tile_h as usize {
        let src_off = (ref_y as usize + r) * canvas_stride + (ref_x as usize) * bpp;
        let dst_off = r * tile_stride;
        decomp[dst_off..dst_off + tile_stride]
            .copy_from_slice(&ref_canvas.bytes[src_off..src_off + tile_stride]);
    }

    Ok(decomp)
}
