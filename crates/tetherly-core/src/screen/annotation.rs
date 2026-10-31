// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Collaborative annotation, laser pointer and shared whiteboard state.

use super::*;
use crate::error::CoreError;
use crate::frame::InnerFrame;
use serde::Deserialize;
use serde::Serialize;

/// Category of visual annotation primitives on screen overlay (v2.19 / ADR-105).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AnnotationKind {
    /// Freehand polyline stroke connecting sequence of screen coordinates.
    Stroke { points: Vec<(u16, u16)> },
    /// Rectangular bounding box with top-left coordinate and dimensions.
    Rect { x: u16, y: u16, w: u16, h: u16 },
    /// Directional arrow pointer from start coordinate to end coordinate.
    Arrow { from: (u16, u16), to: (u16, u16) },
    /// Text note rendered with 8x8 ASCII bitmap font.
    Text {
        x: u16,
        y: u16,
        text: String,
        scale: u8,
    },
}

pub(super) fn default_annotation_color() -> [u8; 3] {
    [255, 59, 48]
}

pub(super) fn default_annotation_thickness() -> u8 {
    3
}

pub(super) fn default_annotation_alpha() -> u8 {
    230
}

/// Collaborative whiteboard annotation item on screen streaming session (v2.19 / ADR-105).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationItem {
    pub id: u32,
    #[serde(default)]
    pub author: String,
    pub kind: AnnotationKind,
    #[serde(default = "default_annotation_color")]
    pub color_rgb: [u8; 3],
    #[serde(default = "default_annotation_thickness")]
    pub thickness: u8,
    #[serde(default = "default_annotation_alpha")]
    pub alpha: u8,
    #[serde(default)]
    pub ttl_ms: u32,
    #[serde(default)]
    pub created_at_ms: u64,
}

impl AnnotationItem {
    pub fn is_expired(&self, now_ms: u64) -> bool {
        if self.ttl_ms == 0 {
            false
        } else {
            now_ms >= self.created_at_ms.saturating_add(self.ttl_ms as u64)
        }
    }
}

pub(super) fn default_laser_color() -> [u8; 3] {
    [255, 45, 85]
}

pub(super) fn default_laser_radius() -> u8 {
    12
}

/// Transient virtual laser pointer coordinate and visual configuration (v2.19 / ADR-105).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaserPointerState {
    pub x: u16,
    pub y: u16,
    pub active: bool,
    #[serde(default = "default_laser_color")]
    pub color_rgb: [u8; 3],
    #[serde(default = "default_laser_radius")]
    pub radius: u8,
}

impl Default for LaserPointerState {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            active: false,
            color_rgb: default_laser_color(),
            radius: default_laser_radius(),
        }
    }
}

/// Comprehensive report for active annotations, laser pointer and statistics (v2.19 / ADR-105, v2.20 / ADR-106).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ScreenAnnotationReport {
    pub ok: bool,
    pub annotations_count: usize,
    pub annotations: Vec<AnnotationItem>,
    pub laser_active: bool,
    pub laser: Option<LaserPointerState>,
    pub total_annotations_added: u64,
    pub laser_updates_total: u64,
    pub timestamp_ms: u64,
    #[serde(default)]
    pub whiteboard_policy: WhiteboardPolicy,
    #[serde(default)]
    pub redo_count: usize,
}

/// FNV-1a 64-bit compact hash for author identity strings (v2.20 / ADR-106).
pub fn hash_author(author: &str) -> u64 {
    if author.is_empty() {
        return 0;
    }
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in author.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

pub(super) fn default_allow_peer_clear() -> bool {
    true
}

pub(super) fn default_max_items() -> u16 {
    1000
}

/// Collaborative whiteboard policy and host governance settings (v2.20 / ADR-106).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhiteboardPolicy {
    #[serde(default)]
    pub read_only: bool,
    #[serde(default = "default_allow_peer_clear")]
    pub allow_peer_clear: bool,
    #[serde(default = "default_max_items")]
    pub max_items: u16,
}

impl Default for WhiteboardPolicy {
    fn default() -> Self {
        Self {
            read_only: false,
            allow_peer_clear: true,
            max_items: 1000,
        }
    }
}

/// Serializable vector snapshot of collaborative whiteboard annotations (v2.20 / ADR-106).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhiteboardSnapshot {
    pub version: u32,
    pub created_at_ms: u64,
    pub session_id: String,
    pub active_items: Vec<AnnotationItem>,
    pub total_added_counter: u64,
}

/// Chronological undo/redo history and capacity engine for whiteboard (v2.20 / ADR-106).
#[derive(Debug, Clone, Default)]
pub struct WhiteboardHistory {
    pub(super) active: Vec<AnnotationItem>,
    pub(super) undo_stack: Vec<AnnotationItem>,
    pub(super) redo_stack: Vec<AnnotationItem>,
    pub(super) policy: WhiteboardPolicy,
    pub(super) total_added: u64,
}

impl WhiteboardHistory {
    pub fn new(policy: WhiteboardPolicy) -> Self {
        Self {
            active: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            policy,
            total_added: 0,
        }
    }

    pub fn policy(&self) -> WhiteboardPolicy {
        self.policy
    }

    pub fn set_policy(&mut self, policy: WhiteboardPolicy) {
        self.policy = policy;
    }

    pub fn active_items(&self) -> &[AnnotationItem] {
        &self.active
    }

