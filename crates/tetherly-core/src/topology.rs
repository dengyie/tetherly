use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::frame::{InnerFrame, TYPE_TOPOLOGY_STATE, TYPE_TOPOLOGY_SYNC_REQ};

/// Represents a screen border edge where cursor transitions occur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl DisplayEdge {
    /// Returns the geometrically opposing edge.
    pub fn opposite(&self) -> Self {
        match self {
            DisplayEdge::Left => DisplayEdge::Right,
            DisplayEdge::Right => DisplayEdge::Left,
            DisplayEdge::Top => DisplayEdge::Bottom,
            DisplayEdge::Bottom => DisplayEdge::Top,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            DisplayEdge::Left => "left",
            DisplayEdge::Right => "right",
            DisplayEdge::Top => "top",
            DisplayEdge::Bottom => "bottom",
        }
    }
}

impl fmt::Display for DisplayEdge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for DisplayEdge {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" => Ok(DisplayEdge::Left),
            "right" => Ok(DisplayEdge::Right),
            "top" | "up" => Ok(DisplayEdge::Top),
            "bottom" | "down" => Ok(DisplayEdge::Bottom),
            _ => Err(CoreError::InvalidPin),
        }
    }
}

/// Logical display resolution and scale factor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DisplayRect {
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_scale_factor")]
    pub scale_factor: f32,
}

fn default_scale_factor() -> f32 {
    1.0
}

impl DisplayRect {
    pub fn new(width: u32, height: u32, scale_factor: f32) -> Self {
        Self {
            width: TopologyValidator::clamp_dimension(width, true),
            height: TopologyValidator::clamp_dimension(height, false),
            scale_factor: TopologyValidator::clamp_scale(scale_factor),
        }
    }

    pub fn sanitize(&mut self) {
        self.width = TopologyValidator::clamp_dimension(self.width, true);
        self.height = TopologyValidator::clamp_dimension(self.height, false);
        self.scale_factor = TopologyValidator::clamp_scale(self.scale_factor);
    }
}

impl Default for DisplayRect {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            scale_factor: 1.0,
        }
    }
}

/// Description of an adjacent peer linked to a specific edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeNeighbor {
    pub peer_id: String,
    pub peer_edge: DisplayEdge,
    #[serde(default)]
    pub offset: i32,
}

/// Tuning parameters governing cursor edge switching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeCrossingConfig {
    #[serde(default = "default_resistance")]
    pub resistance_px: u32,
    #[serde(default = "default_delay_ms")]
    pub delay_ms: u32,
    #[serde(default = "default_corner_lock")]
    pub corner_lock_size_px: u32,
    #[serde(default)]
    pub wrap_around: bool,
}

fn default_resistance() -> u32 {
    15
}

fn default_delay_ms() -> u32 {
    100
}

fn default_corner_lock() -> u32 {
    30
}

impl Default for EdgeCrossingConfig {
    fn default() -> Self {
        Self {
            resistance_px: 15,
            delay_ms: 100,
            corner_lock_size_px: 30,
            wrap_around: false,
        }
    }
}

impl EdgeCrossingConfig {
    pub fn sanitize(&mut self) {
        self.resistance_px = TopologyValidator::clamp_resistance(self.resistance_px);
        self.delay_ms = TopologyValidator::clamp_delay(self.delay_ms);
        self.corner_lock_size_px = TopologyValidator::clamp_corner_lock(self.corner_lock_size_px);
    }
}

/// Wire frame broadcasting or reporting spatial display topology and neighbors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopologyStateFrame {
    pub device_id: String,
    pub display: DisplayRect,
    #[serde(default)]
    pub neighbors: HashMap<DisplayEdge, EdgeNeighbor>,
    #[serde(default)]
    pub config: EdgeCrossingConfig,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl TopologyStateFrame {
    pub fn sanitize(&mut self) {
        self.display.sanitize();
        self.config.sanitize();
        if self.device_id.len() > 64 {
            self.device_id.truncate(64);
        }
        for neighbor in self.neighbors.values_mut() {
            if neighbor.peer_id.len() > 64 {
                neighbor.peer_id.truncate(64);
            }
            neighbor.offset = neighbor.offset.clamp(-8192, 8192);
        }
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_TOPOLOGY_STATE, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }
}

