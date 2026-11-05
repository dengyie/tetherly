// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Desktop input protocol. Binary frames only — never JSON mouse/keyboard.
//! OS injection lives in tetherly-node; this module is OS-free.

use crate::error::CoreError;
use crate::frame::{InnerFrame, FLAG_MUST_UNDERSTAND, INNER_PAYLOAD_MAX};
use std::collections::VecDeque;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Input channel inner types. Not listed in the JSON control-plane table.
pub const TYPE_INPUT_ENTER: u16 = 0x0401;
pub const TYPE_INPUT_LEAVE: u16 = 0x0402;
pub const TYPE_INPUT_MOVE: u16 = 0x0403;
pub const TYPE_INPUT_BUTTON: u16 = 0x0404;
pub const TYPE_INPUT_WHEEL: u16 = 0x0405;
pub const TYPE_INPUT_KEY: u16 = 0x0406;
pub const TYPE_INPUT_CLIP_HINT: u16 = 0x0407;

pub const INPUT_MAGIC: [u8; 4] = *b"TIN1";
pub const INPUT_PROTO: u8 = 1;
pub const INPUT_BATCH_MAX: usize = 64;
pub const INPUT_SEQ_WINDOW: u64 = 65_536;
/// Most recent applied events `MemorySink` retains, so a long-lived client
/// cannot grow the trace (or its per-frame clone) without bound.
pub const INPUT_SINK_TRACE_MAX: usize = 256;

pub const BTN_LEFT: u8 = 1;
pub const BTN_RIGHT: u8 = 2;
pub const BTN_MIDDLE: u8 = 4;

pub const KEY_DOWN: u8 = 1;
pub const KEY_UP: u8 = 0;
pub const BUTTON_DOWN: u8 = 1;
pub const BUTTON_UP: u8 = 0;

/// Logical screen in tenths of a percent (0..=1000). Avoids float on the wire.
pub const SCREEN_UNITS: i32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Enter,
    Leave,
    Move { x: i32, y: i32 },
    Button { mask: u8, down: bool },
    Wheel { dx: i16, dy: i16 },
    Key { code: u16, down: bool, mods: u8 },
    ClipHint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEvent {
    pub seq: u64,
    pub kind: InputKind,
}

impl InputEvent {
    pub fn enter(seq: u64) -> Self {
        Self {
            seq,
            kind: InputKind::Enter,
        }
    }

    pub fn leave(seq: u64) -> Self {
        Self {
            seq,
            kind: InputKind::Leave,
        }
    }

    pub fn move_to(seq: u64, x: i32, y: i32) -> Self {
        Self {
            seq,
            kind: InputKind::Move { x, y },
        }
    }

    pub fn button(seq: u64, mask: u8, down: bool) -> Self {
        Self {
            seq,
            kind: InputKind::Button { mask, down },
        }
    }

    pub fn wheel(seq: u64, dx: i16, dy: i16) -> Self {
        Self {
            seq,
            kind: InputKind::Wheel { dx, dy },
        }
    }

    pub fn key(seq: u64, code: u16, down: bool, mods: u8) -> Self {
        Self {
            seq,
            kind: InputKind::Key { code, down, mods },
        }
    }

    pub fn clip_hint(seq: u64) -> Self {
        Self {
            seq,
            kind: InputKind::ClipHint,
        }
    }

    fn type_id(&self) -> u16 {
        match self.kind {
            InputKind::Enter => TYPE_INPUT_ENTER,
            InputKind::Leave => TYPE_INPUT_LEAVE,
            InputKind::Move { .. } => TYPE_INPUT_MOVE,
            InputKind::Button { .. } => TYPE_INPUT_BUTTON,
            InputKind::Wheel { .. } => TYPE_INPUT_WHEEL,
            InputKind::Key { .. } => TYPE_INPUT_KEY,
            InputKind::ClipHint => TYPE_INPUT_CLIP_HINT,
        }
    }

    pub fn encode_payload(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(16);
        out.extend_from_slice(&INPUT_MAGIC);
        out.push(INPUT_PROTO);
        out.extend_from_slice(&self.seq.to_be_bytes());
        match self.kind {
            InputKind::Enter | InputKind::Leave | InputKind::ClipHint => {}
            InputKind::Move { x, y } => {
                out.extend_from_slice(&x.to_be_bytes());
                out.extend_from_slice(&y.to_be_bytes());
            }
            InputKind::Button { mask, down } => {
                out.push(mask);
                out.push(if down { BUTTON_DOWN } else { BUTTON_UP });
            }
            InputKind::Wheel { dx, dy } => {
                out.extend_from_slice(&dx.to_be_bytes());
                out.extend_from_slice(&dy.to_be_bytes());
            }
            InputKind::Key { code, down, mods } => {
                out.extend_from_slice(&code.to_be_bytes());
                out.push(if down { KEY_DOWN } else { KEY_UP });
                out.push(mods);
            }
        }
        out
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = self.encode_payload();
        if payload.len() > INNER_PAYLOAD_MAX {
            return Err(CoreError::PayloadTooLarge);
        }
        Ok(InnerFrame {
            ty: self.type_id(),
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 0,
            payload,
        })
    }

    pub fn from_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        decode_event(frame.ty, &frame.payload)
    }
}

pub fn is_input_type(ty: u16) -> bool {
    matches!(
        ty,
        TYPE_INPUT_ENTER
            | TYPE_INPUT_LEAVE
            | TYPE_INPUT_MOVE
            | TYPE_INPUT_BUTTON
            | TYPE_INPUT_WHEEL
            | TYPE_INPUT_KEY
            | TYPE_INPUT_CLIP_HINT
    )
}

