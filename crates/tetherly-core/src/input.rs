// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Desktop input protocol. Binary frames only — never JSON mouse/keyboard.
//! OS injection lives in tetherly-node; this module is OS-free.

use crate::error::CoreError;
use crate::frame::{InnerFrame, FLAG_MUST_UNDERSTAND, INNER_PAYLOAD_MAX};
use std::collections::VecDeque;

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

/// Edge a cursor can leave through. v1 uses a 1-D right/left pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenEdge {
    Left,
    Right,
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
    pub applied: Vec<InputKind>,
}

impl InputSink for MemorySink {
    fn move_abs(&mut self, x: i32, y: i32) {
        self.cursor = (x, y);
        self.applied.push(InputKind::Move { x, y });
    }

    fn button(&mut self, mask: u8, down: bool) {
        if down {
            self.buttons |= mask;
        } else {
            self.buttons &= !mask;
        }
        self.applied.push(InputKind::Button { mask, down });
    }

    fn wheel(&mut self, dx: i16, dy: i16) {
        self.applied.push(InputKind::Wheel { dx, dy });
    }

    fn key(&mut self, code: u16, down: bool, mods: u8) {
        if down {
            if !self.keys_down.contains(&code) {
                self.keys_down.push(code);
            }
        } else {
            self.keys_down.retain(|c| *c != code);
        }
        self.applied.push(InputKind::Key { code, down, mods });
    }
}

/// Server-side capture: local cursor, detect edge leave, encode outbound events.
pub struct InputServer {
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    seat: CursorSeat,
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
            seat: CursorSeat::Local,
            next_seq: 1,
            pending: VecDeque::with_capacity(INPUT_BATCH_MAX),
        }
    }

    pub fn seat(&self) -> CursorSeat {
        self.seat
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

    fn push(&mut self, kind: InputKind) {
        if self.pending.len() >= INPUT_BATCH_MAX {
            self.pending.pop_front();
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.pending.push_back(InputEvent { seq, kind });
    }

    /// Pixel-space local motion. Crossing the right edge parks the cursor
    /// locally and emits Enter + Move in screen units for the client.
    pub fn local_move(&mut self, x: i32, y: i32) -> Option<ScreenEdge> {
        let y = y.clamp(0, self.height.saturating_sub(1));
        if self.seat == CursorSeat::Remote {
            self.x = x.clamp(0, self.width.saturating_sub(1));
            self.y = y;
            return None;
        }
        if x >= self.width {
            self.x = self.width.saturating_sub(1);
            self.y = y;
            self.seat = CursorSeat::Remote;
            self.push(InputKind::Enter);
            self.push(InputKind::Move {
                x: 0,
                y: self.norm_y(),
            });
            return Some(ScreenEdge::Right);
        }
        if x < 0 {
            self.x = 0;
            self.y = y;
            return None;
        }
        self.x = x;
        self.y = y;
        None
    }

    pub fn local_button(&mut self, mask: u8, down: bool) {
        if self.seat == CursorSeat::Remote {
            self.push(InputKind::Button { mask, down });
        }
    }

    pub fn local_wheel(&mut self, dx: i16, dy: i16) {
        if self.seat == CursorSeat::Remote {
            self.push(InputKind::Wheel { dx, dy });
        }
    }

    pub fn local_key(&mut self, code: u16, down: bool, mods: u8) {
        if self.seat == CursorSeat::Remote {
            self.push(InputKind::Key { code, down, mods });
        }
    }

    /// Remote reports the cursor left back through its left edge.
    pub fn on_remote_leave(&mut self) {
        self.seat = CursorSeat::Local;
        self.x = self.width.saturating_sub(1);
    }

    /// Peer input session died: keyboard/mouse return here (M3.2).
    pub fn on_peer_gone(&mut self) {
        self.pending.clear();
        self.seat = CursorSeat::Local;
        self.x = self.width / 2;
        self.y = self.height / 2;
    }

    pub fn drain(&mut self) -> Vec<InputEvent> {
        self.pending.drain(..).collect()
    }

    fn norm_y(&self) -> i32 {
        if self.height <= 1 {
            return 0;
        }
        ((self.y as i64) * SCREEN_UNITS as i64 / (self.height as i64 - 1)) as i32
    }
}

/// Client-side apply: decode, drop old seq, inject, leave when x==0.
pub struct InputClient<S: InputSink> {
    sink: S,
    last_seq: u64,
    focused: bool,
    /// True after a focused move with x > 0. Entry lands at x=0 and must not leave.
    interior: bool,
    dropped: u64,
    applied: u64,
}

impl<S: InputSink> InputClient<S> {
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            last_seq: 0,
            focused: false,
            interior: false,
            dropped: 0,
            applied: 0,
        }
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
                self.focused = true;
                self.interior = false;
                self.applied = self.applied.saturating_add(1);
                Ok(None)
            }
            InputKind::Leave => {
                self.focused = false;
                self.interior = false;
                self.applied = self.applied.saturating_add(1);
                Ok(None)
            }
            InputKind::Move { x, y } => {
                if !self.focused {
                    self.dropped = self.dropped.saturating_add(1);
                    return Ok(None);
                }
                let leaving = x == 0 && self.interior;
                self.sink.move_abs(x, y);
                self.applied = self.applied.saturating_add(1);
                if x > 0 {
                    self.interior = true;
                }
                if leaving {
                    self.focused = false;
                    self.interior = false;
                    return Ok(Some(InputEvent::leave(ev.seq.saturating_add(1))));
                }
                Ok(None)
            }
            InputKind::Button { mask, down } => {
                if self.focused {
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
        assert_eq!(srv.local_move(99, 10), None);
        assert_eq!(srv.seat(), CursorSeat::Local);
        assert_eq!(srv.local_move(100, 10), Some(ScreenEdge::Right));
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
        let mut cli = InputClient::new(MemorySink::default());
        let mut lost = 0u32;
        for _ in 0..100 {
            assert_eq!(srv.local_move(200, 40), Some(ScreenEdge::Right));
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
            srv.local_move(100, 40);
        }
        assert_eq!(lost, 0);
        assert_eq!(cli.dropped(), 0);
        assert_eq!(srv.seat(), CursorSeat::Local);
    }

    #[test]
    fn peer_gone_returns_cursor_home() {
        let mut srv = InputServer::new(80, 40);
        assert_eq!(srv.local_move(80, 5), Some(ScreenEdge::Right));
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
}
