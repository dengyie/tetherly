// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Spatial tile grid, dirty-tile extraction, tile dictionary cache and focus scheduling.

use super::*;
use crate::error::CoreError;
use std::collections::HashMap;
use std::collections::VecDeque;

/// Spatial tile-grid dirty tile payload record (v2.30 / ADR-116).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DirtyTile {
    pub index: u16,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub data: Vec<u8>,
}

/// Screen Spatial Tile-Grid Decomposition Configuration (v2.30 / ADR-116).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileConfig {
    pub enabled: bool,
    pub tile_size: u16,
}

impl Default for ScreenTileConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tile_size: SCREEN_TILE_DEFAULT_SIZE,
        }
    }
}

impl ScreenTileConfig {
    pub fn new(enabled: bool, tile_size: u16) -> Self {
        Self {
            enabled,
            tile_size: if tile_size == 0 {
                SCREEN_TILE_DEFAULT_SIZE
            } else {
                tile_size.clamp(SCREEN_TILE_MIN_SIZE, SCREEN_TILE_MAX_SIZE)
            },
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Screen Spatial Tile-Grid Telemetry Report (v2.30 / ADR-116).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileReport {
    pub ok: bool,
    pub enabled: bool,
    pub tile_size: u16,
    pub grid_frames_sent: u64,
    pub dirty_tiles_sent: u64,
    pub skipped_tiles_count: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenTileReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            tile_size: SCREEN_TILE_DEFAULT_SIZE,
            grid_frames_sent: 0,
            dirty_tiles_sent: 0,
            skipped_tiles_count: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// Computes (columns, rows, total_tiles) for screen dimensions given tile_size (v2.30 / ADR-116).
pub fn compute_tile_grid_dims(screen_w: u16, screen_h: u16, tile_size: u16) -> (u16, u16, u16) {
    if screen_w == 0 || screen_h == 0 {
        return (0, 0, 0);
    }
    let t = if tile_size == 0 {
        SCREEN_TILE_DEFAULT_SIZE
    } else {
        tile_size.clamp(SCREEN_TILE_MIN_SIZE, SCREEN_TILE_MAX_SIZE)
    };
    let cols = screen_w.div_ceil(t);
    let rows = screen_h.div_ceil(t);
    let total = cols.saturating_mul(rows);
    (cols, rows, total)
}

/// Extracts dirty tiles between previous uncompressed frame (`prev`) and current uncompressed frame (`curr`) (v2.30 / ADR-116).
/// Returns (total_tiles, Vec<DirtyTile>).
pub fn extract_dirty_tiles(
    prev: &ScreenFrame,
    curr: &ScreenFrame,
    tile_size: u16,
) -> (u16, Vec<DirtyTile>) {
    if prev.compressed() || curr.compressed() {
        return (0, Vec::new());
    }
    if prev.rect.w != curr.rect.w || prev.rect.h != curr.rect.h || prev.format != curr.format {
        return (0, Vec::new());
    }
    let w = curr.rect.w;
    let h = curr.rect.h;
    let (cols, rows, total_tiles) = compute_tile_grid_dims(w, h, tile_size);
    if total_tiles == 0 {
        return (0, Vec::new());
    }
    let t = if tile_size == 0 {
        SCREEN_TILE_DEFAULT_SIZE
    } else {
        tile_size.clamp(SCREEN_TILE_MIN_SIZE, SCREEN_TILE_MAX_SIZE)
    };

    let bpp = curr.format.bpp();
    let prev_stride = (prev.rect.w as usize) * bpp;
    let curr_stride = (curr.rect.w as usize) * bpp;

    let mut dirty_tiles = Vec::new();
    let mut tile_idx = 0u16;

    for r in 0..rows {
        let ty = r * t;
        let th = (h - ty).min(t);
        for c in 0..cols {
            let tx = c * t;
            let tw = (w - tx).min(t);
            let idx = tile_idx;
            tile_idx += 1;

            let mut is_dirty = false;
            let tile_row_bytes = (tw as usize) * bpp;

            for dy in 0..th as usize {
                let p_off = (ty as usize + dy) * prev_stride + (tx as usize) * bpp;
                let c_off = (ty as usize + dy) * curr_stride + (tx as usize) * bpp;
                if p_off + tile_row_bytes <= prev.bytes.len()
                    && c_off + tile_row_bytes <= curr.bytes.len()
                {
                    if prev.bytes[p_off..p_off + tile_row_bytes]
                        != curr.bytes[c_off..c_off + tile_row_bytes]
                    {
                        is_dirty = true;
                        break;
                    }
                } else {
                    is_dirty = true;
                    break;
                }
            }

            if is_dirty {
                let mut data = Vec::with_capacity((tw as usize) * (th as usize) * bpp);
                for dy in 0..th as usize {
                    let c_off = (ty as usize + dy) * curr_stride + (tx as usize) * bpp;
                    data.extend_from_slice(&curr.bytes[c_off..c_off + tile_row_bytes]);
                }
                dirty_tiles.push(DirtyTile {
                    index: idx,
                    x: tx,
                    y: ty,
                    w: tw,
                    h: th,
                    data,
                });
            }
        }
    }

    (total_tiles, dirty_tiles)
}

/// Spatial tile cache dirty tile payload record (v2.31 / ADR-117).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedTileEntry {
    pub index: u16,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub cache_hit: bool,
    pub cache_id: u16,
    pub data: Vec<u8>,
}

/// Screen Spatial Tile Dictionary Cache Configuration (v2.31 / ADR-117).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileCacheConfig {
    pub enabled: bool,
    pub max_entries: u16,
}

impl Default for ScreenTileCacheConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_entries: SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES,
        }
    }
}