fn decode_event(ty: u16, payload: &[u8]) -> Result<InputEvent, CoreError> {
    if payload.len() < 13 {
        return Err(CoreError::TruncatedFrame);
    }
    if payload[0..4] != INPUT_MAGIC {
        return Err(CoreError::InputFrame);
    }
    if payload[4] != INPUT_PROTO {
        return Err(CoreError::UnsupportedProto);
    }
    let seq = u64::from_be_bytes(
        payload[5..13]
            .try_into()
            .map_err(|_| CoreError::TruncatedFrame)?,
    );
    let rest = &payload[13..];
    let kind = match ty {
        TYPE_INPUT_ENTER => {
            if !rest.is_empty() {
                return Err(CoreError::InputFrame);
            }
            InputKind::Enter
        }
        TYPE_INPUT_LEAVE => {
            if !rest.is_empty() {
                return Err(CoreError::InputFrame);
            }
            InputKind::Leave
        }
        TYPE_INPUT_CLIP_HINT => {
            if !rest.is_empty() {
                return Err(CoreError::InputFrame);
            }
            InputKind::ClipHint
        }
        TYPE_INPUT_MOVE => {
            if rest.len() != 8 {
                return Err(CoreError::TruncatedFrame);
            }
            let x = i32::from_be_bytes(rest[0..4].try_into().unwrap());
            let y = i32::from_be_bytes(rest[4..8].try_into().unwrap());
            if !(0..=SCREEN_UNITS).contains(&x) || !(0..=SCREEN_UNITS).contains(&y) {
                return Err(CoreError::InputFrame);
            }
            InputKind::Move { x, y }
        }
        TYPE_INPUT_BUTTON => {
            if rest.len() != 2 {
                return Err(CoreError::TruncatedFrame);
            }
            InputKind::Button {
                mask: rest[0],
                down: rest[1] == BUTTON_DOWN,
            }
        }
        TYPE_INPUT_WHEEL => {
            if rest.len() != 4 {
                return Err(CoreError::TruncatedFrame);
            }
            InputKind::Wheel {
                dx: i16::from_be_bytes(rest[0..2].try_into().unwrap()),
                dy: i16::from_be_bytes(rest[2..4].try_into().unwrap()),
            }
        }
        TYPE_INPUT_KEY => {
            if rest.len() != 4 {
                return Err(CoreError::TruncatedFrame);
            }
            InputKind::Key {
                code: u16::from_be_bytes(rest[0..2].try_into().unwrap()),
                down: rest[2] == KEY_DOWN,
                mods: rest[3],
            }
        }
        _ => return Err(CoreError::InputFrame),
    };
    Ok(InputEvent { seq, kind })
}

/// Where the logical cursor currently lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorSeat {
    Local,
    Remote,
}

/// Edge a cursor can leave through (supports 4-direction spatial arrangement like Deskflow).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl ScreenEdge {
    /// Returns the geometrically opposing edge.
    pub fn opposite(&self) -> Self {
        match self {
            ScreenEdge::Left => ScreenEdge::Right,
            ScreenEdge::Right => ScreenEdge::Left,
            ScreenEdge::Top => ScreenEdge::Bottom,
            ScreenEdge::Bottom => ScreenEdge::Top,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            ScreenEdge::Left => "left",
            ScreenEdge::Right => "right",
            ScreenEdge::Top => "top",
            ScreenEdge::Bottom => "bottom",
        }
    }
}

impl fmt::Display for ScreenEdge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for ScreenEdge {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "left" => Ok(ScreenEdge::Left),
            "right" => Ok(ScreenEdge::Right),
            "top" | "up" => Ok(ScreenEdge::Top),
            "bottom" | "down" => Ok(ScreenEdge::Bottom),
            _ => Err(CoreError::Json(format!("invalid screen edge '{s}'"))),
        }
    }
}

impl From<crate::topology::DisplayEdge> for ScreenEdge {
    fn from(d: crate::topology::DisplayEdge) -> Self {
        match d {
            crate::topology::DisplayEdge::Left => ScreenEdge::Left,
            crate::topology::DisplayEdge::Right => ScreenEdge::Right,
            crate::topology::DisplayEdge::Top => ScreenEdge::Top,
            crate::topology::DisplayEdge::Bottom => ScreenEdge::Bottom,
        }
    }
}

impl From<ScreenEdge> for crate::topology::DisplayEdge {
    fn from(s: ScreenEdge) -> Self {
        match s {
            ScreenEdge::Left => crate::topology::DisplayEdge::Left,
            ScreenEdge::Right => crate::topology::DisplayEdge::Right,
            ScreenEdge::Top => crate::topology::DisplayEdge::Top,
            ScreenEdge::Bottom => crate::topology::DisplayEdge::Bottom,
        }
    }
}

pub trait InputSink: Send + Sync {
    fn move_abs(&mut self, x: i32, y: i32);
    fn button(&mut self, mask: u8, down: bool);
    fn wheel(&mut self, dx: i16, dy: i16);
    fn key(&mut self, code: u16, down: bool, mods: u8);
}

/// In-memory sink for tests. Never talks to the OS.
#[derive(Debug, Default, Clone)]
pub struct MemorySink {
    pub cursor: (i32, i32),
    pub buttons: u8,
    pub keys_down: Vec<u16>,
    pub applied: VecDeque<InputKind>,
}

impl MemorySink {
    fn record(&mut self, kind: InputKind) {
        if self.applied.len() >= INPUT_SINK_TRACE_MAX {
            self.applied.pop_front();
        }
        self.applied.push_back(kind);
    }
}

impl InputSink for MemorySink {
    fn move_abs(&mut self, x: i32, y: i32) {
        self.cursor = (x, y);
        self.record(InputKind::Move { x, y });
    }

    fn button(&mut self, mask: u8, down: bool) {
        if down {
            self.buttons |= mask;
        } else {
            self.buttons &= !mask;
        }
        self.record(InputKind::Button { mask, down });
    }

    fn wheel(&mut self, dx: i16, dy: i16) {
        self.record(InputKind::Wheel { dx, dy });
    }

    fn key(&mut self, code: u16, down: bool, mods: u8) {
        if down {
            if !self.keys_down.contains(&code) {
                self.keys_down.push(code);
            }
        } else {
            self.keys_down.retain(|c| *c != code);
        }
        self.record(InputKind::Key { code, down, mods });
    }
}

/// Server-side capture: local cursor, detect edge leave, encode outbound events.
pub struct InputServer {
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    rem_x: i32,
    rem_y: i32,
    seat: CursorSeat,
    edge: ScreenEdge,
    last_crossed_edge: ScreenEdge,
    remote_interior: bool,
    buttons_down: u8,
    next_seq: u64,
    pending: VecDeque<InputEvent>,
}

