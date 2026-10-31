// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Scroll motion-vector estimation and displacement compensation.

use super::*;

/// Estimated 2D motion vector between consecutive screen frames (v2.29 / ADR-115).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionVector {
    pub dx: i16,
    pub dy: i16,
    pub match_ratio: f32,
}

/// Screen 2D Motion Vector Estimation and Scroll Acceleration Configuration (v2.29 / ADR-115).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenMotionConfig {
    pub enabled: bool,
    pub max_displacement: u16,
}

impl Default for ScreenMotionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_displacement: SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT,
        }
    }
}

impl ScreenMotionConfig {
    pub fn new(enabled: bool, max_displacement: u16) -> Self {
        Self {
            enabled,
            max_displacement: if max_displacement == 0 {
                SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT
            } else {
                max_displacement.clamp(
                    SCREEN_MOTION_MIN_DISPLACEMENT,
                    SCREEN_MOTION_MAX_DISPLACEMENT,
                )
            },
        }
    }
}

/// Screen 2D Motion Vector Telemetry Report (v2.29 / ADR-115).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenMotionReport {
    pub ok: bool,
    pub enabled: bool,
    pub max_displacement: u16,
    pub motion_frames_sent: u64,
    pub fallback_frames_sent: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenMotionReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            max_displacement: SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT,
            motion_frames_sent: 0,
            fallback_frames_sent: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// Computes the exposed edge strip rectangle newly revealed by scroll displacement (dx, dy) (v2.29 / ADR-115).
pub fn compute_exposed_strip(w: u16, h: u16, dx: i16, dy: i16) -> Option<ScreenDamageRect> {
    if w == 0 || h == 0 || (dx == 0 && dy == 0) {
        return None;
    }
    if dy != 0 && dx == 0 {
        let abs_dy = dy.unsigned_abs().min(h);
        if dy < 0 {
            // Shift up: exposed strip is at the bottom
            let y = h.saturating_sub(abs_dy);
            Some(ScreenDamageRect {
                x: 0,
                y,
                width: w,
                height: abs_dy,
            })
        } else {
            // Shift down: exposed strip is at the top
            Some(ScreenDamageRect {
                x: 0,
                y: 0,
                width: w,
                height: abs_dy,
            })
        }
    } else if dx != 0 && dy == 0 {
        let abs_dx = dx.unsigned_abs().min(w);
        if dx < 0 {
            // Shift left: exposed strip is at the right
            let x = w.saturating_sub(abs_dx);
            Some(ScreenDamageRect {
                x,
                y: 0,
                width: abs_dx,
                height: h,
            })
        } else {
            // Shift right: exposed strip is at the left
            Some(ScreenDamageRect {
                x: 0,
                y: 0,
                width: abs_dx,
                height: h,
            })
        }
    } else {
        // 2D diagonal: cover union bounding rect
        let x = if dx > 0 {
            0
        } else {
            w.saturating_sub(dx.unsigned_abs())
        };
        let y = if dy > 0 {
            0
        } else {
            h.saturating_sub(dy.unsigned_abs())
        };
        let width = if dx != 0 { dx.unsigned_abs() } else { w };
        let height = if dy != 0 { dy.unsigned_abs() } else { h };
        Some(ScreenDamageRect {
            x,
            y,
            width: width.min(w),
            height: height.min(h),
        })
    }
}