/// Wire frame requesting a remote peer to reply with its current TopologyStateFrame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopologySyncReqFrame {
    pub request_id: String,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl TopologySyncReqFrame {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = serde_json::to_vec(self).map_err(|_| CoreError::PayloadTooLarge)?;
        InnerFrame::new(TYPE_TOPOLOGY_SYNC_REQ, payload)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        serde_json::from_slice(payload).map_err(|_| CoreError::TruncatedFrame)
    }
}

/// Validation rules and bounds enforcement for spatial multi-display topology.
pub struct TopologyValidator;

impl TopologyValidator {
    pub const MIN_WIDTH: u32 = 320;
    pub const MAX_WIDTH: u32 = 15360; // 16K
    pub const MIN_HEIGHT: u32 = 240;
    pub const MAX_HEIGHT: u32 = 8640;

    pub const MIN_SCALE: f32 = 0.5;
    pub const MAX_SCALE: f32 = 4.0;

    pub const MAX_RESISTANCE_PX: u32 = 100;
    pub const MAX_DELAY_MS: u32 = 1000;
    pub const MAX_CORNER_LOCK_PX: u32 = 100;

    pub fn clamp_dimension(val: u32, is_width: bool) -> u32 {
        if is_width {
            val.clamp(Self::MIN_WIDTH, Self::MAX_WIDTH)
        } else {
            val.clamp(Self::MIN_HEIGHT, Self::MAX_HEIGHT)
        }
    }

    pub fn clamp_width(val: u32) -> u32 {
        Self::clamp_dimension(val, true)
    }

    pub fn clamp_height(val: u32) -> u32 {
        Self::clamp_dimension(val, false)
    }

    pub fn clamp_scale(val: f32) -> f32 {
        if val.is_nan() || val < Self::MIN_SCALE {
            Self::MIN_SCALE
        } else if val > Self::MAX_SCALE {
            Self::MAX_SCALE
        } else {
            val
        }
    }

    pub fn clamp_resistance(val: u32) -> u32 {
        val.min(Self::MAX_RESISTANCE_PX)
    }

    pub fn clamp_delay(val: u32) -> u32 {
        val.min(Self::MAX_DELAY_MS)
    }

    pub fn clamp_corner_lock(val: u32) -> u32 {
        val.min(Self::MAX_CORNER_LOCK_PX)
    }

    pub fn sanitize_peer_id(peer_id: &str) -> Result<String, CoreError> {
        let trimmed = peer_id.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(CoreError::InvalidPin);
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(CoreError::InvalidPin);
        }
        Ok(trimmed.to_string())
    }

    pub fn sanitize_request_id(req_id: &str) -> Result<String, CoreError> {
        let trimmed = req_id.trim();
        if trimmed.is_empty() || trimmed.len() > 64 {
            return Err(CoreError::InvalidPin);
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(CoreError::InvalidPin);
        }
        Ok(trimmed.to_string())
    }

    /// Checks if a cursor coordinate (x, y) falls inside the corner lock exclusion zone.
    pub fn is_in_corner_zone(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        corner_lock_size: u32,
    ) -> bool {
        if corner_lock_size == 0 {
            return false;
        }
        let lock = corner_lock_size as i32;
        let w = width as i32;
        let h = height as i32;

        let left = x < lock;
        let right = x >= w - lock;
        let top = y < lock;
        let bottom = y >= h - lock;

        (left && top) || (right && top) || (left && bottom) || (right && bottom)
    }
}