impl InputServer {
    pub fn new(width: i32, height: i32) -> Self {
        let w = width.max(1);
        let h = height.max(1);
        Self {
            width: w,
            height: h,
            x: w / 2,
            y: h / 2,
            rem_x: SCREEN_UNITS / 2,
            rem_y: SCREEN_UNITS / 2,
            seat: CursorSeat::Local,
            edge: ScreenEdge::Right,
            last_crossed_edge: ScreenEdge::Right,
            remote_interior: false,
            buttons_down: 0,
            next_seq: 1,
            pending: VecDeque::with_capacity(INPUT_BATCH_MAX),
        }
    }

    pub fn seat(&self) -> CursorSeat {
        self.seat
    }

    pub fn edge(&self) -> ScreenEdge {
        self.edge
    }

    pub fn set_edge(&mut self, edge: ScreenEdge) {
        self.edge = edge;
    }

    pub fn last_crossed_edge(&self) -> ScreenEdge {
        self.last_crossed_edge
    }

    pub fn note_button(&mut self, mask: u8, down: bool) {
        if down {
            self.buttons_down |= mask;
        } else {
            self.buttons_down &= !mask;
        }
    }

    pub fn buttons_down(&self) -> u8 {
        self.buttons_down
    }

    /// Forcefully disarm remote roaming and restore the cursor seat to Local.
    pub fn force_local(&mut self) {
        self.buttons_down = 0;
        if self.seat == CursorSeat::Remote {
            self.seat = CursorSeat::Local;
            self.remote_interior = false;
            self.push(InputKind::Leave);
            // Park on the crossed edge, but map the free axis back from the
            // last remote position so an emergency return lands at the
            // screen location matching where the cursor was on the target
            // (Deskflow semantics), not wherever the parked hook last saw.
            match self.last_crossed_edge {
                ScreenEdge::Right => {
                    self.x = self.width.saturating_sub(17).max(0);
                    self.y = self.denorm_y(self.rem_y);
                }
                ScreenEdge::Left => {
                    self.x = 16.min(self.width.saturating_sub(1));
                    self.y = self.denorm_y(self.rem_y);
                }
                ScreenEdge::Top => {
                    self.y = 16.min(self.height.saturating_sub(1));
                    self.x = self.denorm_x(self.rem_x);
                }
                ScreenEdge::Bottom => {
                    self.y = self.height.saturating_sub(17).max(0);
                    self.x = self.denorm_x(self.rem_x);
                }
            }
        }
    }