/// Estimates 2D scroll displacement motion vector between reference frame (`prev`)
/// and current frame (`curr`) using lightweight spatial sampling (v2.29 / ADR-115).
/// Returns `Some(MotionVector)` if the dominant displacement achieves >= `min_match_ratio`.
pub fn estimate_scroll_motion(
    prev: &ScreenFrame,
    curr: &ScreenFrame,
    max_displacement: u16,
    min_match_ratio: f32,
) -> Option<MotionVector> {
    if prev.compressed() || curr.compressed() {
        return None;
    }
    if prev.rect.w != curr.rect.w || prev.rect.h != curr.rect.h || prev.format != curr.format {
        return None;
    }
    if prev.bytes == curr.bytes {
        return None;
    }
    let w = prev.rect.w as usize;
    let h = prev.rect.h as usize;
    if w < 16 || h < 16 {
        return None;
    }
    let bpp = prev.format.bpp();
    let prev_stride = prev.effective_stride() as usize;
    let curr_stride = curr.effective_stride() as usize;

    let d_max = (max_displacement as usize)
        .clamp(
            SCREEN_MOTION_MIN_DISPLACEMENT as usize,
            SCREEN_MOTION_MAX_DISPLACEMENT as usize,
        )
        .min(h.saturating_sub(2) / 2);
    if d_max < 2 {
        return None;
    }

    // 1. Vertical motion estimation (dy in [-d_max, d_max], dy != 0)
    let mut x_samples = Vec::with_capacity(16);
    let step_x = (w / 17).max(1);
    for i in 1..=16 {
        let x = (i * step_x).min(w - 1);
        x_samples.push(x);
    }

    let y_start = d_max;
    let y_end = h.saturating_sub(d_max);
    if y_end <= y_start {
        return None;
    }
    let step_y = ((y_end - y_start) / 16).max(1);
    let mut y_samples = Vec::with_capacity(16);
    let mut y = y_start;
    while y < y_end {
        y_samples.push(y);
        y += step_y;
    }
    let total_samples = x_samples.len() * y_samples.len();
    if total_samples == 0 {
        return None;
    }

    let mut best_v_dy: i16 = 0;
    let mut best_v_matches: usize = 0;

    for dy in -(d_max as i16)..=(d_max as i16) {
        if dy == 0 {
            continue;
        }
        let mut matches = 0;
        for &cy in &y_samples {
            let py = (cy as i32 - dy as i32) as usize;
            let c_row = cy * curr_stride;
            let p_row = py * prev_stride;
            for &cx in &x_samples {
                let c_off = c_row + cx * bpp;
                let p_off = p_row + cx * bpp;
                if c_off + bpp <= curr.bytes.len()
                    && p_off + bpp <= prev.bytes.len()
                    && curr.bytes[c_off..c_off + bpp] == prev.bytes[p_off..p_off + bpp]
                {
                    matches += 1;
                }
            }
        }
        if matches > best_v_matches {
            best_v_matches = matches;
            best_v_dy = dy;
        }
    }

    let v_ratio = best_v_matches as f32 / total_samples as f32;
    if v_ratio >= min_match_ratio && best_v_dy != 0 {
        return Some(MotionVector {
            dx: 0,
            dy: best_v_dy,
            match_ratio: v_ratio,
        });
    }

    // 2. Horizontal motion estimation (dx in [-d_max_x, d_max_x], dx != 0)
    let d_max_x = d_max.min(w.saturating_sub(2) / 2);
    if d_max_x >= 2 {
        let x_start = d_max_x;
        let x_end = w.saturating_sub(d_max_x);
        if x_end > x_start {
            let step_h_x = ((x_end - x_start) / 16).max(1);
            let mut h_x_samples = Vec::with_capacity(16);
            let mut x = x_start;
            while x < x_end {
                h_x_samples.push(x);
                x += step_h_x;
            }
            let step_h_y = (h / 17).max(1);
            let mut h_y_samples = Vec::with_capacity(16);
            for i in 1..=16 {
                let y = (i * step_h_y).min(h - 1);
                h_y_samples.push(y);
            }
            let h_total = h_x_samples.len() * h_y_samples.len();
            if h_total > 0 {
                let mut best_h_dx: i16 = 0;
                let mut best_h_matches: usize = 0;
                for dx in -(d_max_x as i16)..=(d_max_x as i16) {
                    if dx == 0 {
                        continue;
                    }
                    let mut matches = 0;
                    for &cy in &h_y_samples {
                        let c_row = cy * curr_stride;
                        let p_row = cy * prev_stride;
                        for &cx in &h_x_samples {
                            let px = (cx as i32 - dx as i32) as usize;
                            let c_off = c_row + cx * bpp;
                            let p_off = p_row + px * bpp;
                            if c_off + bpp <= curr.bytes.len()
                                && p_off + bpp <= prev.bytes.len()
                                && curr.bytes[c_off..c_off + bpp] == prev.bytes[p_off..p_off + bpp]
                            {
                                matches += 1;
                            }
                        }
                    }
                    if matches > best_h_matches {
                        best_h_matches = matches;
                        best_h_dx = dx;
                    }
                }
                let h_ratio = best_h_matches as f32 / h_total as f32;
                if h_ratio >= min_match_ratio && best_h_dx != 0 {
                    return Some(MotionVector {
                        dx: best_h_dx,
                        dy: 0,
                        match_ratio: h_ratio,
                    });
                }
            }
        }
    }

    None
}