impl ScreenTileCacheConfig {
    pub fn new(enabled: bool, max_entries: u16) -> Self {
        Self {
            enabled,
            max_entries: if max_entries == 0 {
                SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
            } else {
                max_entries.clamp(SCREEN_TILE_CACHE_MIN_ENTRIES, SCREEN_TILE_CACHE_MAX_ENTRIES)
            },
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Screen Spatial Tile Dictionary Cache Telemetry Report (v2.31 / ADR-117).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenTileCacheReport {
    pub ok: bool,
    pub enabled: bool,
    pub max_entries: u16,
    pub current_entries: u16,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub hit_ratio_pct: f32,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenTileCacheReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            max_entries: SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES,
            current_entries: 0,
            cache_hits: 0,
            cache_misses: 0,
            hit_ratio_pct: 0.0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// Fast, zero-dependency 64-bit FNV-1a content hash for dirty tile deduplication (v2.31 / ADR-117).
pub fn fnv1a_hash64(data: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in data {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Dual-synchronized LRU tile dictionary cache for cross-frame and spatial deduplication (v2.31 / ADR-117).
#[derive(Debug, Clone)]
pub struct ScreenTileDictionary {
    pub(super) max_entries: u16,
    pub(super) hash_to_id: HashMap<u64, u16>,
    pub(super) id_to_hash: Vec<Option<u64>>,
    pub(super) lru_order: VecDeque<u16>,
}

impl ScreenTileDictionary {
    pub fn new(max_entries: u16) -> Self {
        let max = if max_entries == 0 {
            SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
        } else {
            max_entries.clamp(SCREEN_TILE_CACHE_MIN_ENTRIES, SCREEN_TILE_CACHE_MAX_ENTRIES)
        };
        Self {
            max_entries: max,
            hash_to_id: HashMap::with_capacity(max as usize),
            id_to_hash: vec![None; max as usize],
            lru_order: VecDeque::with_capacity(max as usize),
        }
    }

    pub fn max_entries(&self) -> u16 {
        self.max_entries
    }

    pub fn current_entries(&self) -> u16 {
        self.hash_to_id.len() as u16
    }

    pub fn clear(&mut self) {
        self.hash_to_id.clear();
        self.id_to_hash.fill(None);
        self.lru_order.clear();
    }

    pub fn resize(&mut self, new_max_entries: u16) {
        let max = if new_max_entries == 0 {
            SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES
        } else {
            new_max_entries.clamp(SCREEN_TILE_CACHE_MIN_ENTRIES, SCREEN_TILE_CACHE_MAX_ENTRIES)
        };
        if max != self.max_entries {
            self.max_entries = max;
            self.clear();
            self.id_to_hash.resize(max as usize, None);
        }
    }

    /// Checks if a tile is in the dictionary. If present, returns `(true, cache_id)`.
    /// If absent, assigns an entry (evicting LRU if full) and returns `(false, cache_id)`.
    pub fn lookup_or_insert(&mut self, hash: u64) -> (bool, u16) {
        if let Some(&id) = self.hash_to_id.get(&hash) {
            // Cache Hit: update LRU order
            if let Some(pos) = self.lru_order.iter().position(|&x| x == id) {
                self.lru_order.remove(pos);
            }
            self.lru_order.push_back(id);
            (true, id)
        } else {
            // Cache Miss: assign a slot
            let id = if (self.hash_to_id.len() as u16) < self.max_entries {
                let new_id = self.hash_to_id.len() as u16;
                self.id_to_hash[new_id as usize] = Some(hash);
                new_id
            } else {
                // Evict LRU
                let victim_id = self.lru_order.pop_front().unwrap_or(0);
                if let Some(old_hash) = self.id_to_hash[victim_id as usize].take() {
                    self.hash_to_id.remove(&old_hash);
                }
                self.id_to_hash[victim_id as usize] = Some(hash);
                victim_id
            };
            self.hash_to_id.insert(hash, id);
            self.lru_order.push_back(id);
            (false, id)
        }
    }
}

/// Screen streaming foveated focus mode (v2.32 / ADR-118).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ScreenFocusMode {
    #[default]
    Cursor,
    Box,
}

impl ScreenFocusMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ScreenFocusMode::Cursor => "cursor",
            ScreenFocusMode::Box => "box",
        }
    }

    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => ScreenFocusMode::Box,
            _ => ScreenFocusMode::Cursor,
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            ScreenFocusMode::Cursor => 0,
            ScreenFocusMode::Box => 1,
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "box" | "1" | "rect" | "window" => Some(ScreenFocusMode::Box),
            "cursor" | "0" | "mouse" | "pointer" => Some(ScreenFocusMode::Cursor),
            _ => None,
        }
    }
}