    /// Change the logical screen without resetting `seq`.
    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
    }

    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn note_seq(&mut self, seq: u64) {
        if seq >= self.next_seq {
            self.next_seq = seq.saturating_add(1);
        }
    }

    pub fn local_pos(&self) -> (i32, i32) {
        (self.x, self.y)
    }

    pub fn remote_pos(&self) -> (i32, i32) {
        (self.rem_x, self.rem_y)
    }

    /// Prepare resume events if re-attaching while cursor was already in remote seat.
    pub fn resume_remote(&mut self) {
        if self.seat == CursorSeat::Remote {
            self.push(InputKind::Enter);
            self.push(InputKind::Move {
                x: self.rem_x,
                y: self.rem_y,
            });
        }
    }

    /// Re-assert the remote session after a policy change that alters how
    /// key events are interpreted (e.g. keymap profile switch). The client
    /// flushes every held key/button on `Leave` (`release_all_inputs`), then
    /// re-enters at the current cursor position, so a modifier held across
    /// the switch cannot end up stuck under the old mapping. Seat stays
    /// `Remote` and the seq keeps increasing.
    pub fn refresh_remote_session(&mut self) {
        if self.seat == CursorSeat::Remote {
            self.push(InputKind::Leave);
            self.push(InputKind::Enter);
            self.push(InputKind::Move {
                x: self.rem_x,
                y: self.rem_y,
            });
        }
    }

    fn push(&mut self, kind: InputKind) {
        if self.pending.len() >= INPUT_BATCH_MAX {
            self.pending.pop_front();
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.pending.push_back(InputEvent { seq, kind });
    }

    /// Motion with relative displacement deltas. When the seat is remote,
    /// deltas drive smooth free roaming on the target screen without cursor locking.
    pub fn local_move_delta(&mut self, x: i32, y: i32, dx: i32, dy: i32) -> Option<ScreenEdge> {
        let y_clamped = y.clamp(0, self.height.saturating_sub(1));
        let x_clamped = x.clamp(0, self.width.saturating_sub(1));
        if self.seat == CursorSeat::Remote {
            let crossed_edge = self.last_crossed_edge;
            let delta_x = if dx != 0 {
                let d = (dx as i64 * SCREEN_UNITS as i64 / self.width.max(1) as i64) as i32;
                if d == 0 {
                    dx.signum()
                } else {
                    d
                }
            } else {
                0
            };
            let delta_y = if dy != 0 {
                let d = (dy as i64 * SCREEN_UNITS as i64 / self.height.max(1) as i64) as i32;
                if d == 0 {
                    dy.signum()
                } else {
                    d
                }
            } else {
                0
            };

            let next_rem_x = (self.rem_x + delta_x).clamp(0, SCREEN_UNITS);
            let next_rem_y = (self.rem_y + delta_y).clamp(0, SCREEN_UNITS);

            const REMOTE_INTERIOR_HYSTERESIS: i32 = 20;
            let is_interior = match crossed_edge {
                ScreenEdge::Top => next_rem_y < SCREEN_UNITS - REMOTE_INTERIOR_HYSTERESIS,
                ScreenEdge::Bottom => next_rem_y > REMOTE_INTERIOR_HYSTERESIS,
                ScreenEdge::Left => next_rem_x < SCREEN_UNITS - REMOTE_INTERIOR_HYSTERESIS,
                ScreenEdge::Right => next_rem_x > REMOTE_INTERIOR_HYSTERESIS,
            };
            if is_interior {
                self.remote_interior = true;
            }

            // Do not switch screen back to local while holding down a mouse button (prevent drag interruption)
            let returning_local = self.remote_interior
                && (self.buttons_down == 0)
                && match crossed_edge {
                    ScreenEdge::Top => {
                        (self.rem_y >= SCREEN_UNITS && dy > 4)
                            || (self.rem_y + delta_y >= SCREEN_UNITS && dy > 4)
                    }
                    ScreenEdge::Bottom => {
                        (self.rem_y <= 0 && dy < -4) || (self.rem_y + delta_y <= 0 && dy < -4)
                    }
                    ScreenEdge::Left => {
                        (self.rem_x >= SCREEN_UNITS && dx > 4)
                            || (self.rem_x + delta_x >= SCREEN_UNITS && dx > 4)
                    }
                    ScreenEdge::Right => {
                        (self.rem_x <= 0 && dx < -4) || (self.rem_x + delta_x <= 0 && dx < -4)
                    }
                };

            if returning_local {
                self.seat = CursorSeat::Local;
                self.remote_interior = false;
                self.push(InputKind::Leave);
                // Deskflow OSXScreen semantics: the return parks on the
                // crossed edge, and the FREE axis maps back from the last
                // remote (0..=1000) position so the cursor re-enters this
                // screen at the location matching where it left the target.
                // Using the raw hook x/y here would land wherever the parked
                // local cursor sat (screen center on macOS), losing the
                // cross-screen position mapping in both directions.
                match crossed_edge {
                    ScreenEdge::Top => {
                        self.x = self.denorm_x(self.rem_x);
                        self.y = 16.min(self.height.saturating_sub(1));
                    }
                    ScreenEdge::Bottom => {
                        self.x = self.denorm_x(self.rem_x);
                        self.y = self.height.saturating_sub(17).max(0);
                    }
                    ScreenEdge::Left => {
                        self.x = 16.min(self.width.saturating_sub(1));
                        self.y = self.denorm_y(self.rem_y);
                    }
                    ScreenEdge::Right => {
                        self.x = self.width.saturating_sub(17).max(0);
                        self.y = self.denorm_y(self.rem_y);
                    }
                }
                return Some(crossed_edge);
            }

            self.rem_x = next_rem_x;
            self.rem_y = next_rem_y;
            self.x = x_clamped;
            self.y = y_clamped;
            self.push(InputKind::Move {
                x: self.rem_x,
                y: self.rem_y,
            });
            return None;
        }

        match self.edge {
            ScreenEdge::Right => {
                if x >= self.width {
                    self.x = self.width.saturating_sub(1);
                    self.y = y_clamped;
                    self.rem_x = 0;
                    self.rem_y = self.norm_y();
                    self.seat = CursorSeat::Remote;
                    self.remote_interior = false;
                    self.last_crossed_edge = ScreenEdge::Right;
                    self.push(InputKind::Enter);
                    self.push(InputKind::Move {
                        x: self.rem_x,
                        y: self.rem_y,
                    });
                    return Some(ScreenEdge::Right);
                }
                if x < 0 {
                    self.x = 0;
                    self.y = y_clamped;
                    return None;
                }
            }
            ScreenEdge::Left => {
                if x <= 0 || (x <= 4 && dx < 0) {
                    self.x = 0;
                    self.y = y_clamped;
                    self.rem_x = SCREEN_UNITS;
                    self.rem_y = self.norm_y();
                    self.seat = CursorSeat::Remote;
                    self.remote_interior = false;
                    self.last_crossed_edge = ScreenEdge::Left;
                    self.push(InputKind::Enter);
                    self.push(InputKind::Move {
                        x: self.rem_x,
                        y: self.rem_y,
                    });
                    return Some(ScreenEdge::Left);
                }
                if x >= self.width {
                    self.x = self.width.saturating_sub(1);
                    self.y = y_clamped;
                    return None;
                }
            }
            ScreenEdge::Top => {
                if y <= 0 || (y <= 4 && dy < 0) {
                    self.x = x_clamped;
                    self.y = 0;
                    self.rem_x = self.norm_x();
                    self.rem_y = SCREEN_UNITS;
                    self.seat = CursorSeat::Remote;
                    self.remote_interior = false;
                    self.last_crossed_edge = ScreenEdge::Top;
                    self.push(InputKind::Enter);
                    self.push(InputKind::Move {
                        x: self.rem_x,
                        y: self.rem_y,
                    });
                    return Some(ScreenEdge::Top);
                }
                if y >= self.height {
                    self.x = x_clamped;
                    self.y = self.height.saturating_sub(1);
                    return None;
                }
            }
            ScreenEdge::Bottom => {
                if y >= self.height {
                    self.x = x_clamped;
                    self.y = self.height.saturating_sub(1);
                    self.rem_x = self.norm_x();
                    self.rem_y = 0;
                    self.seat = CursorSeat::Remote;
                    self.remote_interior = false;
                    self.last_crossed_edge = ScreenEdge::Bottom;
                    self.push(InputKind::Enter);
                    self.push(InputKind::Move {
                        x: self.rem_x,
                        y: self.rem_y,
                    });
                    return Some(ScreenEdge::Bottom);
                }
                if y <= 0 {
                    self.x = x_clamped;
                    self.y = 0;
                    return None;
                }
            }
        }
        self.x = x_clamped;
        self.y = y_clamped;
        None
    }

    pub fn local_key(&mut self, code: u16, down: bool, mods: u8) {
        if self.seat == CursorSeat::Remote {
            self.push(InputKind::Key { code, down, mods });
        }
    }

    /// Remote reports the cursor left back through its entry edge.
    pub fn on_remote_leave(&mut self) {
        self.seat = CursorSeat::Local;
        self.remote_interior = false;
        self.buttons_down = 0;
        match self.last_crossed_edge {
            ScreenEdge::Right => self.x = self.width.saturating_sub(1),
            ScreenEdge::Left => self.x = 1.min(self.width.saturating_sub(1)),
            ScreenEdge::Top => self.y = 1.min(self.height.saturating_sub(1)),
            ScreenEdge::Bottom => self.y = self.height.saturating_sub(1),
        }
    }

    /// Peer input session died: keyboard/mouse return here (M3.2).
    pub fn on_peer_gone(&mut self) {
        self.pending.clear();
        self.seat = CursorSeat::Local;
        self.remote_interior = false;
        self.buttons_down = 0;
        self.x = self.width / 2;
        self.y = self.height / 2;
    }

    pub fn drain(&mut self) -> Vec<InputEvent> {
        self.pending.drain(..).collect()
    }

    fn norm_x(&self) -> i32 {
        if self.width <= 1 {
            return 0;
        }
        // Round, not truncate: the remote side's pixel mapping rounds too.
        // Truncation on both hops shears every round trip in one direction,
        // so repeated crossings visibly walk the cursor (production bug).
        ((self.x as i64 * SCREEN_UNITS as i64 + (self.width as i64 - 1) / 2)
            / (self.width as i64 - 1)) as i32
    }

    fn norm_y(&self) -> i32 {
        if self.height <= 1 {
            return 0;
        }
        ((self.y as i64 * SCREEN_UNITS as i64 + (self.height as i64 - 1) / 2)
            / (self.height as i64 - 1)) as i32
    }

    /// Inverse of `norm_x`: map a remote 0..=1000 position back onto this
    /// screen's pixel axis. Used for the local landing position when the
    /// cursor returns from the target screen.
    fn denorm_x(&self, v: i32) -> i32 {
        // Round to nearest; must mirror norm_x so round trips are stable.
        ((v.clamp(0, SCREEN_UNITS) as i64 * (self.width.max(1) as i64 - 1)
            + SCREEN_UNITS as i64 / 2)
            / SCREEN_UNITS as i64) as i32
    }

    /// Inverse of `norm_y`; see `denorm_x`.
    fn denorm_y(&self, v: i32) -> i32 {
        ((v.clamp(0, SCREEN_UNITS) as i64 * (self.height.max(1) as i64 - 1)
            + SCREEN_UNITS as i64 / 2)
            / SCREEN_UNITS as i64) as i32
    }
}

