// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Region-of-interest crop and sub-viewport roaming.

use serde::Deserialize;
use serde::Serialize;

/// Default zoom percentage for Region-of-Interest viewports (100 = 100%).
pub fn default_roi_zoom() -> u16 {
    100
}

/// Region-of-Interest (ROI) viewport parameters for dynamic sub-rectangle cropping and zoom (v2.21 / ADR-107).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenRoi {
    pub active: bool,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    #[serde(default = "default_roi_zoom")]
    pub zoom_percent: u16,
}

impl Default for ScreenRoi {
    fn default() -> Self {
        Self {
            active: false,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
            zoom_percent: default_roi_zoom(),
        }
    }
}

impl ScreenRoi {
    /// Sanitize and clamp ROI viewport to screen bounds, enforcing even dimensions for codec safety.
    pub fn sanitize(&mut self, screen_w: u16, screen_h: u16) {
        if screen_w < 2 || screen_h < 2 {
            self.active = false;
            self.x = 0;
            self.y = 0;
            self.w = screen_w;
            self.h = screen_h;
            return;
        }
        self.zoom_percent = self.zoom_percent.clamp(50, 800);
        self.x = self.x.min(screen_w.saturating_sub(2));
        self.y = self.y.min(screen_h.saturating_sub(2));
        let max_w = screen_w.saturating_sub(self.x);
        let max_h = screen_h.saturating_sub(self.y);
        let clamped_w = self.w.clamp(2, max_w);
        let clamped_h = self.h.clamp(2, max_h);
        self.w = (clamped_w / 2) * 2;
        self.h = (clamped_h / 2) * 2;
        if self.w == 0 || self.h == 0 {
            self.active = false;
        }
    }

    /// Maps a screen-space coordinate (e.g. laser pointer or whiteboard annotation) into ROI-relative space.
    /// Returns None if the point lies outside the cropped viewport rectangle.
    pub fn map_screen_to_roi(&self, sx: u16, sy: u16) -> Option<(u16, u16)> {
        if !self.active {
            return Some((sx, sy));
        }
        if sx < self.x || sy < self.y {
            return None;
        }
        let rx = sx - self.x;
        let ry = sy - self.y;
        if rx < self.w && ry < self.h {
            Some((rx, ry))
        } else {
            None
        }
    }

    /// Maps an ROI-relative coordinate back to full-screen space.
    pub fn map_roi_to_screen(&self, rx: u16, ry: u16) -> (u16, u16) {
        if !self.active {
            (rx, ry)
        } else {
            (self.x.saturating_add(rx), self.y.saturating_add(ry))
        }
    }
}

/// Status report for the active Screen ROI viewport pipeline (v2.21 / ADR-107).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScreenRoiReport {
    pub active: bool,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub zoom_percent: u16,
    pub source_w: u16,
    pub source_h: u16,
    pub effective_w: u16,
    pub effective_h: u16,
    pub update_count: u64,
}
