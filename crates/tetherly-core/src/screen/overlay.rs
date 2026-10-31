// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Display/privacy telemetry and the 2D canvas overlay primitives.

#[inline(always)]
pub(super) fn blend_pixel(
    pixels: &mut [u8],
    offset: usize,
    is_bgra: bool,
    r_fg: u8,
    g_fg: u8,
    b_fg: u8,
    alpha: u8,
) {
    if alpha == 0 || offset + 3 >= pixels.len() {
        return;
    }
    let alpha_u32 = alpha as u32;
    let inv_alpha = 255u32 - alpha_u32;
    if is_bgra {
        let bg_b = pixels[offset] as u32;
        let bg_g = pixels[offset + 1] as u32;
        let bg_r = pixels[offset + 2] as u32;
        pixels[offset] = ((b_fg as u32 * alpha_u32 + bg_b * inv_alpha + 128) / 255) as u8;
        pixels[offset + 1] = ((g_fg as u32 * alpha_u32 + bg_g * inv_alpha + 128) / 255) as u8;
        pixels[offset + 2] = ((r_fg as u32 * alpha_u32 + bg_r * inv_alpha + 128) / 255) as u8;
    } else {
        let bg_r = pixels[offset] as u32;
        let bg_g = pixels[offset + 1] as u32;
        let bg_b = pixels[offset + 2] as u32;
        pixels[offset] = ((r_fg as u32 * alpha_u32 + bg_r * inv_alpha + 128) / 255) as u8;
        pixels[offset + 1] = ((g_fg as u32 * alpha_u32 + bg_g * inv_alpha + 128) / 255) as u8;
        pixels[offset + 2] = ((b_fg as u32 * alpha_u32 + bg_b * inv_alpha + 128) / 255) as u8;
    }
}

pub(super) fn isqrt(val: i64) -> i64 {
    if val <= 0 {
        return 0;
    }
    let mut x0 = val / 2;
    if x0 == 0 {
        return 1;
    }
    let mut x1 = (x0 + val / x0) / 2;
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + val / x0) / 2;
    }
    x0
}

/// Rendering style bundle shared by collaborative annotation primitives (ADR-105).
pub(super) struct Stroke {
    pub(super) thickness: u8,
    pub(super) color: [u8; 3],
    pub(super) alpha: u8,
}

/// Pixel raster context bundling the frame buffer view with geometry and channel
/// order so overlay primitives stay small, allocation-free methods (ADR-105).
pub(super) struct Canvas<'a> {
    pub(super) pixels: &'a mut [u8],
    pub(super) w: usize,
    pub(super) h: usize,
    pub(super) is_bgra: bool,
}