/// Computes the proportional entry coordinates on the target display when exiting
/// from a local display's border edge.
pub fn map_edge_exit_to_entry(
    local_display: &DisplayRect,
    local_edge: DisplayEdge,
    exit_pos: (i32, i32),
    remote_display: &DisplayRect,
    remote_edge: DisplayEdge,
    offset: i32,
) -> (i32, i32) {
    let lw = local_display.width.max(1) as f32;
    let lh = local_display.height.max(1) as f32;
    let rw = remote_display.width.max(1) as f32;
    let rh = remote_display.height.max(1) as f32;

    let (exit_x, exit_y) = exit_pos;

    match (local_edge, remote_edge) {
        // Horizontal: Left/Right to Left/Right
        (DisplayEdge::Right, DisplayEdge::Left) => {
            let frac_y = (exit_y as f32 / lh).clamp(0.0, 1.0);
            let entry_y = (frac_y * rh) as i32 + offset;
            (
                0,
                entry_y.clamp(0, remote_display.height.saturating_sub(1) as i32),
            )
        }
        (DisplayEdge::Left, DisplayEdge::Right) => {
            let frac_y = (exit_y as f32 / lh).clamp(0.0, 1.0);
            let entry_y = (frac_y * rh) as i32 + offset;
            (
                remote_display.width.saturating_sub(1) as i32,
                entry_y.clamp(0, remote_display.height.saturating_sub(1) as i32),
            )
        }
        // Vertical: Top/Bottom to Top/Bottom
        (DisplayEdge::Top, DisplayEdge::Bottom) => {
            let frac_x = (exit_x as f32 / lw).clamp(0.0, 1.0);
            let entry_x = (frac_x * rw) as i32 + offset;
            (
                entry_x.clamp(0, remote_display.width.saturating_sub(1) as i32),
                remote_display.height.saturating_sub(1) as i32,
            )
        }
        (DisplayEdge::Bottom, DisplayEdge::Top) => {
            let frac_x = (exit_x as f32 / lw).clamp(0.0, 1.0);
            let entry_x = (frac_x * rw) as i32 + offset;
            (
                entry_x.clamp(0, remote_display.width.saturating_sub(1) as i32),
                0,
            )
        }
        // Orthogonal or same edge fallback
        (DisplayEdge::Right, DisplayEdge::Right) => {
            let frac_y = (exit_y as f32 / lh).clamp(0.0, 1.0);
            let entry_y = (frac_y * rh) as i32 + offset;
            (
                remote_display.width.saturating_sub(1) as i32,
                entry_y.clamp(0, remote_display.height.saturating_sub(1) as i32),
            )
        }
        (DisplayEdge::Left, DisplayEdge::Left) => {
            let frac_y = (exit_y as f32 / lh).clamp(0.0, 1.0);
            let entry_y = (frac_y * rh) as i32 + offset;
            (
                0,
                entry_y.clamp(0, remote_display.height.saturating_sub(1) as i32),
            )
        }
        (DisplayEdge::Top, DisplayEdge::Top) => {
            let frac_x = (exit_x as f32 / lw).clamp(0.0, 1.0);
            let entry_x = (frac_x * rw) as i32 + offset;
            (
                entry_x.clamp(0, remote_display.width.saturating_sub(1) as i32),
                0,
            )
        }
        (DisplayEdge::Bottom, DisplayEdge::Bottom) => {
            let frac_x = (exit_x as f32 / lw).clamp(0.0, 1.0);
            let entry_x = (frac_x * rw) as i32 + offset;
            (
                entry_x.clamp(0, remote_display.width.saturating_sub(1) as i32),
                remote_display.height.saturating_sub(1) as i32,
            )
        }
        // Right to Top/Bottom
        (DisplayEdge::Right, DisplayEdge::Top) => {
            let frac_y = (exit_y as f32 / lh).clamp(0.0, 1.0);
            let entry_x = (frac_y * rw) as i32 + offset;
            (
                entry_x.clamp(0, remote_display.width.saturating_sub(1) as i32),
                0,
            )
        }
        (DisplayEdge::Right, DisplayEdge::Bottom) => {
            let frac_y = (exit_y as f32 / lh).clamp(0.0, 1.0);
            let entry_x = (frac_y * rw) as i32 + offset;
            (
                entry_x.clamp(0, remote_display.width.saturating_sub(1) as i32),
                remote_display.height.saturating_sub(1) as i32,
            )
        }
        // Left to Top/Bottom
        (DisplayEdge::Left, DisplayEdge::Top) => {
            let frac_y = (exit_y as f32 / lh).clamp(0.0, 1.0);
            let entry_x = (frac_y * rw) as i32 + offset;
            (
                entry_x.clamp(0, remote_display.width.saturating_sub(1) as i32),
                0,
            )
        }
        (DisplayEdge::Left, DisplayEdge::Bottom) => {
            let frac_y = (exit_y as f32 / lh).clamp(0.0, 1.0);
            let entry_x = (frac_y * rw) as i32 + offset;
            (
                entry_x.clamp(0, remote_display.width.saturating_sub(1) as i32),
                remote_display.height.saturating_sub(1) as i32,
            )
        }
        // Top to Left/Right
        (DisplayEdge::Top, DisplayEdge::Left) => {
            let frac_x = (exit_x as f32 / lw).clamp(0.0, 1.0);
            let entry_y = (frac_x * rh) as i32 + offset;
            (
                0,
                entry_y.clamp(0, remote_display.height.saturating_sub(1) as i32),
            )
        }
        (DisplayEdge::Top, DisplayEdge::Right) => {
            let frac_x = (exit_x as f32 / lw).clamp(0.0, 1.0);
            let entry_y = (frac_x * rh) as i32 + offset;
            (
                remote_display.width.saturating_sub(1) as i32,
                entry_y.clamp(0, remote_display.height.saturating_sub(1) as i32),
            )
        }
        // Bottom to Left/Right
        (DisplayEdge::Bottom, DisplayEdge::Left) => {
            let frac_x = (exit_x as f32 / lw).clamp(0.0, 1.0);
            let entry_y = (frac_x * rh) as i32 + offset;
            (
                0,
                entry_y.clamp(0, remote_display.height.saturating_sub(1) as i32),
            )
        }
        (DisplayEdge::Bottom, DisplayEdge::Right) => {
            let frac_x = (exit_x as f32 / lw).clamp(0.0, 1.0);
            let entry_y = (frac_x * rh) as i32 + offset;
            (
                remote_display.width.saturating_sub(1) as i32,
                entry_y.clamp(0, remote_display.height.saturating_sub(1) as i32),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_edge_opposite_and_parsing() {
        assert_eq!(DisplayEdge::Left.opposite(), DisplayEdge::Right);
        assert_eq!(DisplayEdge::Right.opposite(), DisplayEdge::Left);
        assert_eq!(DisplayEdge::Top.opposite(), DisplayEdge::Bottom);
        assert_eq!(DisplayEdge::Bottom.opposite(), DisplayEdge::Top);

        assert_eq!("left".parse::<DisplayEdge>().unwrap(), DisplayEdge::Left);
        assert_eq!("RIGHT".parse::<DisplayEdge>().unwrap(), DisplayEdge::Right);
        assert_eq!("up".parse::<DisplayEdge>().unwrap(), DisplayEdge::Top);
        assert_eq!("down".parse::<DisplayEdge>().unwrap(), DisplayEdge::Bottom);
        assert!("invalid".parse::<DisplayEdge>().is_err());
    }

    #[test]
    fn test_topology_validator_clamping() {
        assert_eq!(TopologyValidator::clamp_dimension(100, true), 320);
        assert_eq!(TopologyValidator::clamp_dimension(20000, true), 15360);
        assert_eq!(TopologyValidator::clamp_dimension(50, false), 240);
        assert_eq!(TopologyValidator::clamp_dimension(10000, false), 8640);

        assert_eq!(TopologyValidator::clamp_scale(0.1), 0.5);
        assert_eq!(TopologyValidator::clamp_scale(5.0), 4.0);
        assert_eq!(TopologyValidator::clamp_resistance(150), 100);
        assert_eq!(TopologyValidator::clamp_delay(1500), 1000);
        assert_eq!(TopologyValidator::clamp_corner_lock(150), 100);

        assert!(TopologyValidator::sanitize_peer_id("peer-123_abc").is_ok());
        assert!(TopologyValidator::sanitize_peer_id("peer@bad").is_err());
        assert!(TopologyValidator::sanitize_peer_id("").is_err());
    }

    #[test]
    fn test_corner_lock_zone_detection() {
        // 1920x1080 with 30px corner lock
        assert!(TopologyValidator::is_in_corner_zone(10, 10, 1920, 1080, 30)); // Top-Left
        assert!(TopologyValidator::is_in_corner_zone(
            1910, 10, 1920, 1080, 30
        )); // Top-Right
        assert!(TopologyValidator::is_in_corner_zone(
            10, 1070, 1920, 1080, 30
        )); // Bottom-Left
        assert!(TopologyValidator::is_in_corner_zone(
            1910, 1070, 1920, 1080, 30
        )); // Bottom-Right

        // Middle edges are not in corner zone
        assert!(!TopologyValidator::is_in_corner_zone(
            1919, 540, 1920, 1080, 30
        )); // Mid-Right
        assert!(!TopologyValidator::is_in_corner_zone(
            0, 540, 1920, 1080, 30
        )); // Mid-Left
        assert!(!TopologyValidator::is_in_corner_zone(
            960, 0, 1920, 1080, 30
        )); // Mid-Top
        assert!(!TopologyValidator::is_in_corner_zone(
            960, 1079, 1920, 1080, 30
        )); // Mid-Bottom
    }

    #[test]
    fn test_map_edge_exit_to_entry_scaling() {
        let local = DisplayRect::new(1920, 1080, 1.0);
        let remote = DisplayRect::new(3840, 2160, 1.0);

        // Exit at Right midpoint (1919, 540) -> Enter Remote Left (0, 1080)
        let (rx, ry) = map_edge_exit_to_entry(
            &local,
            DisplayEdge::Right,
            (1919, 540),
            &remote,
            DisplayEdge::Left,
            0,
        );
        assert_eq!(rx, 0);
        assert_eq!(ry, 1080);

        // Exit at Top midpoint (960, 0) -> Enter Remote Bottom (1920, 2159)
        let (rx, ry) = map_edge_exit_to_entry(
            &local,
            DisplayEdge::Top,
            (960, 0),
            &remote,
            DisplayEdge::Bottom,
            0,
        );
        assert_eq!(rx, 1920);
        assert_eq!(ry, 2159);
    }

    #[test]
    fn test_topology_frames_roundtrip() {
        let mut neighbors = HashMap::new();
        neighbors.insert(
            DisplayEdge::Right,
            EdgeNeighbor {
                peer_id: "peer_b".to_string(),
                peer_edge: DisplayEdge::Left,
                offset: 0,
            },
        );

        let frame = TopologyStateFrame {
            device_id: "peer_a".to_string(),
            display: DisplayRect::new(1920, 1080, 1.0),
            neighbors,
            config: EdgeCrossingConfig::default(),
            timestamp_ms: 123456789,
        };

        let inner = frame.to_frame().unwrap();
        assert_eq!(inner.ty, TYPE_TOPOLOGY_STATE);
        let decoded = TopologyStateFrame::from_payload(&inner.payload).unwrap();
        assert_eq!(decoded.device_id, "peer_a");
        assert_eq!(decoded.display.width, 1920);
        assert_eq!(
            decoded.neighbors.get(&DisplayEdge::Right).unwrap().peer_id,
            "peer_b"
        );

        let req = TopologySyncReqFrame {
            request_id: "req-999".to_string(),
            timestamp_ms: 123456789,
        };
        let inner_req = req.to_frame().unwrap();
        assert_eq!(inner_req.ty, TYPE_TOPOLOGY_SYNC_REQ);
        let decoded_req = TopologySyncReqFrame::from_payload(&inner_req.payload).unwrap();
        assert_eq!(decoded_req.request_id, "req-999");
    }
}
