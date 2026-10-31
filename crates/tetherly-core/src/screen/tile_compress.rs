// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Tile palette/bilevel and predictive-filter lossless codecs.

use super::*;
use crate::error::CoreError;

/// Screen streaming tile-level adaptive palette configuration (v2.33 / ADR-119).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTilePaletteConfig {
    pub enabled: bool,
    pub max_colors: u8,
    pub enable_rle: bool,
}

impl Default for ScreenTilePaletteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_colors: SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS,
            enable_rle: false,
        }
    }
}

impl ScreenTilePaletteConfig {
    pub fn new(enabled: bool, max_colors: Option<u8>, enable_rle: Option<bool>) -> Self {
        let max_c = max_colors.unwrap_or(SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS);
        let clamped = max_c.clamp(2, 16);
        Self {
            enabled,
            max_colors: clamped,
            enable_rle: enable_rle.unwrap_or(false),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Screen streaming tile-level adaptive palette telemetry report (v2.33 / ADR-119).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTilePaletteReport {
    pub ok: bool,
    pub enabled: bool,
    pub max_colors: u8,
    pub enable_rle: bool,
    pub solid_tiles_count: u64,
    pub bilevel_tiles_count: u64,
    pub palette_tiles_count: u64,
    pub truecolor_tiles_count: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenTilePaletteReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            max_colors: SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS,
            enable_rle: false,
            solid_tiles_count: 0,
            bilevel_tiles_count: 0,
            palette_tiles_count: 0,
            truecolor_tiles_count: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// Compresses a 32-bit RGBA dirty tile into an adaptive palette or bi-level representation (v2.33 / ADR-119).
/// Returns (subcodec, encoded_payload).
/// If the tile has > max_colors (default 16), early-aborts in < 2us and returns (TILE_SUBCODEC_RAW, raw_pixels.to_vec()).
pub fn compress_tile_palette(
    raw_pixels: &[u8],
    w: u16,
    h: u16,
    config: &ScreenTilePaletteConfig,
) -> (u8, Vec<u8>) {
    let total_pixels = (w as usize) * (h as usize);
    if raw_pixels.len() != total_pixels * 4 || total_pixels == 0 || !config.enabled {
        return (TILE_SUBCODEC_RAW, raw_pixels.to_vec());
    }

    let max_colors = (config.max_colors as usize).clamp(2, 16);
    let mut palette = [0u32; 16];
    let mut color_count = 0usize;

    for chunk in raw_pixels.chunks_exact(4) {
        let color = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let found = palette[..color_count].contains(&color);
        if !found {
            if color_count >= max_colors {
                return (TILE_SUBCODEC_RAW, raw_pixels.to_vec());
            }
            palette[color_count] = color;
            color_count += 1;
        }
    }

    if color_count == 1 {
        // Solid Color Tile: 1 byte subcodec + 4 bytes color (Total 5 bytes)
        let mut payload = Vec::with_capacity(5);
        payload.push(TILE_SUBCODEC_SOLID);
        payload.extend_from_slice(&palette[0].to_ne_bytes());
        (TILE_SUBCODEC_SOLID, payload)
    } else if color_count == 2 {
        // Bi-Level Bitmap Tile: 1 byte subcodec + 8 bytes (C0, C1) + 1-bit packed bitmap
        let bitmap_bytes = total_pixels.div_ceil(8);
        let mut payload = Vec::with_capacity(1 + 8 + bitmap_bytes);
        payload.push(TILE_SUBCODEC_BILEVEL);
        payload.extend_from_slice(&palette[0].to_ne_bytes());
        payload.extend_from_slice(&palette[1].to_ne_bytes());
        let c1 = palette[1];
        let mut cur_byte = 0u8;
        let mut bit_idx = 0usize;
        for chunk in raw_pixels.chunks_exact(4) {
            let color = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if color == c1 {
                cur_byte |= 1 << (7 - bit_idx);
            }
            bit_idx += 1;
            if bit_idx == 8 {
                payload.push(cur_byte);
                cur_byte = 0;
                bit_idx = 0;
            }
        }
        if bit_idx > 0 {
            payload.push(cur_byte);
        }
        (TILE_SUBCODEC_BILEVEL, payload)
    } else {
        // Indexed Palette (3..=16 colors): 1 byte subcodec + 1 byte count + K*4 bytes palette + 4-bit packed nibbles
        let nibble_bytes = total_pixels.div_ceil(2);
        let mut payload = Vec::with_capacity(2 + color_count * 4 + nibble_bytes);
        payload.push(TILE_SUBCODEC_PALETTE16);
        payload.push(color_count as u8);
        for color in &palette[..color_count] {
            payload.extend_from_slice(&color.to_ne_bytes());
        }
        let mut high_nibble = true;
        let mut cur_byte = 0u8;
        for chunk in raw_pixels.chunks_exact(4) {
            let color = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let idx = palette[..color_count]
                .iter()
                .position(|&c| c == color)
                .unwrap_or(0) as u8;
            if high_nibble {
                cur_byte = (idx & 0x0F) << 4;
                high_nibble = false;
            } else {
                cur_byte |= idx & 0x0F;
                payload.push(cur_byte);
                cur_byte = 0;
                high_nibble = true;
            }
        }
        if !high_nibble {
            payload.push(cur_byte);
        }
        (TILE_SUBCODEC_PALETTE16, payload)
    }
}

/// Decompresses an adaptive palette, bi-level bitmap or solid color tile back into raw 32-bit RGBA pixels (v2.33 / ADR-119).
pub fn decompress_tile_palette(
    subcodec: u8,
    payload: &[u8],
    w: u16,
    h: u16,
    expected_len: usize,
) -> Result<Vec<u8>, CoreError> {
    let total_pixels = (w as usize) * (h as usize);
    let target_len = total_pixels * 4;
    if target_len != expected_len && expected_len != 0 {
        return Err(CoreError::ScreenFrame);
    }
    let actual_expected = if expected_len == 0 {
        target_len
    } else {
        expected_len
    };

    match subcodec {
        TILE_SUBCODEC_SOLID => {
            let color_bytes = if payload.len() == 5 && payload[0] == TILE_SUBCODEC_SOLID {
                &payload[1..5]
            } else if payload.len() >= 4 {
                &payload[0..4]
            } else {
                return Err(CoreError::ScreenFrame);
            };
            let mut out = Vec::with_capacity(actual_expected);
            for _ in 0..total_pixels {
                out.extend_from_slice(color_bytes);
            }
            Ok(out)
        }
        TILE_SUBCODEC_BILEVEL => {
            let offset = if payload.len() >= 9 && payload[0] == TILE_SUBCODEC_BILEVEL {
                1
            } else {
                0
            };
            if payload.len() < offset + 8 {
                return Err(CoreError::ScreenFrame);
            }
            let c0 = &payload[offset..offset + 4];
            let c1 = &payload[offset + 4..offset + 8];
            let bitmap = &payload[offset + 8..];
            let mut out = Vec::with_capacity(actual_expected);
            for p in 0..total_pixels {
                let byte_idx = p / 8;
                let bit_idx = p % 8;
                if byte_idx >= bitmap.len() {
                    return Err(CoreError::ScreenFrame);
                }
                let bit = (bitmap[byte_idx] >> (7 - bit_idx)) & 1;
                if bit == 0 {
                    out.extend_from_slice(c0);
                } else {
                    out.extend_from_slice(c1);
                }
            }
            Ok(out)
        }
        TILE_SUBCODEC_PALETTE16 => {
            let offset = if payload.len() >= 2 && payload[0] == TILE_SUBCODEC_PALETTE16 {
                1
            } else {
                0
            };
            if payload.len() < offset + 1 {
                return Err(CoreError::ScreenFrame);
            }
            let count = payload[offset] as usize;
            if !(1..=16).contains(&count) || payload.len() < offset + 1 + count * 4 {
                return Err(CoreError::ScreenFrame);
            }
            let pal_start = offset + 1;
            let nibbles_start = pal_start + count * 4;
            let nibbles = &payload[nibbles_start..];
            let mut out = Vec::with_capacity(actual_expected);
            for p in 0..total_pixels {
                let byte_idx = p / 2;
                if byte_idx >= nibbles.len() {
                    return Err(CoreError::ScreenFrame);
                }
                let idx = if (p % 2) == 0 {
                    (nibbles[byte_idx] >> 4) & 0x0F
                } else {
                    nibbles[byte_idx] & 0x0F
                } as usize;
                if idx >= count {
                    return Err(CoreError::ScreenFrame);
                }
                let color_off = pal_start + idx * 4;
                out.extend_from_slice(&payload[color_off..color_off + 4]);
            }
            Ok(out)
        }
        TILE_SUBCODEC_RAW => Ok(payload.to_vec()),
        _ => Err(CoreError::ScreenFrame),
    }
}

/// Screen streaming tile-level predictive filter mode (v2.34 / ADR-120).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ScreenTilePredictFilter {
    None = 0,
    Sub = 1,
    Up = 2,
    Average = 3,
    Paeth = 4,
    #[default]
    Auto = 5,
}

impl ScreenTilePredictFilter {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenTilePredictFilter::None => "none",
            ScreenTilePredictFilter::Sub => "sub",
            ScreenTilePredictFilter::Up => "up",
            ScreenTilePredictFilter::Average => "average",
            ScreenTilePredictFilter::Paeth => "paeth",
            ScreenTilePredictFilter::Auto => "auto",
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => ScreenTilePredictFilter::None,
            1 => ScreenTilePredictFilter::Sub,
            2 => ScreenTilePredictFilter::Up,
            3 => ScreenTilePredictFilter::Average,
            4 => ScreenTilePredictFilter::Paeth,
            _ => ScreenTilePredictFilter::Auto,
        }
    }

    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "0" | "off" => Some(ScreenTilePredictFilter::None),
            "sub" | "1" | "left" | "h" | "horizontal" => Some(ScreenTilePredictFilter::Sub),
            "up" | "2" | "top" | "v" | "vertical" => Some(ScreenTilePredictFilter::Up),
            "average" | "avg" | "3" | "mean" => Some(ScreenTilePredictFilter::Average),
            "paeth" | "4" | "diag" | "gradient" => Some(ScreenTilePredictFilter::Paeth),
            "auto" | "5" | "adaptive" | "best" => Some(ScreenTilePredictFilter::Auto),
            _ => None,
        }
    }
}