impl Canvas<'_> {
    #[inline]
    pub(super) fn blend(&mut self, x: i32, y: i32, color: [u8; 3], alpha: u8) {
        if x < 0 || y < 0 || x >= self.w as i32 || y >= self.h as i32 {
            return;
        }
        let offset = (y as usize * self.w + x as usize) * 4;
        blend_pixel(
            self.pixels,
            offset,
            self.is_bgra,
            color[0],
            color[1],
            color[2],
            alpha,
        );
    }

    pub(super) fn disk(&mut self, cx: i32, cy: i32, radius: i32, color: [u8; 3], alpha: u8) {
        if radius <= 0 {
            self.blend(cx, cy, color, alpha);
            return;
        }
        let r2 = radius * radius;
        let min_y = (cy - radius).max(0);
        let max_y = (cy + radius).min(self.h as i32 - 1);
        let min_x = (cx - radius).max(0);
        let max_x = (cx + radius).min(self.w as i32 - 1);
        for y in min_y..=max_y {
            let dy = y - cy;
            let dy2 = dy * dy;
            for x in min_x..=max_x {
                let dx = x - cx;
                if dx * dx + dy2 <= r2 {
                    self.blend(x, y, color, alpha);
                }
            }
        }
    }

    /// Bresenham scan with per-step disk expansion for stroke thickness (ADR-105).
    pub(super) fn line(&mut self, mut x0: i32, mut y0: i32, x1: i32, y1: i32, stroke: &Stroke) {
        let radius = (stroke.thickness.max(1) as i32) / 2;
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;

        loop {
            self.disk(x0, y0, radius, stroke.color, stroke.alpha);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }

    pub(super) fn rect(&mut self, x: i32, y: i32, rw: i32, rh: i32, stroke: &Stroke) {
        if rw <= 0 || rh <= 0 {
            return;
        }
        let x1 = x + rw;
        let y1 = y + rh;
        self.line(x, y, x1, y, stroke);
        self.line(x1, y, x1, y1, stroke);
        self.line(x1, y1, x, y1, stroke);
        self.line(x, y1, x, y, stroke);
    }

    /// Shaft plus two 30-degree wings computed with integer-only geometry (ADR-105).
    pub(super) fn arrow(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, stroke: &Stroke) {
        self.line(x0, y0, x1, y1, stroke);

        let vx = (x1 - x0) as i64;
        let vy = (y1 - y0) as i64;
        let len = isqrt(vx * vx + vy * vy);
        if len <= 2 {
            return;
        }
        let wing_len = ((stroke.thickness as i64) * 4)
            .clamp(10, 40)
            .min(len / 2 + 5);
        let bx = -vx * 64 / len;
        let by = -vy * 64 / len;

        let w1x = (x1 as i64) + (bx * 55 - by * 32) * wing_len / 4096;
        let w1y = (y1 as i64) + (by * 55 + bx * 32) * wing_len / 4096;

        let w2x = (x1 as i64) + (bx * 55 + by * 32) * wing_len / 4096;
        let w2y = (y1 as i64) + (by * 55 - bx * 32) * wing_len / 4096;

        self.line(x1, y1, w1x as i32, w1y as i32, stroke);
        self.line(x1, y1, w2x as i32, w2y as i32, stroke);
    }

    /// Midpoint-circle spotlight: hot inner core plus a quadratically decaying
    /// integer alpha halo (ADR-105).
    pub(super) fn laser_spot(&mut self, cx: i32, cy: i32, radius: u8, color: [u8; 3]) {
        let r = radius.max(4) as i32;
        let r2 = r * r;
        let core_r = (r / 3).max(2);
        let core_r2 = core_r * core_r;
        let min_y = (cy - r).max(0);
        let max_y = (cy + r).min(self.h as i32 - 1);
        let min_x = (cx - r).max(0);
        let max_x = (cx + r).min(self.w as i32 - 1);

        let core_color = [
            color[0].saturating_add(180),
            color[1].saturating_add(180),
            color[2].saturating_add(180),
        ];

        for y in min_y..=max_y {
            let dy = y - cy;
            let dy2 = dy * dy;
            for x in min_x..=max_x {
                let dx = x - cx;
                let dist2 = dx * dx + dy2;
                if dist2 <= core_r2 {
                    self.blend(x, y, core_color, 240);
                } else if dist2 <= r2 {
                    let span = (r2 - core_r2).max(1) as u64;
                    let rem = (r2 - dist2) as u64;
                    let alpha = (20 + (rem * 160 / span)).min(200) as u8;
                    self.blend(x, y, color, alpha);
                }
            }
        }
    }

    /// 8x8 bitmap-font annotation text with integer alpha blending (ADR-105).
    pub(super) fn annotation_text(
        &mut self,
        start_x: usize,
        start_y: usize,
        text: &str,
        scale: u8,
        color: [u8; 3],
        alpha: u8,
    ) {
        if alpha == 0 || text.is_empty() {
            return;
        }
        let scale = if scale == 0 { 1 } else { scale.min(4) } as usize;
        let char_w = 8 * scale;
        for (idx, ch) in text.chars().enumerate() {
            let char_x = start_x + idx * char_w;
            let glyph =
                crate::screen_font::get_glyph(ch).unwrap_or(&crate::screen_font::FONT_8X8[0]);
            for (row, &bits) in glyph.iter().enumerate() {
                for col in 0..8 {
                    if (bits & (1 << (7 - col))) != 0 {
                        for dy in 0..scale {
                            let py = start_y + row * scale + dy;
                            if py >= self.h {
                                continue;
                            }
                            for dx in 0..scale {
                                let px = char_x + col * scale + dx;
                                if px >= self.w {
                                    continue;
                                }
                                let offset = (py * self.w + px) * 4;
                                blend_pixel(
                                    self.pixels,
                                    offset,
                                    self.is_bgra,
                                    color[0],
                                    color[1],
                                    color[2],
                                    alpha,
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Detailed display properties and geometry for multi-display and virtual display multiplexing (v2.11 / ADR-097).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenDisplayInfo {
    /// Zero-based identifier of the display on the host system.
    pub display_id: u8,
    /// Human-readable title or monitor model name.
    pub title: String,
    /// Physical width in pixels.
    pub width: u32,
    /// Physical height in pixels.
    pub height: u32,
    /// Operating system display scaling percentage (e.g. 100%, 125%, 150%, 200%).
    pub scale_factor_pct: u32,
    /// Indicates whether this display is designated as the primary monitor.
    pub is_primary: bool,
    /// Indicates whether this display is an emulated virtual secondary display (ADR-073).
    pub is_virtual: bool,
}

impl ScreenDisplayInfo {
    pub fn new(
        display_id: u8,
        title: impl Into<String>,
        width: u32,
        height: u32,
        scale_factor_pct: u32,
        is_primary: bool,
        is_virtual: bool,
    ) -> Self {
        Self {
            display_id,
            title: title.into(),
            width,
            height,
            scale_factor_pct,
            is_primary,
            is_virtual,
        }
    }

    pub fn primary_default() -> Self {
        Self {
            display_id: 0,
            title: "Display 0 (Primary)".to_string(),
            width: 1920,
            height: 1080,
            scale_factor_pct: 100,
            is_primary: true,
            is_virtual: false,
        }
    }
}

/// Status report of active multi-display configuration and dynamic multiplexing statistics (v2.11 / ADR-097).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenDisplayReport {
    pub ok: bool,
    /// Currently active display ID being captured and streamed.
    pub active_display_id: u8,
    /// List of all discovered displays on the streaming host.
    pub displays: Vec<ScreenDisplayInfo>,
    /// Cumulative count of dynamic display switches performed.
    pub switches_count: u64,
    /// Timestamp in microseconds of the last display switch event.
    pub last_switch_us: u64,
}

/// Status report of screen privacy mode, display blanking and input lockout (v2.12 / ADR-098).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenPrivacyReport {
    pub ok: bool,
    /// Whether privacy screen blanking is currently active on the host.
    pub enabled: bool,
    /// Whether local physical input (keyboard/mouse) lockout is active on the host.
    pub lock_input: bool,
    /// Blanking implementation method ("virtual_overlay", "display_power", "driver_curtain", or "none").
    pub blanking_method: String,
    /// Timestamp in microseconds when privacy mode was last toggled.
    pub last_change_us: u64,
}

impl Default for ScreenPrivacyReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            lock_input: false,
            blanking_method: "virtual_overlay".to_string(),
            last_change_us: 0,
        }
    }
}
