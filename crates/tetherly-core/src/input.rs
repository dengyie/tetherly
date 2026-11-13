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

/// Evidence of the last sustained-push return to the local seat (v0.2.38):
/// lets a production bounce be attributed from the log alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnDiag {
    /// Milliseconds between the crossing into Remote and the return.
    pub elapsed_ms: u64,
    /// Deepest interior travel (0..=1000 units) away from the shared edge
    /// during the stint.
    pub peak_depth: i32,
    /// Accumulated push (local px) beyond the shared edge that returned.
    pub push: i32,
    /// Threshold (local px) in force at the moment of the return.
    pub required_push: i32,
    /// v0.2.39 stroke balance: raw local px moved away from the shared
    /// edge (into the remote screen) during the stint.
    pub away_px: i32,
    /// Raw local px moved toward the shared edge during the stint.
    pub toward_px: i32,
    /// Motion events the stint consumed (excluding the crossing event).
    pub events: u32,
    /// Largest single-event toward-edge delta (raw local px).
    pub max_toward: i32,
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
    /// Speed-aware Top/Bottom redundancy band: start of the current quiet
    /// period inside the band (None while the cursor moves into the edge
    /// faster than `fast_push_delta_px` per event, or sits outside the
    /// band). A slow glide must stay quiet for `slow_dwell_ms` before the
    /// seat may flip; a fast push crosses immediately.
    tb_band_quiet_since: Option<std::time::Instant>,
    /// Active band settings (defaults from EdgeCrossingConfig; a 0 band
    /// disables the gate entirely, restoring the legacy instant cross).
    tb_band_px: u32,
    tb_slow_dwell_ms: u32,
    tb_fast_push_delta_px: u32,
    /// Fractional remainders of the remote-seat per-event delta
    /// normalization (units of `delta * SCREEN_UNITS / dimension`).
    /// Without these, integer truncation makes the remote displacement
    /// depend on how the same physical gesture is chunked into events:
    /// small deltas were forced to ±1 (over-moving) while larger ones
    /// repeatedly dropped the fraction (under-moving). Kept in i64 so
    /// `delta * SCREEN_UNITS` never overflows i32 for any real delta.
    rem_frac_x: i64,
    rem_frac_y: i64,
    /// Sustained-push switch-back accumulator (raw px contributed toward the
    /// shared edge while the remote cursor sits saturated on it). Replaces
    /// the old per-event `|delta| > 4` flick gate, which at real report
    /// rates (125-1000 Hz, 1-3 px per event) demanded a >2000 px/s jerk and
    /// stranded both nodes in Remote seat (production 2026-11-11).
    far_push_accum: i32,
    /// Last event that contributed to `far_push_accum`; pushes older than
    /// [`Self::FAR_PUSH_WINDOW`] no longer count.
    far_push_last: Option<std::time::Instant>,
    /// Entry return grace (v0.2.31): for this long after the seat crossed
    /// INTO Remote, the sustained-push return is suppressed and the push
    /// accumulator stays empty. Production 2026-11-13 22:28: with the
    /// entry placement parked ON the mac's top edge (menu bar zone) and
    /// only 20 units of interior hysteresis, the hand's residual push
    /// after the crossing accumulated 12 px within 600 ms and bounced the
    /// seat straight back 0.9-3.3 s after entry. Zero (the core default)
    /// keeps the legacy instant return; the node configures it from
    /// `EdgeCrossingConfig::entry_return_grace_ms`.
    entry_return_grace: std::time::Duration,
    /// Instant the seat last crossed into Remote (anchor of the grace).
    remote_entered_at: Option<std::time::Instant>,
    /// Peer screen size (px/pt) learnt from the tunnel accept (v0.2.32).
    /// When known, remote-seat deltas normalize by the REMOTE dimension so
    /// one local px moves one remote px/pt on both axes; `None` falls back
    /// to the local dimension (legacy: a 1920x1080 controller moved a
    /// 1470x956 mac cursor 0.77 / 0.89 pt per px, slow and anisotropic).
    remote_screen: Option<(i32, i32)>,
    /// Remote delta multiplier in percent (100 = 1:1 against the peer
    /// screen); clamped to `REMOTE_SPEED_PERCENT_RANGE`.
    remote_speed_percent: u32,
    /// Deepest interior travel of the current Remote stint (units).
    remote_peak_depth: i32,
    /// Stroke balance of the current Remote stint (see [`ReturnDiag`]).
    stint_away_px: i32,
    stint_toward_px: i32,
    stint_events: u32,
    stint_max_toward: i32,
    /// Evidence of the last sustained-push return (see [`ReturnDiag`]).
    last_return_diag: Option<ReturnDiag>,
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
            tb_band_quiet_since: None,
            tb_band_px: 0,
            tb_slow_dwell_ms: 120,
            tb_fast_push_delta_px: 6,
            rem_frac_x: 0,
            rem_frac_y: 0,
            far_push_accum: 0,
            far_push_last: None,
            entry_return_grace: std::time::Duration::ZERO,
            remote_entered_at: None,
            remote_screen: None,
            remote_speed_percent: 100,
            remote_peak_depth: 0,
            stint_away_px: 0,
            stint_toward_px: 0,
            stint_events: 0,
            stint_max_toward: 0,
            last_return_diag: None,
        }
    }

    /// Sustained push (raw px) at the shared edge required to switch back to
    /// the local seat. Mirrors the entry-side resistance so both directions
    /// feel symmetric.
    pub const FAR_PUSH_RETURN_PX: i32 = 12;
    /// Entry-time multiplier of `FAR_PUSH_RETURN_PX`: the return push
    /// required right after the crossing, decaying linearly to the base
    /// threshold over the entry return grace (v0.2.32).
    const ENTRY_RETURN_PUSH_MULTIPLIER: i32 = 10;
    /// A return push older than this no longer counts toward the threshold;
    /// a fresh deliberate push must re-accumulate.
    const FAR_PUSH_WINDOW: std::time::Duration = std::time::Duration::from_millis(600);
    /// Accepted `remote_speed_percent` range.
    pub const REMOTE_SPEED_PERCENT_RANGE: std::ops::RangeInclusive<u32> = 25..=400;

    /// Return push (raw px) required `elapsed` after the crossing into
    /// Remote with an entry return grace of `grace`: decays linearly from
    /// `ENTRY_RETURN_PUSH_MULTIPLIER x FAR_PUSH_RETURN_PX` at the crossing
    /// to `FAR_PUSH_RETURN_PX` when the grace ends; the base threshold
    /// applies with no grace or after it. Replaces the v0.2.31 hard cut,
    /// which blocked every return inside the grace and then let the hand's
    /// residual momentum bounce the seat the instant it expired.
    pub fn required_return_push_px(
        elapsed: std::time::Duration,
        grace: std::time::Duration,
    ) -> i32 {
        let base = Self::FAR_PUSH_RETURN_PX;
        if grace.is_zero() || elapsed >= grace {
            return base;
        }
        let extra = base * (Self::ENTRY_RETURN_PUSH_MULTIPLIER - 1);
        let decayed =
            (extra as i128 * elapsed.as_nanos() as i128 / grace.as_nanos() as i128) as i32;
        base + extra - decayed
    }

    /// Set (or clear with `None`) the peer screen size used to normalize
    /// remote-seat deltas (see the field doc). Changing the divisor resets
    /// the fraction carry, which is denominated in the old geometry.
    pub fn set_remote_screen(&mut self, screen: Option<(i32, i32)>) {
        let screen = screen.map(|(w, h)| (w.max(1), h.max(1)));
        if self.remote_screen != screen {
            self.remote_screen = screen;
            self.rem_frac_x = 0;
            self.rem_frac_y = 0;
        }
    }

    /// Remote delta multiplier in percent (100 = 1:1 against the peer
    /// screen); out-of-range values clamp, so the remote cursor can never
    /// freeze.
    pub fn set_remote_speed_percent(&mut self, percent: u32) {
        let percent = percent.clamp(
            *Self::REMOTE_SPEED_PERCENT_RANGE.start(),
            *Self::REMOTE_SPEED_PERCENT_RANGE.end(),
        );
        if self.remote_speed_percent != percent {
            self.remote_speed_percent = percent;
            self.rem_frac_x = 0;
            self.rem_frac_y = 0;
        }
    }

    /// Peer screen size currently used for remote-seat normalization.
    pub fn remote_screen(&self) -> Option<(i32, i32)> {
        self.remote_screen
    }

    /// Effective (clamped) remote speed percent.
    pub fn remote_speed_percent(&self) -> u32 {
        self.remote_speed_percent
    }

    /// Normalization divisors for remote-seat deltas: `(dim * 100)` of the
    /// peer screen when known, else of the local screen (legacy).
    fn remote_delta_divisors(&self) -> (i64, i64) {
        let (w, h) = self
            .remote_screen
            .unwrap_or((self.width.max(1), self.height.max(1)));
        (w as i64 * 100, h as i64 * 100)
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

    /// Configure the entry return grace (see the field doc). `ZERO`
    /// disables it (legacy instant return).
    pub fn set_entry_return_grace(&mut self, grace: std::time::Duration) {
        self.entry_return_grace = grace;
    }

    /// Return push (raw px) required at `now`, given the entry return
    /// ramp since the last crossing into Remote (base threshold when no
    /// crossing is recorded).
    /// Evidence of the last sustained-push return to the local seat.
    fn reset_stint_balance(&mut self) {
        self.stint_away_px = 0;
        self.stint_toward_px = 0;
        self.stint_events = 0;
        self.stint_max_toward = 0;
    }

    pub fn last_return_diag(&self) -> Option<ReturnDiag> {
        self.last_return_diag
    }

    fn current_return_push_px(&self, now: std::time::Instant) -> i32 {
        self.remote_entered_at
            .map(|t| Self::required_return_push_px(now.duration_since(t), self.entry_return_grace))
            .unwrap_or(Self::FAR_PUSH_RETURN_PX)
    }

    /// Configure the speed-aware Top/Bottom redundancy band (see
    /// [`EdgeCrossingConfig`]). `band_px == 0` disables the gate (legacy
    /// instant cross on every edge).
    pub fn set_tb_redundancy_band(
        &mut self,
        band_px: u32,
        slow_dwell_ms: u32,
        fast_push_delta_px: u32,
    ) {
        let slow_dwell_ms = slow_dwell_ms.max(1);
        let fast_push_delta_px = fast_push_delta_px.max(1);
        // Idempotent when unchanged: callers mirror persisted config on every
        // mouse event, and resetting the dwell timestamp here would make the
        // slow-glide gate unreachable (the dwell could never accumulate).
        if self.tb_band_px == band_px
            && self.tb_slow_dwell_ms == slow_dwell_ms
            && self.tb_fast_push_delta_px == fast_push_delta_px
        {
            return;
        }
        self.tb_band_px = band_px;
        self.tb_slow_dwell_ms = slow_dwell_ms;
        self.tb_fast_push_delta_px = fast_push_delta_px;
        self.tb_band_quiet_since = None;
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

    /// Drop a STALE Remote seat before establishing a new controller session.
    ///
    /// A takeover demotes this node while its engine still sits in Remote
    /// from the previous controller stint. If a later re-attach finds the
    /// seat Remote, `resume_remote` replays the old rem coordinates onto the
    /// target and — worse — the fresh edge-crossing arm never fires again
    /// (it requires seat Local), so the cursor keeps drifting from the stale
    /// spot instead of entering proportionally at the shared edge.
    /// Production symptom: after auto-reverse, the cursor emerged mid-screen
    /// on the target instead of at the mapped entry.
    ///
    /// Unlike `force_local`, nothing is pushed and nothing warps: the
    /// hook-observed x/y are kept so the next physical edge push crosses
    /// fresh with a proportional rem.
    pub fn clear_stale_remote(&mut self) {
        if self.seat == CursorSeat::Remote {
            self.seat = CursorSeat::Local;
            self.remote_interior = false;
            self.buttons_down = 0;
            self.rem_frac_x = 0;
            self.rem_frac_y = 0;
            self.far_push_accum = 0;
            self.far_push_last = None;
            self.pending.clear();
        }
    }

    /// Forcefully disarm remote roaming and restore the cursor seat to Local.
    pub fn force_local(&mut self) {
        self.buttons_down = 0;
        if self.seat == CursorSeat::Remote {
            self.seat = CursorSeat::Local;
            self.remote_interior = false;
            self.rem_frac_x = 0;
            self.rem_frac_y = 0;
            self.far_push_accum = 0;
            self.far_push_last = None;
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
                    // v0.2.26: dock OUTSIDE the Top/Bottom redundancy band.
                    // The old y=16 sat inside the 24px band, where the
                    // band gate's slow-dwell path (quiet >= slow_dwell_ms,
                    // trivially true when nobody touches the mouse) turned
                    // any residual dy<=0 event — e.g. a warp echo that
                    // missed the echo gate — into an immediate re-crossing
                    // (production: ghost-controller bounce loop, rescue ->
                    // pre-arm -> crossing every ~13s while the target's
                    // physical mouse was silently swallowed).
                    self.y = self.band_safe_dock_y();
                    self.x = self.denorm_x(self.rem_x);
                }
                ScreenEdge::Bottom => {
                    self.y = self
                        .height
                        .saturating_sub(1)
                        .saturating_sub(self.band_safe_dock_y());
                    self.x = self.denorm_x(self.rem_x);
                }
            }
        }
    }

    /// Vertical dock offset that keeps a Top/Bottom park clear of the
    /// redundancy band: `band_px + 24` (24px of margin above the band so
    /// an OS clamp or a stray 1-2px event cannot fall back inside).
    fn band_safe_dock_y(&self) -> i32 {
        let band = self.tb_band_px as i32;
        (band.max(24) + 24).min(self.height.saturating_sub(1))
    }

    /// Change the logical screen without resetting `seq`.
    pub fn resize(&mut self, width: i32, height: i32) {
        self.width = width.max(1);
        self.height = height.max(1);
        // The accumulator is denominated in `delta * SCREEN_UNITS /
        // dimension`; keeping it across a dimension change would apply the
        // old geometry's fraction against new normalization.
        self.rem_frac_x = 0;
        self.rem_frac_y = 0;
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

    /// ADR-138 remote session: promote to Remote seat and emit a single
    /// permanent `[Enter, Move{center}]` pair. Session mode has no boundary
    /// semantics — the controller's session-window mapping owns every
    /// subsequent Move coordinate. Unconditional (unlike `resume_remote`):
    /// the caller has already validated the transition.
    pub fn force_remote_center(&mut self) {
        self.seat = CursorSeat::Remote;
        self.remote_entered_at = Some(std::time::Instant::now());
        self.remote_interior = false;
        self.buttons_down = 0;
        self.pending.clear();
        self.push(InputKind::Enter);
        self.push(InputKind::Move {
            x: SCREEN_UNITS / 2,
            y: SCREEN_UNITS / 2,
        });
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

    /// Speed-aware gate for Top/Bottom edge crossings (menu bar / task bar
    /// redundancy). Returns true when the crossing may proceed:
    ///
    /// - a fast push (this event's into-edge |delta| >= fast_push_delta_px)
    ///   crosses immediately;
    /// - a slow glide must be inside the redundancy band (`in_band`) and
    ///   have stayed quiet (no further into-edge motion) for
    ///   `slow_dwell_ms` before the seat may flip;
    /// - a slow glide outside the band never crosses (and drops any dwell).
    ///
    /// Always true when the band is disabled (`band_px == 0`).
    ///
    /// Mutates quiet-period bookkeeping, hence `&mut self`.
    fn tb_band_gate(&mut self, in_band: bool, into_delta: i32) -> bool {
        if self.tb_band_px == 0 {
            return true;
        }
        if into_delta.abs() >= self.tb_fast_push_delta_px as i32 {
            // Deliberate push: cross now and forget any dwell.
            self.tb_band_quiet_since = None;
            return true;
        }
        if !in_band {
            // Outside the redundancy band (and not fast): keep waiting.
            self.tb_band_quiet_since = None;
            return false;
        }
        let now = std::time::Instant::now();
        match self.tb_band_quiet_since {
            Some(started) => {
                now.duration_since(started).as_millis() as u32 >= self.tb_slow_dwell_ms
            }
            None => {
                self.tb_band_quiet_since = Some(now);
                false
            }
        }
    }

    pub fn local_move_delta(&mut self, x: i32, y: i32, dx: i32, dy: i32) -> Option<ScreenEdge> {
        let y_clamped = y.clamp(0, self.height.saturating_sub(1));
        let x_clamped = x.clamp(0, self.width.saturating_sub(1));
        if self.seat == CursorSeat::Remote {
            let crossed_edge = self.last_crossed_edge;
            // Fractional-accumulator normalization: keep the exact
            // `delta * SCREEN_UNITS * speed% / (dimension * 100)` remainder
            // between events so the applied remote displacement depends on
            // the total physical motion, not on how it was chunked into
            // events. Applied units come from the accumulator; truncation
            // error is carried, never dropped and never invented (a zero
            // delta still applies zero).
            //
            // v0.2.32: the dimension is the PEER screen's when known (the
            // tunnel accept carries it), so 1 local px moves 1 remote px at
            // 100 %. Dividing by the local size (legacy) made a 1920x1080
            // controller move only 0.77 pt per px on a 1470x956 Mac and
            // made X and Y speeds differ.
            let (div_x, div_y) = self.remote_delta_divisors();
            let pct = self.remote_speed_percent as i64;
            let (delta_x, new_frac_x) = if dx != 0 {
                let scaled = dx as i64 * SCREEN_UNITS as i64 * pct + self.rem_frac_x;
                let d = (scaled / div_x) as i32;
                let frac = scaled.rem_euclid(div_x);
                // Sign-correct: rem_euclid is non-negative; keep the fraction
                // alongside the direction it belongs to.
                let frac = if scaled < 0 { frac - div_x } else { frac };
                (d, frac)
            } else {
                (0, self.rem_frac_x)
            };
            let (delta_y, new_frac_y) = if dy != 0 {
                let scaled = dy as i64 * SCREEN_UNITS as i64 * pct + self.rem_frac_y;
                let d = (scaled / div_y) as i32;
                let frac = scaled.rem_euclid(div_y);
                let frac = if scaled < 0 { frac - div_y } else { frac };
                (d, frac)
            } else {
                (0, self.rem_frac_y)
            };
            self.rem_frac_x = new_frac_x;
            self.rem_frac_y = new_frac_y;

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

            // Do not switch screen back to local while holding down a mouse
            // button (prevent drag interruption).
            //
            // Deskflow-style sustained-push switch-back: the shared edge (the
            // entry edge on the target) must receive an ACCUMULATED push of
            // FAR_PUSH_RETURN_PX raw px within FAR_PUSH_WINDOW — not a
            // per-event flick. The old per-event `|delta| > 4` gate demanded
            // a >2000 px/s jerk at 500 Hz report rates, so a deliberate slow
            // push never returned and both nodes were stranded in Remote
            // seat (production 2026-11-11). Fast flicks still return
            // instantly: one event contributes its whole delta.
            let (push_toward_edge, at_shared_edge, depth_before, depth_after) = match crossed_edge {
                ScreenEdge::Top => (
                    dy,
                    next_rem_y >= SCREEN_UNITS,
                    SCREEN_UNITS - self.rem_y,
                    SCREEN_UNITS - next_rem_y,
                ),
                ScreenEdge::Bottom => (-dy, next_rem_y <= 0, self.rem_y, next_rem_y),
                ScreenEdge::Left => (
                    dx,
                    next_rem_x >= SCREEN_UNITS,
                    SCREEN_UNITS - self.rem_x,
                    SCREEN_UNITS - next_rem_x,
                ),
                ScreenEdge::Right => (-dx, next_rem_x <= 0, self.rem_x, next_rem_x),
            };
            self.remote_peak_depth = self.remote_peak_depth.max(depth_after);
            self.stint_events = self.stint_events.saturating_add(1);
            if push_toward_edge > 0 {
                self.stint_toward_px = self.stint_toward_px.saturating_add(push_toward_edge);
                self.stint_max_toward = self.stint_max_toward.max(push_toward_edge);
            } else {
                self.stint_away_px = self.stint_away_px.saturating_add(-push_toward_edge);
            }
            // v0.2.38: only the motion that goes PAST the shared edge pushes
            // against it. The travel back to the edge is ordinary roaming —
            // counting the whole event delta let a shallow entry's
            // lift-and-reset stroke clear the return threshold (production
            // 2026-11-13 19:27: five bounces 0.34-0.63 s after entry).
            let push_beyond_edge = if at_shared_edge && push_toward_edge > 0 {
                let div = match crossed_edge {
                    ScreenEdge::Top | ScreenEdge::Bottom => div_y,
                    ScreenEdge::Left | ScreenEdge::Right => div_x,
                };
                let px_to_edge =
                    (depth_before.max(0) as i64 * div / (SCREEN_UNITS as i64 * pct.max(1))) as i32;
                (push_toward_edge - px_to_edge).max(0)
            } else {
                0
            };
            let now = std::time::Instant::now();
            let window_expired = self
                .far_push_last
                .map(|t| now.duration_since(t) > Self::FAR_PUSH_WINDOW)
                .unwrap_or(true);
            if window_expired || !at_shared_edge || push_toward_edge < 0 {
                self.far_push_accum = 0;
            }
            if push_beyond_edge > 0 && self.buttons_down == 0 {
                self.far_push_accum = self.far_push_accum.saturating_add(push_beyond_edge);
                self.far_push_last = Some(now);
            }
            // v0.2.32 entry return ramp (replaces the v0.2.31 hard grace):
            // right after the crossing the hand's residual momentum lands
            // small pushes on the shared edge, so the required push starts
            // at 10x the base (v0.2.38; 4x in v0.2.32 let a shallow entry's
            // reset stroke bounce the seat) and decays to it over the grace.
            // A deliberate
            // flick clears the ramp at any time; a hard cut instead bounced
            // the seat the instant the grace expired (production
            // 2026-11-13: five bounces in one 3-minute session).
            let required_push = self.current_return_push_px(now);
            let returning_local = self.remote_interior
                && (self.buttons_down == 0)
                && self.far_push_accum >= required_push;

            if returning_local {
                self.last_return_diag = Some(ReturnDiag {
                    elapsed_ms: self
                        .remote_entered_at
                        .map(|t| now.duration_since(t).as_millis() as u64)
                        .unwrap_or(0),
                    peak_depth: self.remote_peak_depth,
                    push: self.far_push_accum,
                    required_push,
                    away_px: self.stint_away_px,
                    toward_px: self.stint_toward_px,
                    events: self.stint_events,
                    max_toward: self.stint_max_toward,
                });
                self.seat = CursorSeat::Local;
                self.remote_interior = false;
                self.far_push_accum = 0;
                self.far_push_last = None;
                self.pending.clear();
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
                        // v0.2.26: same band-safe dock as force_local — the
                        // old y=16 parked inside the redundancy band and
                        // re-crossed on the next stray dy<=0 event.
                        self.y = self.band_safe_dock_y();
                    }
                    ScreenEdge::Bottom => {
                        self.x = self.denorm_x(self.rem_x);
                        self.y = self
                            .height
                            .saturating_sub(1)
                            .saturating_sub(self.band_safe_dock_y());
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

            // Wire economy (v0.2.30): only a CHANGED normalized position is
            // information for the target — the previous Move already parked
            // its cursor there. The OS-clamped zero-delta tail (Windows), the
            // post-warp settle events whose deltas the macOS tap zeroes (4
            // per park warp) and sub-unit deltas still carrying their
            // fraction all used to push one redundant Move per event (the
            // `push: lone edge-clamp move` quadruplets in production logs).
            // Bookkeeping (fraction carry, local x/y) still updates.
            let rem_changed = next_rem_x != self.rem_x || next_rem_y != self.rem_y;
            self.rem_x = next_rem_x;
            self.rem_y = next_rem_y;
            self.x = x_clamped;
            self.y = y_clamped;
            if rem_changed {
                self.push(InputKind::Move {
                    x: self.rem_x,
                    y: self.rem_y,
                });
            }
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
                    self.remote_entered_at = Some(std::time::Instant::now());
                    self.remote_peak_depth = 0;
                    self.reset_stint_balance();
                    self.remote_interior = false;
                    self.last_crossed_edge = ScreenEdge::Right;
                    // Root-fix E: drop any stale frame lingering in the
                    // pending queue (e.g. a remote-branch Move that raced a
                    // teardown) — the crossing batch must be exactly
                    // [Enter, Move{rem}]; a stale Move after the Enter
                    // re-places the cursor at an old position (production:
                    // cursor snapped to screen center).
                    self.pending.clear();
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
                    self.remote_entered_at = Some(std::time::Instant::now());
                    self.remote_peak_depth = 0;
                    self.reset_stint_balance();
                    self.remote_interior = false;
                    self.last_crossed_edge = ScreenEdge::Left;
                    // Root-fix E: drop any stale frame lingering in the
                    // pending queue (e.g. a remote-branch Move that raced a
                    // teardown) — the crossing batch must be exactly
                    // [Enter, Move{rem}]; a stale Move after the Enter
                    // re-places the cursor at an old position (production:
                    // cursor snapped to screen center).
                    self.pending.clear();
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
                // Speed-aware redundancy: EVERY into-edge motion (dy <= 0,
                // including the OS-clamped y==0 tail with dy==0) goes through
                // the band gate — a fast push (|dy| >= fast_push_delta_px)
                // crosses immediately, a slow glide must settle inside the
                // band for slow_dwell_ms first. Outgoing motion (dy > 0) and
                // interior positions never cross. Without gating the clamped
                // y==0 tail, a slow trackpad glide toward the macOS menu bar
                // teleports the seat across (production complaint).
                let into_edge = dy <= 0 && y <= 8;
                let may_cross = if into_edge {
                    self.tb_band_gate(y <= self.tb_band_px as i32, dy)
                } else {
                    false
                };
                if may_cross {
                    self.x = x_clamped;
                    self.y = 0;
                    self.rem_x = self.norm_x();
                    self.rem_y = SCREEN_UNITS;
                    self.seat = CursorSeat::Remote;
                    self.remote_entered_at = Some(std::time::Instant::now());
                    self.remote_peak_depth = 0;
                    self.reset_stint_balance();
                    self.remote_interior = false;
                    self.last_crossed_edge = ScreenEdge::Top;
                    // Root-fix E: drop any stale frame lingering in the
                    // pending queue (e.g. a remote-branch Move that raced a
                    // teardown) — the crossing batch must be exactly
                    // [Enter, Move{rem}]; a stale Move after the Enter
                    // re-places the cursor at an old position (production:
                    // cursor snapped to screen center).
                    self.pending.clear();
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
                // Win-side twin of the Top redundancy band: EVERY into-edge
                // motion (dy >= 0 near the bottom, including the OS-clamped
                // y==height-1 tail with dy==0) goes through the band gate —
                // a fast push crosses immediately, a slow glide toward the
                // Windows task bar must settle inside the band first.
                let into_edge = dy >= 0 && y >= self.height.saturating_sub(8);
                let may_cross = if into_edge {
                    let depth = self.height.saturating_sub(y);
                    self.tb_band_gate(depth <= self.tb_band_px as i32, dy)
                } else {
                    false
                };
                if may_cross {
                    self.x = x_clamped;
                    self.y = self.height.saturating_sub(1);
                    self.rem_x = self.norm_x();
                    self.rem_y = 0;
                    self.seat = CursorSeat::Remote;
                    self.remote_entered_at = Some(std::time::Instant::now());
                    self.remote_peak_depth = 0;
                    self.reset_stint_balance();
                    self.remote_interior = false;
                    self.last_crossed_edge = ScreenEdge::Bottom;
                    // Root-fix E: drop any stale frame lingering in the
                    // pending queue (e.g. a remote-branch Move that raced a
                    // teardown) — the crossing batch must be exactly
                    // [Enter, Move{rem}]; a stale Move after the Enter
                    // re-places the cursor at an old position (production:
                    // cursor snapped to screen center).
                    self.pending.clear();
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
        self.rem_frac_x = 0;
        self.rem_frac_y = 0;
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
        self.rem_frac_x = 0;
        self.rem_frac_y = 0;
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

    /// Same total physical displacement, different event chunking, must land
    /// the remote cursor at (approximately) the same place when no boundary
    /// is involved. Per-event integer normalization with no fractional
    /// accumulator violates this: small per-event deltas are promoted to a
    /// forced ±1, so many tiny events outrun one coalesced event carrying
    /// the same total displacement (production: Windows→Mac roaming speed
    /// depends on the mouse report rate and queue coalescing).
    #[test]
    fn remote_displacement_is_chunking_invariant_interior() {
        // width 1920: dx=1 normalizes to 1000/1920 = 0 -> forced signum(+1).
        let mut tiny = InputServer::new(1920, 1080);
        assert_eq!(
            tiny.local_move_delta(1920, 540, 1, 0),
            Some(ScreenEdge::Right)
        );
        let _ = tiny.drain(); // [Enter, Move{entry}]
        for _ in 0..192 {
            tiny.local_move_delta(1919, 540, 1, 0);
            let _ = tiny.drain();
        }
        let (tx, _) = tiny.remote_pos();

        let mut one = InputServer::new(1920, 1080);
        assert_eq!(
            one.local_move_delta(1920, 540, 1, 0),
            Some(ScreenEdge::Right)
        );
        let _ = one.drain();
        one.local_move_delta(1919, 540, 192, 0);
        let _ = one.drain();
        let (ox, _) = one.remote_pos();

        assert!(
            (tx - ox).abs() <= 2,
            "same total displacement must land equally: 192×dx=1 -> rem_x={tx}, 1×dx=192 -> rem_x={ox}"
        );
    }

    /// Windows twin: the hook derives deltas from consecutive cursor
    /// positions, and on entering Remote the cursor is parked AT the shared
    /// screen edge. If the OS clamps the cursor to the boundary while the
    /// user keeps pushing, every subsequent event reports dy=0 and the
    /// remote cursor freezes at the entry edge. The CORE contract pinned
    /// here: a zero-delta tail must never invent motion nor strand the
    /// seat, and once the hook recenter path supplies real inward deltas
    /// again, motion must resume from the held position.
    #[test]
    fn remote_seat_survives_zero_delta_tail_and_resumes_on_motion() {
        // 1920x1080, cross Bottom: remote entry (500, 0). band_px=0 keeps
        // the gate pass-through (set_tb_redundancy_band idempotent mirror);
        // into_edge still requires y within the last 8 rows, so drive the
        // cursor to the bottom edge with a fast dy first.
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Bottom);
        srv.set_tb_redundancy_band(0, 120, 6);
        assert_eq!(
            srv.local_move_delta(960, 1072, 0, 40),
            Some(ScreenEdge::Bottom)
        );
        assert_eq!(srv.seat(), CursorSeat::Remote);
        let _ = srv.drain();

        // OS-clamped tail: hook reports the same edge position, zero delta.
        for _ in 0..10 {
            srv.local_move_delta(960, 1079, 0, 0);
            let _ = srv.drain();
        }
        let (_, fy) = srv.remote_pos();
        assert_eq!(fy, 0, "zero-delta tail must not invent motion");
        assert_eq!(
            srv.seat(),
            CursorSeat::Remote,
            "tail must not strand the seat"
        );

        // Hook recentered the cursor to mid-screen; the first real event
        // after the settle reports zero delta (baseline forgotten), then
        // real inward motion resumes and MUST reach the interior.
        srv.local_move_delta(960, 540, 0, 0);
        let _ = srv.drain();
        for _ in 0..30 {
            srv.local_move_delta(960, 570, 0, 30);
            let _ = srv.drain();
        }
        let (_, ry) = srv.remote_pos();
        assert!(
            ry > 200,
            "real inward motion after the zero tail must advance the remote cursor, got rem_y={ry}"
        );
    }

    /// v0.2.30 wire economy: a Remote-seat event that leaves the normalized
    /// remote position unchanged carries no information for the target —
    /// the previous Move already placed the cursor there. Such events come
    /// from the OS-clamped zero-delta tail (Windows), the post-warp settle
    /// events whose deltas the macOS tap zeroes (4 per park warp), and
    /// sub-unit deltas whose fraction is still accumulating. They must
    /// still update the engine's bookkeeping (fraction carry, local x/y)
    /// but must NOT push a redundant Move frame; the first event that does
    /// change the remote position pushes exactly one.
    #[test]
    fn remote_seat_suppresses_move_frames_when_remote_position_unchanged() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Bottom);
        srv.set_tb_redundancy_band(0, 120, 6);
        assert_eq!(
            srv.local_move_delta(960, 1072, 0, 40),
            Some(ScreenEdge::Bottom)
        );
        assert_eq!(srv.seat(), CursorSeat::Remote);
        let entry = srv.drain();
        assert!(
            matches!(entry.as_slice(), [a, b]
                if matches!(a.kind, InputKind::Enter)
                    && matches!(b.kind, InputKind::Move { .. })),
            "entry must still push enter+move, got {entry:?}"
        );
        let before = srv.remote_pos();

        // Zero-delta tail / settle events: nothing changes, nothing pushed.
        for _ in 0..4 {
            assert_eq!(srv.local_move_delta(960, 540, 0, 0), None);
            let drained = srv.drain();
            assert!(
                drained.is_empty(),
                "zero-delta event must not push a redundant Move, got {drained:?}"
            );
        }
        assert_eq!(srv.remote_pos(), before);
        assert_eq!(srv.seat(), CursorSeat::Remote);

        // Sub-unit delta: 1 px on a 1920 px screen is 0.52 units — the first
        // event carries the fraction without moving, the second crosses the
        // unit boundary and pushes exactly one Move.
        assert_eq!(srv.local_move_delta(961, 540, 1, 0), None);
        assert!(
            srv.drain().is_empty(),
            "sub-unit delta with unchanged rem must not push a Move"
        );
        assert_eq!(srv.remote_pos(), before);
        assert_eq!(srv.local_move_delta(962, 540, 1, 0), None);
        let moved = srv.drain();
        assert_eq!(
            moved.len(),
            1,
            "unit-crossing delta pushes exactly one Move"
        );
        match moved[0].kind {
            InputKind::Move { x, y } => {
                assert_eq!((x, y), (before.0 + 1, before.1));
            }
            ref other => panic!("expected Move, got {other:?}"),
        }

        // A real inward delta still pushes one Move per event.
        assert_eq!(srv.local_move_delta(962, 570, 0, 30), None);
        let moved = srv.drain();
        assert_eq!(moved.len(), 1);
        let (_, ry) = srv.remote_pos();
        assert!(ry > before.1, "real inward motion must advance rem_y");
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
        // v0.2.26: the return dock must sit OUTSIDE the redundancy band
        // (band_px=0 here -> band_safe_dock_y floors at 48); the old y=16
        // parked inside the band and re-crossed on the next stray dy<=0
        // event (production ghost-controller bounce loop).
        assert_eq!(
            srv.local_pos().1,
            48,
            "far-push return dock outside the band"
        );

        // Test force_local
        srv.local_move_delta(960, 0, 0, -10);
        assert_eq!(srv.seat(), CursorSeat::Remote);
        srv.force_local();
        assert_eq!(srv.seat(), CursorSeat::Local);
        let events = srv.drain();
        assert_eq!(events[events.len() - 1].kind, InputKind::Leave);
    }

    /// v0.2.32 ramp: a residual push inside the entry grace (below the
    /// raised threshold) must not return the seat; the same push after the
    /// grace clears the base threshold and returns normally.
    #[test]
    fn entry_return_grace_blocks_residual_return_then_allows_it() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Top);
        srv.set_entry_return_grace(std::time::Duration::from_millis(300));

        assert_eq!(srv.local_move_delta(960, 0, 0, -10), Some(ScreenEdge::Top));
        assert_eq!(srv.seat(), CursorSeat::Remote);
        srv.drain();
        // Interior move (973 units, past the 20-unit hysteresis), then a
        // 30 px push back onto the shared edge within the grace: the legacy
        // engine would return here (30 px >= 12), the ramp demands up to
        // 48 px this early.
        assert_eq!(srv.local_move_delta(960, 0, 0, -30), None);
        assert_eq!(srv.local_move_delta(960, 0, 0, 30), None);
        assert_eq!(
            srv.seat(),
            CursorSeat::Remote,
            "ramp must block the residual return"
        );

        std::thread::sleep(std::time::Duration::from_millis(320));
        assert_eq!(srv.local_move_delta(960, 0, 0, 30), Some(ScreenEdge::Top));
        assert_eq!(srv.seat(), CursorSeat::Local);
        let events = srv.drain();
        assert_eq!(events.last().unwrap().kind, InputKind::Leave);
    }

    /// The core default keeps the legacy instant return (every existing
    /// crossing test relies on it); only an explicit grace changes it.
    #[test]
    fn entry_return_grace_defaults_to_zero() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Top);
        assert_eq!(srv.local_move_delta(960, 0, 0, -10), Some(ScreenEdge::Top));
        srv.drain();
        assert_eq!(srv.local_move_delta(960, 0, 0, -100), None);
        assert_eq!(srv.local_move_delta(960, 0, 0, 300), Some(ScreenEdge::Top));
        assert_eq!(srv.seat(), CursorSeat::Local);
    }

    /// v0.2.32: the return push required at the shared edge decays linearly
    /// from 10x (v0.2.38; 4x in v0.2.32) the base threshold at the instant
    /// of entry down to the base
    /// threshold when the grace ends; with no grace (or after it) the base
    /// threshold applies.
    #[test]
    fn return_push_threshold_decays_linearly_over_the_entry_grace() {
        use std::time::Duration;
        let g = Duration::from_millis(3000);
        let base = InputServer::FAR_PUSH_RETURN_PX;
        assert_eq!(
            InputServer::required_return_push_px(Duration::ZERO, g),
            base * 10
        );
        assert_eq!(
            InputServer::required_return_push_px(Duration::from_millis(1500), g),
            base * 10 - (base * 9) / 2
        );
        assert_eq!(InputServer::required_return_push_px(g, g), base);
        assert_eq!(InputServer::required_return_push_px(g * 2, g), base);
        assert_eq!(
            InputServer::required_return_push_px(Duration::ZERO, Duration::ZERO),
            base
        );
    }

    /// v0.2.32: inside the grace a RESIDUAL push (above the legacy 12 px but
    /// below the ramped threshold) must not return, while a DELIBERATE flick
    /// returns immediately — the v0.2.31 hard cut blocked both and then let
    /// the residual momentum bounce the seat the instant the grace expired
    /// (production 2026-11-13: five bounces each ~1.0 s after entry).
    #[test]
    fn entry_return_ramp_ignores_residual_push_but_honors_deliberate_flick() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Top);
        srv.set_entry_return_grace(std::time::Duration::from_millis(3000));

        assert_eq!(srv.local_move_delta(960, 0, 0, -10), Some(ScreenEdge::Top));
        srv.drain();
        // Shallow interior (rem_y 1000 -> 972, past the 20-unit hysteresis).
        assert_eq!(srv.local_move_delta(960, 0, 0, -30), None);
        // Residual drift back onto the shared edge: 20 px (not at edge yet)
        // then 15 px (saturated at the edge, counted). Legacy threshold 12
        // would return on the second event.
        assert_eq!(srv.local_move_delta(960, 0, 0, 20), None);
        assert_eq!(srv.local_move_delta(960, 0, 0, 15), None);
        assert_eq!(
            srv.seat(),
            CursorSeat::Remote,
            "residual 15 px push inside the grace must not return"
        );
        // A deliberate flick clears even the entry-time threshold.
        assert_eq!(srv.local_move_delta(960, 0, 0, 300), Some(ScreenEdge::Top));
        assert_eq!(srv.seat(), CursorSeat::Local);
        let events = srv.drain();
        assert_eq!(events.last().unwrap().kind, InputKind::Leave);
    }

    /// v0.2.38 (production 2026-11-13 19:27, five Windows→Mac bounces each
    /// 0.34–0.63 s after entry): the user strokes down, the cursor crosses
    /// late in the stroke and only travels a shallow distance into the
    /// peer, then the hand's lift-and-reset stroke drags it back up. Only
    /// the motion that goes PAST the shared edge is a push against it; the
    /// travel back to the edge is not. Counting the whole event delta made
    /// a ~50 px reset stroke clear the entry-time threshold.
    #[test]
    fn shallow_entry_reset_stroke_does_not_bounce_the_seat_back() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Bottom);
        srv.set_remote_screen(Some((1920, 1080)));
        srv.set_entry_return_grace(std::time::Duration::from_millis(3000));

        assert_eq!(
            srv.local_move_delta(729, 1079, 0, 4),
            Some(ScreenEdge::Bottom)
        );
        srv.drain();
        // Residual stroke momentum: 40 px into the peer (past hysteresis).
        assert_eq!(srv.local_move_delta(960, 540, 0, 40), None);
        // Lift-and-reset stroke: 10 x 9 px upward (90 px, ~50 px of it
        // beyond the shared edge).
        for _ in 0..10 {
            assert_eq!(srv.local_move_delta(960, 531, 0, -9), None);
        }
        assert_eq!(
            srv.seat(),
            CursorSeat::Remote,
            "a reset stroke right after a shallow entry must not return"
        );
    }

    /// The deliberate return right after an accidental crossing still works
    /// inside the entry grace: a real flick back pushes far past the edge.
    #[test]
    fn deliberate_flick_back_right_after_entry_still_returns() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Bottom);
        srv.set_remote_screen(Some((1920, 1080)));
        srv.set_entry_return_grace(std::time::Duration::from_millis(3000));

        assert_eq!(
            srv.local_move_delta(729, 1079, 0, 4),
            Some(ScreenEdge::Bottom)
        );
        srv.drain();
        assert_eq!(srv.local_move_delta(960, 540, 0, 40), None);
        assert_eq!(
            srv.local_move_delta(960, 340, 0, -200),
            Some(ScreenEdge::Bottom)
        );
        assert_eq!(srv.seat(), CursorSeat::Local);
    }

    /// The return reports its own evidence (time since entry, deepest
    /// interior travel, the push that cleared the threshold) so a
    /// production bounce can be attributed from the log alone.
    #[test]
    fn return_records_entry_elapsed_peak_depth_and_push() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Bottom);
        srv.set_remote_screen(Some((1920, 1080)));

        assert!(srv.last_return_diag().is_none());
        assert_eq!(
            srv.local_move_delta(729, 1079, 0, 4),
            Some(ScreenEdge::Bottom)
        );
        assert_eq!(srv.local_move_delta(960, 540, 0, 108), None);
        assert_eq!(srv.local_move_delta(960, 540, 0, -54), None);
        assert_eq!(
            srv.local_move_delta(960, 540, 0, -108),
            Some(ScreenEdge::Bottom)
        );
        let diag = srv.last_return_diag().expect("return diag");
        assert_eq!(diag.peak_depth, 100);
        assert_eq!(diag.required_push, InputServer::FAR_PUSH_RETURN_PX);
        assert!(diag.push >= 50 && diag.push <= 60, "push={}", diag.push);
        assert!(diag.elapsed_ms < 1000);
    }

    /// v0.2.39: the return also reports the raw stroke balance of the
    /// stint (local px moved away from vs toward the shared edge, event
    /// count, largest single toward-edge event) so a production bounce
    /// separates a hand scrubbing back and forth (large away AND toward)
    /// from a biased delta stream (small away, steady toward).
    #[test]
    fn return_records_stint_stroke_balance() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Bottom);
        srv.set_remote_screen(Some((1920, 1080)));

        assert_eq!(
            srv.local_move_delta(729, 1079, 0, 4),
            Some(ScreenEdge::Bottom)
        );
        assert_eq!(srv.local_move_delta(960, 540, 3, 108), None);
        assert_eq!(srv.local_move_delta(960, 540, 0, 20), None);
        assert_eq!(srv.local_move_delta(960, 540, 0, -54), None);
        assert_eq!(
            srv.local_move_delta(960, 540, 0, -108),
            Some(ScreenEdge::Bottom)
        );
        let diag = srv.last_return_diag().expect("return diag");
        assert_eq!(diag.away_px, 128);
        assert_eq!(diag.toward_px, 162);
        assert_eq!(diag.events, 4);
        assert_eq!(diag.max_toward, 108);

        // The next stint starts from a clean balance.
        assert_eq!(
            srv.local_move_delta(729, 1079, 0, 4),
            Some(ScreenEdge::Bottom)
        );
        assert_eq!(srv.local_move_delta(960, 540, 0, 108), None);
        assert_eq!(
            srv.local_move_delta(960, 540, 0, -400),
            Some(ScreenEdge::Bottom)
        );
        let diag = srv.last_return_diag().expect("second return diag");
        assert_eq!(diag.away_px, 108);
        assert_eq!(diag.toward_px, 400);
        assert_eq!(diag.events, 2);
        assert_eq!(diag.max_toward, 400);
    }

    /// v0.2.32: with the peer's screen known, remote-seat deltas normalize
    /// by the REMOTE dimension so one local px moves one remote px/pt on
    /// both axes. Normalizing by the local 1920x1080 against a 1470x956
    /// peer gave 0.77 / 0.89 pt per px (slow and anisotropic).
    #[test]
    fn remote_screen_normalizes_deltas_by_peer_dimensions() {
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Bottom);
        srv.set_remote_screen(Some((1470, 956)));
        assert_eq!(
            srv.local_move_delta(960, 1080, 0, 10),
            Some(ScreenEdge::Bottom)
        );
        srv.drain();
        let (rx0, ry0) = srv.remote_pos();
        // 147 px = 10% of the remote width = 100 units.
        assert_eq!(srv.local_move_delta(960, 540, 147, 0), None);
        // 478 px = 50% of the remote height = 500 units.
        assert_eq!(srv.local_move_delta(960, 540, 0, 478), None);
        let (rx1, ry1) = srv.remote_pos();
        assert_eq!((rx1 - rx0, ry1 - ry0), (100, 500));

        // Clearing the peer screen falls back to the local normalization
        // (192 px = 10% of the local width).
        srv.set_remote_screen(None);
        assert_eq!(srv.local_move_delta(960, 540, 192, 0), None);
        let (rx2, _) = srv.remote_pos();
        assert_eq!(rx2 - rx1, 100);
    }

    /// v0.2.32: `remote_speed_percent` scales the normalized remote delta
    /// (100 = 1:1 against the peer screen; 150 = 1.5x).
    #[test]
    fn remote_speed_percent_scales_remote_deltas() {
        let mut srv = InputServer::new(1000, 1000);
        srv.set_edge(ScreenEdge::Bottom);
        srv.set_remote_screen(Some((1000, 1000)));
        srv.set_remote_speed_percent(150);
        assert_eq!(
            srv.local_move_delta(500, 1000, 0, 10),
            Some(ScreenEdge::Bottom)
        );
        srv.drain();
        let (rx0, _) = srv.remote_pos();
        assert_eq!(srv.local_move_delta(500, 500, 100, 0), None);
        let (rx1, _) = srv.remote_pos();
        assert_eq!(rx1 - rx0, 150);
        // 0 is not a valid speed: it clamps to the minimum, never freezes.
        srv.set_remote_speed_percent(0);
        assert_eq!(srv.local_move_delta(500, 500, 100, 0), None);
        let (rx2, _) = srv.remote_pos();
        assert!(rx2 > rx1, "a zero speed must not freeze the remote cursor");
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
        // the last remote x (Deskflow OSXScreen semantics), parked just
        // outside the redundancy band — NOT the raw hook position (which
        // sits wherever the parked local cursor was left). 60 px: 40 back
        // to the shared edge plus a 20 px push past it (v0.2.38 counts only
        // the overshoot).
        assert_eq!(srv.local_move_delta(250, 0, 0, 60), Some(ScreenEdge::Top));
        let _ = srv.drain();
        assert_eq!(srv.seat(), CursorSeat::Local);
        let (lx, ly) = srv.local_pos();
        assert_eq!(ly, 48, "band-safe dock (band_px=0 -> floor 48)");
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
        assert_eq!(ly, 48, "force_local dock outside the band");
        // norm(750) = 751 (rounded), denorm(751) = 750 (rounded): the round
        // trip returns exactly to the origin — the truncating chain
        // (750 -> 749) walked a pixel per emergency return.
        assert_eq!(lx, 750, "force_local x = denorm(rem_x)");
    }

    #[test]
    fn tb_band_dock_never_recrosses_from_a_stray_quiet_event() {
        // v0.2.26 ghost-controller bounce loop regression: the rescue /
        // far-push return used to dock at y=16, INSIDE the 24px redundancy
        // band. There the band gate's slow-dwell path (quiet >= 120ms —
        // trivially true when nobody touches the mouse) turned any stray
        // dy<=0 event (a warp echo that missed the echo gate) into an
        // instant re-crossing: the node became a ghost controller, the
        // target's physical mouse was swallowed, and the watcher
        // rescue->pre-arm->crossing loop ran every ~13s.
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Top);
        srv.set_tb_redundancy_band(24, 120, 6);

        // Enter remote, then emergency-rescue back to local.
        assert_eq!(srv.local_move_delta(960, 4, 0, -10), Some(ScreenEdge::Top));
        let _ = srv.drain();
        srv.force_local();
        let _ = srv.drain();
        let (lx, ly) = srv.local_pos();
        assert_eq!(lx, 960);
        assert!(ly > 24, "dock y={ly} must sit outside the 24px band");

        // The exact production ghost event: a single stray dy<=0 event at
        // the dock, after a long quiet period (no arm, no dwell history).
        // From OUTSIDE the band it must NOT cross — even as a "fast push" —
        // because the position is not at the edge (y > 8).
        assert_eq!(
            srv.local_move_delta(lx, ly, 0, -2),
            None,
            "stray quiet event at the band-safe dock must not re-cross"
        );
        assert_eq!(srv.seat(), CursorSeat::Local);
        let _ = srv.drain();

        // And a real user approach still crosses: glide into the band, then
        // hold quiet past the 120ms dwell. (The -6 in-band arrival itself is
        // a fast push and crosses immediately — assert that path too.)
        assert_eq!(
            srv.local_move_delta(960, 40, 0, -10),
            None,
            "glide outside the band stays local"
        );
        assert_eq!(
            srv.local_move_delta(960, 6, 0, -6),
            Some(ScreenEdge::Top),
            "fast in-band push crosses immediately"
        );
        assert_eq!(srv.seat(), CursorSeat::Remote);
        let _ = srv.drain();
        srv.force_local();
        let _ = srv.drain();
        // Slow-dwell path: land in the band gently (dy < fast push), then
        // hold quiet past the 120ms dwell.
        assert_eq!(srv.local_move_delta(960, 40, 0, -4), None);
        assert_eq!(
            srv.local_move_delta(960, 6, 0, -4),
            None,
            "slow in-band arrival waits"
        );
        std::thread::sleep(std::time::Duration::from_millis(140));
        assert_eq!(
            srv.local_move_delta(960, 6, 0, 0),
            Some(ScreenEdge::Top),
            "real sustained edge push still crosses after the dwell"
        );
        assert_eq!(srv.seat(), CursorSeat::Remote);
    }

    #[test]
    fn tb_redundancy_band_speed_semantics() {
        // Defaults in InputServer::new keep the legacy instant cross (band
        // disabled, band_px=0) — the pre-gate behavior all existing tests
        // assert.
        let mut disabled = InputServer::new(1920, 1080);
        disabled.set_edge(ScreenEdge::Top);
        assert_eq!(
            disabled.local_move_delta(960, 0, 0, 0),
            Some(ScreenEdge::Top)
        );
        assert_eq!(disabled.seat(), CursorSeat::Remote);

        // Enabled band: slow glide (|dy| < fast threshold) inside the band
        // must NOT cross on the first event; after the quiet dwell elapses
        // the next boundary event crosses. A fast push crosses immediately.
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Top);
        srv.set_tb_redundancy_band(24, 120, 6);

        // Slow glide toward the menu bar: no crossing while dwell arms.
        assert_eq!(
            srv.local_move_delta(960, 3, 0, -1),
            None,
            "slow glide into the band must not cross instantly"
        );
        assert_eq!(srv.seat(), CursorSeat::Local);
        assert_eq!(srv.local_move_delta(960, 1, 0, -1), None);
        assert_eq!(
            srv.local_move_delta(960, 0, 0, 0),
            None,
            "clamped tail with zero delta still waits out the dwell"
        );
        assert_eq!(srv.seat(), CursorSeat::Local);

        // Quiet dwell completed (test sleeps past slow_dwell_ms): the very
        // next boundary event honors the rest and crosses.
        std::thread::sleep(std::time::Duration::from_millis(140));
        assert_eq!(
            srv.local_move_delta(960, 0, 0, 0),
            Some(ScreenEdge::Top),
            "settled slow glide crosses after the quiet dwell"
        );
        assert_eq!(srv.seat(), CursorSeat::Remote);
        srv.on_remote_leave();
        assert_eq!(srv.seat(), CursorSeat::Local);

        // Fast push: crosses on the first event without any dwell.
        let mut fast = InputServer::new(1920, 1080);
        fast.set_edge(ScreenEdge::Top);
        fast.set_tb_redundancy_band(24, 120, 6);
        assert_eq!(
            fast.local_move_delta(960, 5, 0, -8),
            Some(ScreenEdge::Top),
            "fast push must cross immediately"
        );
        assert_eq!(fast.seat(), CursorSeat::Remote);

        // Outside the band a slow glide never crosses (and resets dwell):
        // y=30 is deeper than the 24px band.
        let mut outside = InputServer::new(1920, 1080);
        outside.set_edge(ScreenEdge::Top);
        outside.set_tb_redundancy_band(24, 120, 6);
        assert_eq!(
            outside.local_move_delta(960, 30, 0, -1),
            None,
            "slow glide outside the band must not arm"
        );
        std::thread::sleep(std::time::Duration::from_millis(140));
        assert_eq!(
            outside.local_move_delta(960, 30, 0, 0),
            None,
            "dwelling outside the band never crosses"
        );
        assert_eq!(outside.seat(), CursorSeat::Local);

        // Bottom mirror: slow glide toward the task bar waits; fast push
        // crosses instantly.
        let mut bottom = InputServer::new(1920, 1080);
        bottom.set_edge(ScreenEdge::Bottom);
        bottom.set_tb_redundancy_band(24, 120, 6);
        assert_eq!(
            bottom.local_move_delta(960, 1079, 0, 1),
            None,
            "slow glide into the bottom band must not cross instantly"
        );
        assert_eq!(bottom.seat(), CursorSeat::Local);
        std::thread::sleep(std::time::Duration::from_millis(140));
        assert_eq!(
            bottom.local_move_delta(960, 1079, 0, 0),
            Some(ScreenEdge::Bottom),
            "settled slow glide crosses after dwell"
        );

        let mut bottom_fast = InputServer::new(1920, 1080);
        bottom_fast.set_edge(ScreenEdge::Bottom);
        bottom_fast.set_tb_redundancy_band(24, 120, 6);
        assert_eq!(
            bottom_fast.local_move_delta(960, 1075, 0, 8),
            Some(ScreenEdge::Bottom),
            "fast push must cross immediately"
        );
    }

    #[test]
    fn tb_band_config_mirror_is_idempotent() {
        // The node mirrors persisted band config into the engine on every
        // mouse event (drive_input_delta). Re-asserting identical values must
        // NOT reset the quiet-dwell timestamp — otherwise the 120ms dwell
        // could never accumulate and a resting hand at the OS-clamped edge
        // (y=height-1, dy=0) could never cross. Regression for the Windows
        // bottom-edge dead band after v0.2.6.
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Bottom);
        srv.set_tb_redundancy_band(24, 120, 6);

        // First in-band event arms the dwell...
        assert_eq!(srv.local_move_delta(960, 1079, 0, 1), None);
        // ...and repeated identical config mirrors (one per event) must leave
        // the armed dwell intact. Before the fix each mirror reset it.
        for _ in 0..50 {
            srv.set_tb_redundancy_band(24, 120, 6);
        }
        assert_eq!(srv.local_move_delta(960, 1079, 0, 0), None);
        srv.set_tb_redundancy_band(24, 120, 6);
        std::thread::sleep(std::time::Duration::from_millis(140));
        srv.set_tb_redundancy_band(24, 120, 6);
        assert_eq!(
            srv.local_move_delta(960, 1079, 0, 0),
            Some(ScreenEdge::Bottom),
            "identical config mirrors must not starve the slow dwell"
        );

        // A genuine config change must still reset the dwell.
        let mut srv = InputServer::new(1920, 1080);
        srv.set_edge(ScreenEdge::Bottom);
        srv.set_tb_redundancy_band(24, 120, 6);
        assert_eq!(srv.local_move_delta(960, 1079, 0, 1), None);
        srv.set_tb_redundancy_band(24, 120, 300);
        std::thread::sleep(std::time::Duration::from_millis(140));
        assert_eq!(
            srv.local_move_delta(960, 1079, 0, 0),
            None,
            "changed dwell config must re-arm the quiet timer"
        );
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