impl std::str::FromStr for ScreenFocusMode {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_loose(s).ok_or(CoreError::ScreenFrame)
    }
}

/// Screen streaming foveated focus configuration (v2.32 / ADR-118).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScreenFocusConfig {
    pub enabled: bool,
    pub mode: ScreenFocusMode,
    pub radius: u16,
    pub peripheral_skip_ratio: u8,
    pub box_x: u16,
    pub box_y: u16,
    pub box_w: u16,
    pub box_h: u16,
}

impl Default for ScreenFocusConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: ScreenFocusMode::Cursor,
            radius: SCREEN_FOCUS_DEFAULT_RADIUS,
            peripheral_skip_ratio: SCREEN_FOCUS_DEFAULT_SKIP_RATIO,
            box_x: 0,
            box_y: 0,
            box_w: 512,
            box_h: 512,
        }
    }
}

impl ScreenFocusConfig {
    pub fn new(
        enabled: bool,
        mode: ScreenFocusMode,
        radius: u16,
        peripheral_skip_ratio: u8,
        box_coords: Option<(u16, u16, u16, u16)>,
    ) -> Self {
        let (box_x, box_y, box_w, box_h) = box_coords.unwrap_or((0, 0, 512, 512));
        Self {
            enabled,
            mode,
            radius: if radius == 0 {
                SCREEN_FOCUS_DEFAULT_RADIUS
            } else {
                radius.clamp(SCREEN_FOCUS_MIN_RADIUS, SCREEN_FOCUS_MAX_RADIUS)
            },
            peripheral_skip_ratio: peripheral_skip_ratio.min(SCREEN_FOCUS_MAX_SKIP_RATIO),
            box_x,
            box_y,
            box_w: box_w.max(32),
            box_h: box_h.max(32),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// Screen streaming foveated focus telemetry report (v2.32 / ADR-118).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ScreenFocusReport {
    pub ok: bool,
    pub enabled: bool,
    pub mode: ScreenFocusMode,
    pub radius: u16,
    pub peripheral_skip_ratio: u8,
    pub box_x: u16,
    pub box_y: u16,
    pub box_w: u16,
    pub box_h: u16,
    pub focus_tiles_sent: u64,
    pub peripheral_tiles_sent: u64,
    pub peripheral_tiles_skipped: u64,
    pub saved_bandwidth_bytes: u64,
}

impl Default for ScreenFocusReport {
    fn default() -> Self {
        Self {
            ok: true,
            enabled: false,
            mode: ScreenFocusMode::Cursor,
            radius: SCREEN_FOCUS_DEFAULT_RADIUS,
            peripheral_skip_ratio: SCREEN_FOCUS_DEFAULT_SKIP_RATIO,
            box_x: 0,
            box_y: 0,
            box_w: 512,
            box_h: 512,
            focus_tiles_sent: 0,
            peripheral_tiles_sent: 0,
            peripheral_tiles_skipped: 0,
            saved_bandwidth_bytes: 0,
        }
    }
}

/// Checks whether a spatial tile is inside the active focus zone (v2.32 / ADR-118).
pub fn is_tile_in_focus(
    tile_x: u16,
    tile_y: u16,
    tile_w: u16,
    tile_h: u16,
    config: &ScreenFocusConfig,
    cursor_pos: Option<(i32, i32)>,
) -> bool {
    let center_x = tile_x as i32 + (tile_w as i32 / 2);
    let center_y = tile_y as i32 + (tile_h as i32 / 2);
    match config.mode {
        ScreenFocusMode::Cursor => {
            let (cx, cy) = cursor_pos.unwrap_or((0, 0));
            let dx = center_x - cx;
            let dy = center_y - cy;
            let dist_sq = (dx as i64 * dx as i64 + dy as i64 * dy as i64) as u64;
            let r = config.radius as u64;
            dist_sq <= r * r
        }
        ScreenFocusMode::Box => {
            let bx1 = config.box_x as i32;
            let by1 = config.box_y as i32;
            let bx2 = bx1 + config.box_w as i32;
            let by2 = by1 + config.box_h as i32;
            center_x >= bx1 && center_x < bx2 && center_y >= by1 && center_y < by2
        }
    }
}