impl std::str::FromStr for ScreenTilePredictFilter {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_loose(s).ok_or_else(|| {
            CoreError::InvalidConfig(format!("unknown screen tile predict filter mode: '{s}'"))
        })
    }
}

/// Screen streaming tile-level predictive filter configuration (v2.34 / ADR-120).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTilePredictConfig {
    pub enabled: bool,
    pub filter_mode: ScreenTilePredictFilter,
    pub enable_rct: bool,
}

impl Default for ScreenTilePredictConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            filter_mode: ScreenTilePredictFilter::Auto,
            enable_rct: true,
        }
    }
}

impl ScreenTilePredictConfig {
    pub fn new(
        enabled: bool,
        filter_mode: Option<ScreenTilePredictFilter>,
        enable_rct: Option<bool>,
    ) -> Self {
        Self {
            enabled,
            filter_mode: filter_mode.unwrap_or(ScreenTilePredictFilter::Auto),
            enable_rct: enable_rct.unwrap_or(true),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Screen streaming tile-level predictive filter telemetry report (v2.34 / ADR-120).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTilePredictReport {
    pub ok: bool,
    pub enabled: bool,
    pub filter_mode: ScreenTilePredictFilter,
    pub enable_rct: bool,
    pub predict_tiles_count: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenTilePredictReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            filter_mode: ScreenTilePredictFilter::Auto,
            enable_rct: true,
            predict_tiles_count: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// In-place / forward Reversible Color Transform (RCT) for 32-bit RGBA (v2.34 / ADR-120).
/// Transforms (R, G, B, A) into (R - G, G, B - G, A) in Z/256Z.
pub fn rct_transform(pixels: &[u8], out: &mut [u8]) {
    assert_eq!(pixels.len(), out.len());
    for (src, dst) in pixels.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let r = src[0];
        let g = src[1];
        let b = src[2];
        let a = src[3];
        dst[0] = r.wrapping_sub(g);
        dst[1] = g;
        dst[2] = b.wrapping_sub(g);
        dst[3] = a;
    }
}

/// In-place / inverse Reversible Color Transform (RCT) for 32-bit RGBA (v2.34 / ADR-120).
/// Reconstructs (R, G, B, A) from (U, G, V, A) where R = U + G, B = V + G in Z/256Z.
pub fn rct_inverse(transformed: &[u8], out: &mut [u8]) {
    assert_eq!(transformed.len(), out.len());
    for (src, dst) in transformed.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
        let u = src[0];
        let g = src[1];
        let v = src[2];
        let a = src[3];
        dst[0] = u.wrapping_add(g);
        dst[1] = g;
        dst[2] = v.wrapping_add(g);
        dst[3] = a;
    }
}

/// Standard 2D Paeth predictor for spatial differential prediction (v2.34 / ADR-120).
#[inline(always)]
pub fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a_i = a as i32;
    let b_i = b as i32;
    let c_i = c as i32;
    let p = a_i + b_i - c_i;
    let pa = (p - a_i).abs();
    let pb = (p - b_i).abs();
    let pc = (p - c_i).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

pub(super) fn apply_single_filter(
    pixels: &[u8],
    w: u16,
    h: u16,
    filter: ScreenTilePredictFilter,
) -> Vec<u8> {
    let expected = (w as usize) * (h as usize) * 4;
    let mut out = vec![0u8; expected];
    if filter == ScreenTilePredictFilter::None {
        out.copy_from_slice(pixels);
        return out;
    }
    let stride = (w as usize) * 4;
    for row in 0..h as usize {
        let row_off = row * stride;
        for col in 0..w as usize {
            let idx = row_off + col * 4;
            for ch in 0..4 {
                let curr = pixels[idx + ch];
                let a = if col > 0 { pixels[idx - 4 + ch] } else { 0 };
                let b = if row > 0 {
                    pixels[row_off - stride + col * 4 + ch]
                } else {
                    0
                };
                let c = if col > 0 && row > 0 {
                    pixels[row_off - stride + (col - 1) * 4 + ch]
                } else {
                    0
                };
                let pred = match filter {
                    ScreenTilePredictFilter::None | ScreenTilePredictFilter::Auto => 0,
                    ScreenTilePredictFilter::Sub => a,
                    ScreenTilePredictFilter::Up => b,
                    ScreenTilePredictFilter::Average => (((a as u16) + (b as u16)) / 2) as u8,
                    ScreenTilePredictFilter::Paeth => paeth_predictor(a, b, c),
                };
                out[idx + ch] = curr.wrapping_sub(pred);
            }
        }
    }
    out
}

/// Applies directional spatial differential predictive filtering on a 32-bit RGBA tile (v2.34 / ADR-120).
/// In Auto mode, evaluates L1 residual energies across candidates and picks the optimal filter.
/// Returns (chosen_filter, residual_bytes).
pub fn predict_filter_tile(
    pixels: &[u8],
    w: u16,
    h: u16,
    filter: ScreenTilePredictFilter,
) -> (ScreenTilePredictFilter, Vec<u8>) {
    let expected = (w as usize) * (h as usize) * 4;
    if pixels.len() != expected || w == 0 || h == 0 {
        return (ScreenTilePredictFilter::None, pixels.to_vec());
    }

    if filter != ScreenTilePredictFilter::Auto {
        return (filter, apply_single_filter(pixels, w, h, filter));
    }

    // Auto Mode: evaluate L1 residual energies across None, Sub, Up, Average, Paeth
    let stride = (w as usize) * 4;
    let mut energy_none = 0u64;
    let mut energy_sub = 0u64;
    let mut energy_up = 0u64;
    let mut energy_avg = 0u64;
    let mut energy_paeth = 0u64;

    for row in 0..h as usize {
        let row_off = row * stride;
        for col in 0..w as usize {
            let idx = row_off + col * 4;
            for ch in 0..4 {
                let curr = pixels[idx + ch];
                let a = if col > 0 { pixels[idx - 4 + ch] } else { 0 };
                let b = if row > 0 {
                    pixels[row_off - stride + col * 4 + ch]
                } else {
                    0
                };
                let c = if col > 0 && row > 0 {
                    pixels[row_off - stride + (col - 1) * 4 + ch]
                } else {
                    0
                };

                let d_none = curr as i8;
                energy_none += (d_none as i16).unsigned_abs() as u64;

                let d_sub = curr.wrapping_sub(a) as i8;
                energy_sub += (d_sub as i16).unsigned_abs() as u64;

                let d_up = curr.wrapping_sub(b) as i8;
                energy_up += (d_up as i16).unsigned_abs() as u64;

                let avg = (((a as u16) + (b as u16)) / 2) as u8;
                let d_avg = curr.wrapping_sub(avg) as i8;
                energy_avg += (d_avg as i16).unsigned_abs() as u64;

                let p_val = paeth_predictor(a, b, c);
                let d_paeth = curr.wrapping_sub(p_val) as i8;
                energy_paeth += (d_paeth as i16).unsigned_abs() as u64;
            }
        }
    }

    let mut best_filter = ScreenTilePredictFilter::None;
    let mut min_energy = energy_none;

    if energy_sub < min_energy {
        min_energy = energy_sub;
        best_filter = ScreenTilePredictFilter::Sub;
    }
    if energy_up < min_energy {
        min_energy = energy_up;
        best_filter = ScreenTilePredictFilter::Up;
    }
    if energy_avg < min_energy {
        min_energy = energy_avg;
        best_filter = ScreenTilePredictFilter::Average;
    }
    if energy_paeth < min_energy {
        best_filter = ScreenTilePredictFilter::Paeth;
    }

    (best_filter, apply_single_filter(pixels, w, h, best_filter))
}

/// Unpredicts / inverts spatial differential prediction back to raw pixels (v2.34 / ADR-120).
pub fn unpredict_filter_tile(
    residual: &[u8],
    w: u16,
    h: u16,
    filter: ScreenTilePredictFilter,
) -> Vec<u8> {
    let expected = (w as usize) * (h as usize) * 4;
    if residual.len() != expected || w == 0 || h == 0 {
        return residual.to_vec();
    }
    if filter == ScreenTilePredictFilter::None {
        return residual.to_vec();
    }

    let stride = (w as usize) * 4;
    let mut out = vec![0u8; expected];

    for row in 0..h as usize {
        let row_off = row * stride;
        for col in 0..w as usize {
            let idx = row_off + col * 4;
            for ch in 0..4 {
                let delta = residual[idx + ch];
                let a = if col > 0 { out[idx - 4 + ch] } else { 0 };
                let b = if row > 0 {
                    out[row_off - stride + col * 4 + ch]
                } else {
                    0
                };
                let c = if col > 0 && row > 0 {
                    out[row_off - stride + (col - 1) * 4 + ch]
                } else {
                    0
                };

                let pred = match filter {
                    ScreenTilePredictFilter::None | ScreenTilePredictFilter::Auto => 0,
                    ScreenTilePredictFilter::Sub => a,
                    ScreenTilePredictFilter::Up => b,
                    ScreenTilePredictFilter::Average => (((a as u16) + (b as u16)) / 2) as u8,
                    ScreenTilePredictFilter::Paeth => paeth_predictor(a, b, c),
                };
                out[idx + ch] = delta.wrapping_add(pred);
            }
        }
    }
    out
}

/// Compresses a 32-bit RGBA dirty tile using Reversible Color Transform, Directional Spatial Prediction
/// and generic residual entropy coding (v2.34 / ADR-120).
/// Returns (subcodec, encoded_payload).
pub fn compress_tile_predictive(
    raw_pixels: &[u8],
    w: u16,
    h: u16,
    config: &ScreenTilePredictConfig,
    codec: &dyn crate::ScreenCodec,
) -> (u8, Vec<u8>) {
    let total_pixels = (w as usize) * (h as usize);
    if raw_pixels.len() != total_pixels * 4 || total_pixels == 0 || !config.enabled {
        return (TILE_SUBCODEC_RAW, raw_pixels.to_vec());
    }

    let mut rct_buf = vec![0u8; raw_pixels.len()];
    let input = if config.enable_rct {
        rct_transform(raw_pixels, &mut rct_buf);
        &rct_buf[..]
    } else {
        raw_pixels
    };

    let (chosen_filter, residual) = predict_filter_tile(input, w, h, config.filter_mode);

    let compressed_res = match codec.compress_frame(&residual, w, h) {
        Ok(c) => c,
        Err(_) => return (TILE_SUBCODEC_RAW, raw_pixels.to_vec()),
    };

    let mut payload = Vec::with_capacity(3 + compressed_res.len());
    payload.push(TILE_SUBCODEC_PREDICT_ZSTD);
    payload.push(chosen_filter.to_u8());
    let flags = if config.enable_rct { 1u8 } else { 0u8 };
    payload.push(flags);
    payload.extend_from_slice(&compressed_res);

    (TILE_SUBCODEC_PREDICT_ZSTD, payload)
}

/// Decompresses a predictive filtered & RCT transformed tile back into raw 32-bit RGBA pixels (v2.34 / ADR-120).
pub fn decompress_tile_predictive(
    payload: &[u8],
    w: u16,
    h: u16,
    expected_len: usize,
    codec: &dyn crate::ScreenCodec,
) -> Result<Vec<u8>, CoreError> {
    let total_pixels = (w as usize) * (h as usize);
    let target_len = total_pixels * 4;
    if target_len != expected_len && expected_len != 0 {
        return Err(CoreError::ScreenFrame);
    }
    let actual_expected = if expected_len == 0 {
        target_len
    } else {
        expected_len
    };

    if payload.len() < 3 || payload[0] != TILE_SUBCODEC_PREDICT_ZSTD {
        return Err(CoreError::ScreenFrame);
    }
    let filter = ScreenTilePredictFilter::from_u8(payload[1]);
    let enable_rct = (payload[2] & 1) != 0;
    let compressed_residual = &payload[3..];

    let residual = codec.decompress_frame(compressed_residual, w, h, actual_expected)?;
    if residual.len() != actual_expected {
        return Err(CoreError::ScreenFrame);
    }

    let unfiltered = unpredict_filter_tile(&residual, w, h, filter);
    if unfiltered.len() != actual_expected {
        return Err(CoreError::ScreenFrame);
    }

    if enable_rct {
        let mut out = vec![0u8; actual_expected];
        rct_inverse(&unfiltered, &mut out);
        Ok(out)
    } else {
        Ok(unfiltered)
    }
}
