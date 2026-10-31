// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Dirty-region damage detection and bounding-rect reporting.

use super::*;
use serde::Deserialize;
use serde::Serialize;

/// Bounding rectangle for detected dirty regions on screen (v2.03 / ADR-089).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ScreenDamageRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl ScreenDamageRect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Summary report of dirty region evaluation across consecutive screen frames (v2.03 / ADR-089).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ScreenDamageReport {
    pub dirty_pixels: u64,
    pub total_pixels: u64,
    pub dirty_ratio: f32,
    pub num_dirty_blocks: usize,
    pub total_blocks: usize,
    pub is_static: bool,
    pub bounding_rect: Option<ScreenDamageRect>,
}

impl ScreenDamageReport {
    pub fn is_dirty(&self) -> bool {
        !self.is_static
    }

    pub fn rect(&self) -> Option<ScreenDamageRect> {
        self.bounding_rect
    }
}

/// Grid-based dirty block damage detector for screen frames (v2.03 / ADR-089).
#[derive(Debug, Clone, Default)]
pub struct ScreenDamageTracker {
    pub(super) has_prior: bool,
}

impl ScreenDamageTracker {
    pub fn new() -> Self {
        Self { has_prior: false }
    }

    /// Evaluates damage between two RGBA8 frames of given width and height.
    /// Operates on 32x32 pixel macroblocks for fast 64-bit comparison.
    pub fn evaluate_rgba(
        &mut self,
        prev_rgba: &[u8],
        curr_rgba: &[u8],
        width: u32,
        height: u32,
    ) -> ScreenDamageReport {
        let total_pixels = (width as u64) * (height as u64);
        if total_pixels == 0
            || prev_rgba.len() != curr_rgba.len()
            || prev_rgba.len() < (total_pixels as usize) * 4
        {
            return ScreenDamageReport {
                total_pixels,
                dirty_pixels: total_pixels,
                dirty_ratio: 1.0,
                num_dirty_blocks: 1,
                total_blocks: 1,
                is_static: false,
                bounding_rect: Some(ScreenDamageRect {
                    x: 0,
                    y: 0,
                    width: width as u16,
                    height: height as u16,
                }),
            };
        }

        let block_w = SCREEN_DAMAGE_BLOCK_SIZE as u32;
        let block_h = SCREEN_DAMAGE_BLOCK_SIZE as u32;
        let grid_cols = width.div_ceil(block_w);
        let grid_rows = height.div_ceil(block_h);
        let total_blocks = (grid_cols * grid_rows) as usize;

        let mut num_dirty_blocks = 0usize;
        let mut dirty_pixels = 0u64;
        let mut min_bx = u32::MAX;
        let mut max_bx = 0u32;
        let mut min_by = u32::MAX;
        let mut max_by = 0u32;

        let stride = (width * 4) as usize;

        for by in 0..grid_rows {
            let y_start = by * block_h;
            let y_end = (y_start + block_h).min(height);
            let bh = y_end - y_start;

            for bx in 0..grid_cols {
                let x_start = bx * block_w;
                let x_end = (x_start + block_w).min(width);
                let bw = x_end - x_start;
                let row_bytes = (bw * 4) as usize;

                let mut block_dirty = false;
                for y in y_start..y_end {
                    let offset = (y as usize) * stride + (x_start as usize) * 4;
                    if prev_rgba[offset..offset + row_bytes]
                        != curr_rgba[offset..offset + row_bytes]
                    {
                        block_dirty = true;
                        break;
                    }
                }

                if block_dirty {
                    num_dirty_blocks += 1;
                    dirty_pixels += (bw as u64) * (bh as u64);
                    min_bx = min_bx.min(x_start);
                    max_bx = max_bx.max(x_end);
                    min_by = min_by.min(y_start);
                    max_by = max_by.max(y_end);
                }
            }
        }

        let is_static = num_dirty_blocks == 0;
        let dirty_ratio = if total_pixels > 0 {
            (dirty_pixels as f32) / (total_pixels as f32)
        } else {
            0.0
        };

        let bounding_rect = if !is_static && min_bx < max_bx && min_by < max_by {
            Some(ScreenDamageRect {
                x: min_bx as u16,
                y: min_by as u16,
                width: (max_bx - min_bx) as u16,
                height: (max_by - min_by) as u16,
            })
        } else {
            None
        };

        self.has_prior = true;

        ScreenDamageReport {
            dirty_pixels,
            total_pixels,
            dirty_ratio,
            num_dirty_blocks,
            total_blocks,
            is_static,
            bounding_rect,
        }
    }

    /// Evaluates damage based on an explicit boolean change hint (e.g. from capture driver or mock).
    pub fn evaluate_hint(&mut self, is_dirty: bool, width: u32, height: u32) -> ScreenDamageReport {
        let total_pixels = (width as u64) * (height as u64);
        let block_w = SCREEN_DAMAGE_BLOCK_SIZE as u32;
        let block_h = SCREEN_DAMAGE_BLOCK_SIZE as u32;
        let grid_cols = width.div_ceil(block_w);
        let grid_rows = height.div_ceil(block_h);
        let total_blocks = (grid_cols * grid_rows) as usize;

        if is_dirty {
            ScreenDamageReport {
                dirty_pixels: total_pixels,
                total_pixels,
                dirty_ratio: 1.0,
                num_dirty_blocks: total_blocks,
                total_blocks,
                is_static: false,
                bounding_rect: Some(ScreenDamageRect {
                    x: 0,
                    y: 0,
                    width: width as u16,
                    height: height as u16,
                }),
            }
        } else {
            ScreenDamageReport {
                dirty_pixels: 0,
                total_pixels,
                dirty_ratio: 0.0,
                num_dirty_blocks: 0,
                total_blocks,
                is_static: true,
                bounding_rect: None,
            }
        }
    }
}