/// Client-side apply: decode, drop old seq, inject, leave when returning through entry edge.
pub struct InputClient<S: InputSink> {
    sink: S,
    last_seq: u64,
    focused: bool,
    autonomous_leave: bool,
    entry_edge: Option<ScreenEdge>,
    /// True after a focused move into the screen interior.
    interior: bool,
    dropped: u64,
    applied: u64,
    keys_down: std::collections::HashSet<u16>,
    buttons_down: u8,
}

impl<S: InputSink> InputClient<S> {
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            last_seq: 0,
            focused: false,
            autonomous_leave: false,
            entry_edge: None,
            interior: false,
            dropped: 0,
            applied: 0,
            keys_down: std::collections::HashSet::new(),
            buttons_down: 0,
        }
    }

    /// Enable legacy autonomous boundary leave for standalone tests.
    /// In production (Deskflow topology), the controlling server is the single
    /// source of truth for screen layout and boundary returns, and the secondary
    /// node acts as a pure slave sink.
    pub fn with_autonomous_leave(mut self, enabled: bool) -> Self {
        self.autonomous_leave = enabled;
        self
    }

    pub fn focused(&self) -> bool {
        self.focused
    }

    pub fn applied(&self) -> u64 {
        self.applied
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Release all currently held mouse buttons and keys.
    /// This prevents stuck keys and stuck mouse buttons on seat transitions,
    /// boundary crossings, and unexpected disconnections.
    pub fn release_all_inputs(&mut self) {
        if self.buttons_down != 0 {
            let btns = [BTN_LEFT, BTN_RIGHT, BTN_MIDDLE];
            for &mask in &btns {
                if (self.buttons_down & mask) != 0 {
                    self.sink.button(mask, false);
                }
            }
            self.buttons_down = 0;
        }
        if !self.keys_down.is_empty() {
            let held: Vec<u16> = self.keys_down.drain().collect();
            for code in held {
                self.sink.key(code, false, 0);
            }
        }
    }

    /// Apply one event. Returns `Some(Leave)` when we must send leave back.
    pub fn apply(&mut self, ev: InputEvent) -> Result<Option<InputEvent>, CoreError> {
        if ev.seq <= self.last_seq {
            self.dropped = self.dropped.saturating_add(1);
            return Ok(None);
        }
        if self.last_seq != 0 && ev.seq > self.last_seq.saturating_add(INPUT_SEQ_WINDOW) {
            return Err(CoreError::InputSeqJump);
        }
        self.last_seq = ev.seq;
        match ev.kind {
            InputKind::Enter => {
                self.release_all_inputs();
                self.focused = true;
                self.entry_edge = None;
                self.interior = false;
                self.applied = self.applied.saturating_add(1);
                Ok(None)
            }
            InputKind::Leave => {
                self.release_all_inputs();
                self.focused = false;
                self.entry_edge = None;
                self.interior = false;
                self.applied = self.applied.saturating_add(1);
                Ok(None)
            }
            InputKind::Move { x, y } => {
                if !self.focused {
                    // Resilient auto-focus on reconnect: if a remote controller reconnected
                    // and immediately transmits Move coordinates, accept focus instead of dropping.
                    self.focused = true;
                    self.entry_edge = None;
                    self.interior = false;
                }
                let leaving = if self.autonomous_leave {
                    let edge = *self.entry_edge.get_or_insert({
                        if x <= 0 {
                            ScreenEdge::Left
                        } else if x >= SCREEN_UNITS {
                            ScreenEdge::Right
                        } else if y <= 0 {
                            ScreenEdge::Top
                        } else if y >= SCREEN_UNITS {
                            ScreenEdge::Bottom
                        } else {
                            ScreenEdge::Left
                        }
                    });
                    const INTERIOR_HYSTERESIS: i32 = 20;
                    let (is_interior, leaving) = match edge {
                        ScreenEdge::Left => (x > INTERIOR_HYSTERESIS, x <= 0 && self.interior),
                        ScreenEdge::Right => (
                            x < SCREEN_UNITS - INTERIOR_HYSTERESIS,
                            x >= SCREEN_UNITS && self.interior,
                        ),
                        ScreenEdge::Top => (y > INTERIOR_HYSTERESIS, y <= 0 && self.interior),
                        ScreenEdge::Bottom => (
                            y < SCREEN_UNITS - INTERIOR_HYSTERESIS,
                            y >= SCREEN_UNITS && self.interior,
                        ),
                    };
                    if is_interior {
                        self.interior = true;
                    }
                    leaving
                } else {
                    false
                };

                self.sink.move_abs(x, y);
                self.applied = self.applied.saturating_add(1);
                if leaving {
                    self.release_all_inputs();
                    self.focused = false;
                    self.entry_edge = None;
                    self.interior = false;
                    return Ok(Some(InputEvent::leave(ev.seq.saturating_add(1))));
                }
                Ok(None)
            }
            InputKind::Button { mask, down } => {
                if self.focused {
                    if down {
                        self.buttons_down |= mask;
                    } else {
                        self.buttons_down &= !mask;
                    }
                    self.sink.button(mask, down);
                    self.applied = self.applied.saturating_add(1);
                } else {
                    self.dropped = self.dropped.saturating_add(1);
                }
                Ok(None)
            }
            InputKind::Wheel { dx, dy } => {
                if self.focused {
                    self.sink.wheel(dx, dy);
                    self.applied = self.applied.saturating_add(1);
                } else {
                    self.dropped = self.dropped.saturating_add(1);
                }
                Ok(None)
            }
            InputKind::Key { code, down, mods } => {
                if self.focused {
                    if down {
                        self.keys_down.insert(code);
                    } else {
                        self.keys_down.remove(&code);
                    }
                    self.sink.key(code, down, mods);
                    self.applied = self.applied.saturating_add(1);
                } else {
                    self.dropped = self.dropped.saturating_add(1);
                }
                Ok(None)
            }
            InputKind::ClipHint => {
                self.applied = self.applied.saturating_add(1);
                Ok(None)
            }
        }
    }

    pub fn on_peer_gone(&mut self) {
        self.release_all_inputs();
        self.focused = false;
        self.interior = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_roundtrip_not_json() {
        let ev = InputEvent::move_to(7, 12, 34);
        let frame = ev.to_frame().unwrap();
        assert!(is_input_type(frame.ty));
        assert_eq!(frame.flags & FLAG_MUST_UNDERSTAND, FLAG_MUST_UNDERSTAND);
        assert!(!frame.payload.starts_with(b"{"));
        assert_eq!(InputEvent::from_frame(&frame).unwrap(), ev);
        let key = InputEvent::key(8, 0x1E, true, 0);
        assert_eq!(
            InputEvent::from_frame(&key.to_frame().unwrap()).unwrap(),
            key
        );
    }

    #[test]
    fn reject_json_shaped_payload() {
        let frame = InnerFrame {
            ty: TYPE_INPUT_MOVE,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 1,
            payload: br#"{"x":1,"y":2}"#.to_vec(),
        };
        assert_eq!(InputEvent::from_frame(&frame), Err(CoreError::InputFrame));
    }

    #[test]
    fn edge_enter_and_keyboard_follow() {
        let mut srv = InputServer::new(100, 50);
        assert_eq!(srv.local_move_delta(99, 10, 0, 0), None);
        assert_eq!(srv.seat(), CursorSeat::Local);
        assert_eq!(srv.local_move_delta(100, 10, 0, 0), Some(ScreenEdge::Right));
        assert_eq!(srv.seat(), CursorSeat::Remote);
        srv.local_key(0x1E, true, 0);
        srv.local_key(0x1E, false, 0);
        let out = srv.drain();
        assert_eq!(out[0].kind, InputKind::Enter);
        assert!(matches!(out[1].kind, InputKind::Move { x: 0, .. }));
        assert!(matches!(
            out[2].kind,
            InputKind::Key {
                code: 0x1E,
                down: true,
                ..
            }
        ));
        assert!(matches!(
            out[3].kind,
            InputKind::Key {
                code: 0x1E,
                down: false,
                ..
            }
        ));
    }

    #[test]
    fn one_hundred_round_trips_zero_loss() {
        let mut srv = InputServer::new(200, 100);
        let mut cli = InputClient::new(MemorySink::default()).with_autonomous_leave(true);
        let mut lost = 0u32;
        for _ in 0..100 {
            assert_eq!(srv.local_move_delta(200, 40, 0, 0), Some(ScreenEdge::Right));
            let batch = srv.drain();
            let mut seq = batch.last().map(|e| e.seq).unwrap_or(0);
            for ev in batch {
                if let Some(leave) = cli.apply(ev).unwrap() {
                    srv.note_seq(leave.seq);
                    srv.on_remote_leave();
                    let _ = cli.apply(leave).unwrap();
                }
            }
            if cli.focused() {
                seq = seq.saturating_add(1);
                let _ = cli.apply(InputEvent::move_to(seq, 40, 400)).unwrap();
                srv.note_seq(seq);
                seq = seq.saturating_add(1);
                if let Some(leave) = cli.apply(InputEvent::move_to(seq, 0, 400)).unwrap() {
                    srv.note_seq(leave.seq);
                    srv.on_remote_leave();
                    let _ = cli.apply(leave).unwrap();
                }
            }
            if srv.seat() != CursorSeat::Local {
                lost += 1;
            }
            srv.local_move_delta(100, 40, 0, 0);
        }
        assert_eq!(lost, 0);
        assert_eq!(cli.dropped(), 0);
        assert_eq!(srv.seat(), CursorSeat::Local);
    }

    #[test]
    fn peer_gone_returns_cursor_home() {
        let mut srv = InputServer::new(80, 40);
        assert_eq!(srv.local_move_delta(80, 5, 0, 0), Some(ScreenEdge::Right));
        assert_eq!(srv.seat(), CursorSeat::Remote);
        srv.on_peer_gone();
        assert_eq!(srv.seat(), CursorSeat::Local);
        assert!(srv.drain().is_empty());
        let mut cli = InputClient::new(MemorySink::default());
        let _ = cli.apply(InputEvent::enter(1)).unwrap();
        assert!(cli.focused());
        cli.on_peer_gone();
        assert!(!cli.focused());
    }

    #[test]
    fn replayed_seq_is_dropped_not_applied() {
        let mut cli = InputClient::new(MemorySink::default());
        let _ = cli.apply(InputEvent::enter(1)).unwrap();
        let _ = cli.apply(InputEvent::move_to(2, 10, 10)).unwrap();
        let before = cli.applied();
        let _ = cli.apply(InputEvent::move_to(2, 99, 99)).unwrap();
        assert_eq!(cli.applied(), before);
        assert_eq!(cli.sink().cursor, (10, 10));
        assert!(cli.dropped() > 0);
    }

    #[test]
    fn remote_motion_is_forwarded_after_edge() {
        let mut srv = InputServer::new(100, 50);
        assert_eq!(srv.local_move_delta(100, 25, 0, 0), Some(ScreenEdge::Right));
        assert_eq!(srv.seat(), CursorSeat::Remote);
        let cross = srv.drain();
        assert!(matches!(cross[0].kind, InputKind::Enter));

        // Delta-driven motion is the ONLY remote-seat input path in
        // production (hooks report deltas): the deleted absolute-coordinate
        // wrapper used to overwrite rem here, a semantic that never existed
        // on the wire path.
        srv.local_move_delta(80, 25, 5, 0);
        let out = srv.drain();
        assert_eq!(out.len(), 1, "remote motion must be forwarded");
        match out[0].kind {
            InputKind::Move { x, y } => {
                assert!(x > 0 && x <= SCREEN_UNITS);
                assert!(y > 0 && y <= SCREEN_UNITS);
            }
            other => panic!("expected Move, got {other:?}"),
        }
    }

    #[test]
    fn sink_trace_is_bounded() {
        let mut sink = MemorySink::default();
        for i in 0..(INPUT_SINK_TRACE_MAX as i32 + 50) {
            sink.move_abs(i, 0);
        }
        assert_eq!(sink.applied.len(), INPUT_SINK_TRACE_MAX);
        assert_eq!(sink.cursor, (INPUT_SINK_TRACE_MAX as i32 + 49, 0));
    }

    #[test]
    fn screen_edge_from_str_and_opposite() {
        assert_eq!(ScreenEdge::Left.opposite(), ScreenEdge::Right);
        assert_eq!(ScreenEdge::Right.opposite(), ScreenEdge::Left);
        assert_eq!(ScreenEdge::Top.opposite(), ScreenEdge::Bottom);
        assert_eq!(ScreenEdge::Bottom.opposite(), ScreenEdge::Top);

        assert_eq!("left".parse::<ScreenEdge>().unwrap(), ScreenEdge::Left);
        assert_eq!("right".parse::<ScreenEdge>().unwrap(), ScreenEdge::Right);
        assert_eq!("top".parse::<ScreenEdge>().unwrap(), ScreenEdge::Top);
        assert_eq!("up".parse::<ScreenEdge>().unwrap(), ScreenEdge::Top);
        assert_eq!("bottom".parse::<ScreenEdge>().unwrap(), ScreenEdge::Bottom);
        assert_eq!("down".parse::<ScreenEdge>().unwrap(), ScreenEdge::Bottom);
        assert!("invalid".parse::<ScreenEdge>().is_err());
    }

    #[test]
    fn four_directional_edge_crossing_and_round_trip() {
        let edges = [
            (ScreenEdge::Right, (201, 50), (0, 500), (500, 500), (0, 500)),
            (
                ScreenEdge::Left,
                (0, 50),
                (SCREEN_UNITS, 500),
                (500, 500),
                (SCREEN_UNITS, 500),
            ),
            (
                ScreenEdge::Top,
                (100, 0),
                (500, SCREEN_UNITS),
                (500, 500),
                (500, SCREEN_UNITS),
            ),
            (
                ScreenEdge::Bottom,
                (100, 101),
                (500, 0),
                (500, 500),
                (500, 0),
            ),
        ];

        for (edge, trigger_pos, expected_entry, interior_pos, leave_pos) in edges {
            let mut srv = InputServer::new(201, 101);
            srv.set_edge(edge);
            assert_eq!(srv.edge(), edge);

            // Cross into remote seat
            assert_eq!(
                srv.local_move_delta(trigger_pos.0, trigger_pos.1, 0, 0),
                Some(edge)
            );
            assert_eq!(srv.seat(), CursorSeat::Remote);
            let batch = srv.drain();
            assert_eq!(batch[0].kind, InputKind::Enter);
            match batch[1].kind {
                InputKind::Move { x, y } => {
                    assert_eq!(
                        (x, y),
                        expected_entry,
                        "edge {edge:?} expected entry {expected_entry:?}"
                    );
                }
                other => panic!("expected Move, got {other:?}"),
            }

            // Client applies Enter and initial Move
            let mut cli = InputClient::new(MemorySink::default()).with_autonomous_leave(true);
            for ev in batch {
                assert!(cli.apply(ev).unwrap().is_none());
            }
            assert!(cli.focused());

            // Move to interior
            let leave = cli
                .apply(InputEvent::move_to(10, interior_pos.0, interior_pos.1))
                .unwrap();
            assert!(leave.is_none());
            assert!(cli.focused());

            // Move back to entry edge -> triggers Leave!
            let leave = cli
                .apply(InputEvent::move_to(11, leave_pos.0, leave_pos.1))
                .unwrap();
            assert!(
                leave.is_some(),
                "edge {edge:?} should trigger Leave on return"
            );
            assert!(!cli.focused());

            // Remote reports leave -> Server returns to local seat
            srv.on_remote_leave();
            assert_eq!(srv.seat(), CursorSeat::Local);
        }
    }

    #[test]
    fn delta_bidirectional_return_and_force_local() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Top);

        // Move to top edge and enter Remote
        let edge = srv.local_move_delta(960, 0, 0, -10);
        assert_eq!(edge, Some(ScreenEdge::Top));
        assert_eq!(srv.seat(), CursorSeat::Remote);
        let events = srv.drain();
        assert_eq!(events[0].kind, InputKind::Enter);
        assert_eq!(
            events[1].kind,
            InputKind::Move {
                x: 500,
                y: SCREEN_UNITS
            }
        );

        // Roam upwards in remote
        let edge = srv.local_move_delta(960, 0, 0, -100);
        assert_eq!(edge, None);
        assert_eq!(srv.seat(), CursorSeat::Remote);
        let events = srv.drain();
        match events[0].kind {
            InputKind::Move { x: _, y } => assert!(y < SCREEN_UNITS),
            _ => panic!("expected Move"),
        }

        // Pull back down towards local screen (dy > 0 and rem_y reaches bottom)
        let edge = srv.local_move_delta(960, 0, 0, 300);
        assert_eq!(edge, Some(ScreenEdge::Top));
        assert_eq!(srv.seat(), CursorSeat::Local);
        let events = srv.drain();
        assert_eq!(events[0].kind, InputKind::Leave);
        assert_eq!(srv.local_pos().1, 16);

        // Test force_local
        srv.local_move_delta(960, 0, 0, -10);
        assert_eq!(srv.seat(), CursorSeat::Remote);
        srv.force_local();
        assert_eq!(srv.seat(), CursorSeat::Local);
        let events = srv.drain();
        assert_eq!(events[events.len() - 1].kind, InputKind::Leave);
    }

    #[test]
    fn cross_screen_position_mapping_is_proportional_both_directions() {
        let mut srv = InputServer::new(1000, 500);
        srv.set_edge(ScreenEdge::Top);

        // 1. Cross at local x=250 (25% of the local width): the entry Move
        // on the target must carry the proportionally mapped x, not a stale
        // remembered position.
        assert_eq!(srv.local_move_delta(250, 0, 0, -10), Some(ScreenEdge::Top));
        let events = srv.drain();
        assert_eq!(events[0].kind, InputKind::Enter);
        match events[1].kind {
            InputKind::Move { x, y } => assert_eq!((x, y), (250, SCREEN_UNITS)),
            other => panic!("expected Move, got {other:?}"),
        }

        // 2. Roam into the target interior (small steps; large dy would hit
        // the return guard because the entry parks at the shared edge).
        assert_eq!(srv.local_move_delta(250, 0, 0, -40), None);
        let _ = srv.drain();
        assert_eq!(srv.seat(), CursorSeat::Remote);

        // 3. Return home: the local landing x must be the inverse mapping of
        // the last remote x (Deskflow OSXScreen semantics), parked 16px below
        // the crossed edge — NOT the raw hook position (which sits wherever
        // the parked local cursor was left).
        assert_eq!(srv.local_move_delta(250, 0, 0, 50), Some(ScreenEdge::Top));
        let _ = srv.drain();
        assert_eq!(srv.seat(), CursorSeat::Local);
        let (lx, ly) = srv.local_pos();
        assert_eq!(ly, 16);
        // denorm(250) = 250 with round-to-nearest; the old truncating map
        // gave 249 and sheared every round trip.
        assert_eq!(lx, 250, "return x = denorm(rem_x)");

        // 4. Cross again at a different position: the entry must use the
        // FRESH exit position, never the previous session's remote pos.
        assert_eq!(srv.local_move_delta(750, 0, 0, -10), Some(ScreenEdge::Top));
        let events = srv.drain();
        match events[1].kind {
            // norm(750) = 751 with round-to-nearest (750*1000/999 = 750.75);
            // the truncating map gave 750 and sheared round trips.
            InputKind::Move { x, y } => assert_eq!((x, y), (751, SCREEN_UNITS)),
            other => panic!("expected Move, got {other:?}"),
        }

        // 5. Emergency force_local maps the landing position the same way.
        assert_eq!(srv.local_move_delta(750, 0, 0, -40), None);
        srv.force_local();
        assert_eq!(srv.seat(), CursorSeat::Local);
        let (lx, ly) = srv.local_pos();
        assert_eq!(ly, 16);
        // norm(750) = 751 (rounded), denorm(751) = 750 (rounded): the round
        // trip returns exactly to the origin — the truncating chain
        // (750 -> 749) walked a pixel per emergency return.
        assert_eq!(lx, 750, "force_local x = denorm(rem_x)");
    }

    #[test]
    fn mouse_drag_prevents_accidental_screen_switch() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Top);

        // Move to top edge and enter Remote
        let edge = srv.local_move_delta(960, 0, 0, -10);
        assert_eq!(edge, Some(ScreenEdge::Top));
        assert_eq!(srv.seat(), CursorSeat::Remote);

        // Roam upwards into remote interior
        srv.local_move_delta(960, 0, 0, -100);
        assert_eq!(srv.seat(), CursorSeat::Remote);

        // Hold left mouse button down (dragging)
        srv.note_button(BTN_LEFT, true);
        assert_eq!(srv.buttons_down(), BTN_LEFT);

        // Pull mouse downwards towards return edge while dragging
        // Normally this would trigger return to local, but during drag it must stay Remote!
        let edge = srv.local_move_delta(960, 0, 0, 300);
        assert_eq!(edge, None);
        assert_eq!(
            srv.seat(),
            CursorSeat::Remote,
            "seat must remain Remote while dragging"
        );

        // Release mouse button (drop / finish drag)
        srv.note_button(BTN_LEFT, false);
        assert_eq!(srv.buttons_down(), 0);

        // Now pulling mouse down triggers return to local
        let edge = srv.local_move_delta(960, 0, 0, 50);
        assert_eq!(edge, Some(ScreenEdge::Top));
        assert_eq!(srv.seat(), CursorSeat::Local);
    }

    #[test]
    fn input_client_flushes_held_keys_and_buttons_on_leave_and_peer_gone() {
        let mut cli = InputClient::new(MemorySink::default());
        let _ = cli.apply(InputEvent::enter(1)).unwrap();
        assert!(cli.focused());

        // Press Left Button and two keys (e.g. Shift 0xA0 and letter A 0x41)
        let _ = cli
            .apply(InputEvent {
                seq: 2,
                kind: InputKind::Button {
                    mask: BTN_LEFT,
                    down: true,
                },
            })
            .unwrap();
        let _ = cli.apply(InputEvent::key(3, 0xA0, true, 0)).unwrap();
        let _ = cli.apply(InputEvent::key(4, 0x41, true, 0)).unwrap();

        assert_eq!(cli.sink().buttons, BTN_LEFT);
        assert_eq!(cli.sink().keys_down.len(), 2);
        assert!(cli.sink().keys_down.contains(&0xA0));
        assert!(cli.sink().keys_down.contains(&0x41));

        // When Leave is applied, all held buttons and keys must be automatically released!
        let _ = cli.apply(InputEvent::leave(5)).unwrap();
        assert!(!cli.focused());
        assert_eq!(
            cli.sink().buttons,
            0,
            "mouse buttons must be flushed on leave"
        );
        assert!(
            cli.sink().keys_down.is_empty(),
            "held keys must be flushed on leave"
        );

        // Test on_peer_gone also flushes
        let _ = cli.apply(InputEvent::enter(6)).unwrap();
        let _ = cli.apply(InputEvent::key(7, 0x11, true, 0)).unwrap(); // Ctrl
        assert_eq!(cli.sink().keys_down.len(), 1);
        cli.on_peer_gone();
        assert!(!cli.focused());
        assert!(
            cli.sink().keys_down.is_empty(),
            "held keys must be flushed on peer gone"
        );
    }
}