    pub fn active_items_mut(&mut self) -> &mut Vec<AnnotationItem> {
        &mut self.active
    }

    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    pub fn redo_count(&self) -> usize {
        self.redo_stack.len()
    }

    pub fn total_added(&self) -> u64 {
        self.total_added
    }

    pub fn prune_expired(&mut self, now_ms: u64) {
        self.active.retain(|it| !it.is_expired(now_ms));
        self.redo_stack.retain(|it| !it.is_expired(now_ms));
        self.undo_stack.retain(|it| !it.is_expired(now_ms));
    }

    pub fn add_item(&mut self, mut item: AnnotationItem) -> Result<u32, CoreError> {
        if self.policy.read_only {
            return Err(CoreError::InvalidConfig("whiteboard is read-only".into()));
        }
        if self.active.len() >= self.policy.max_items as usize {
            return Err(CoreError::InvalidConfig(format!(
                "whiteboard capacity limit reached ({} items)",
                self.policy.max_items
            )));
        }
        self.total_added += 1;
        if item.id == 0 {
            item.id = self.total_added as u32;
        }
        let item_id = item.id;
        self.undo_stack.push(item.clone());
        self.active.push(item);
        self.redo_stack.clear();
        Ok(item_id)
    }

    pub fn undo(&mut self, author: Option<&str>) -> Option<AnnotationItem> {
        let target_author = author.filter(|s| !s.is_empty());
        if let Some(target) = target_author {
            if let Some(pos) = self.active.iter().rposition(|it| it.author == target) {
                let item = self.active.remove(pos);
                self.redo_stack.push(item.clone());
                Some(item)
            } else {
                None
            }
        } else if let Some(item) = self.active.pop() {
            self.redo_stack.push(item.clone());
            Some(item)
        } else {
            None
        }
    }

    pub fn redo(&mut self, author: Option<&str>) -> Option<AnnotationItem> {
        let target_author = author.filter(|s| !s.is_empty());
        if let Some(target) = target_author {
            if let Some(pos) = self.redo_stack.iter().rposition(|it| it.author == target) {
                let item = self.redo_stack.remove(pos);
                self.active.push(item.clone());
                Some(item)
            } else {
                None
            }
        } else if let Some(item) = self.redo_stack.pop() {
            self.active.push(item.clone());
            Some(item)
        } else {
            None
        }
    }

    pub fn clear(&mut self) {
        for item in self.active.drain(..) {
            self.undo_stack.push(item);
        }
        self.redo_stack.clear();
    }

    pub fn export_snapshot(&self, session_id: String, now_ms: u64) -> WhiteboardSnapshot {
        let items: Vec<AnnotationItem> = self
            .active
            .iter()
            .filter(|it| !it.is_expired(now_ms))
            .cloned()
            .collect();
        WhiteboardSnapshot {
            version: 1,
            created_at_ms: now_ms,
            session_id,
            active_items: items,
            total_added_counter: self.total_added,
        }
    }

    pub fn import_snapshot(
        &mut self,
        snapshot: WhiteboardSnapshot,
        replace: bool,
    ) -> Result<usize, CoreError> {
        if self.policy.read_only {
            return Err(CoreError::InvalidConfig("whiteboard is read-only".into()));
        }
        if replace {
            self.clear();
        }
        let mut count = 0;
        for mut it in snapshot.active_items {
            if self.active.len() >= self.policy.max_items as usize {
                break;
            }
            self.total_added += 1;
            it.id = self.total_added as u32;
            self.active.push(it);
            count += 1;
        }
        self.redo_stack.clear();
        Ok(count)
    }
}

/// Inner frame payload for structured annotation synchronization (v2.19 / ADR-105, v2.20 / ADR-106).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "data", rename_all = "snake_case")]
pub enum ScreenAnnotationMsg {
    Add(AnnotationItem),
    Remove { id: u32 },
    Undo { author: String },
    Redo { author: String },
    Clear,
    Snapshot(WhiteboardSnapshot),
    Policy(WhiteboardPolicy),
}

impl ScreenAnnotationMsg {
    pub const HEADER_LEN: usize = 5;

    pub fn encode_payload(&self) -> Result<Vec<u8>, CoreError> {
        let json_bytes =
            serde_json::to_vec(self).map_err(|e| CoreError::ScreenRefused(e.to_string()))?;
        let mut out = Vec::with_capacity(Self::HEADER_LEN + json_bytes.len());
        out.extend_from_slice(&ANNOTATION_MAGIC);
        out.push(ANNOTATION_PROTO);
        out.extend_from_slice(&json_bytes);
        Ok(out)
    }

    pub fn decode_payload(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() < Self::HEADER_LEN {
            return Err(CoreError::TruncatedFrame);
        }
        if payload[0..4] != ANNOTATION_MAGIC {
            return Err(CoreError::ScreenFrame);
        }
        if payload[4] != ANNOTATION_PROTO {
            return Err(CoreError::UnsupportedProto);
        }
        serde_json::from_slice(&payload[Self::HEADER_LEN..])
            .map_err(|e| CoreError::ScreenRefused(e.to_string()))
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = self.encode_payload()?;
        InnerFrame::new(TYPE_SCREEN_ANNOTATION, payload)
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_SCREEN_ANNOTATION {
            return Err(CoreError::ScreenFrame);
        }
        Self::decode_payload(&frame.payload)
    }
}
