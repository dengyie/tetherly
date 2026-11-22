// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Trackpad gesture → remote action mapping layer.
//!
//! Inspired by fusuma's gesture-table config (swipe/pinch × fingers ×
//! direction → command), Touchégg's action taxonomy (SEND_KEYS,
//! CHANGE_DESKTOP, …) and BetterTouchTool's predefined-action vocabulary —
//! restructured for a *remote-control* transport: the user does not bind
//! "gesture → keys" (that couples the config to one target OS), they bind
//! "gesture → intent", and each intent has a per-platform backend.
//!
//! ```text
//! GestureId (physical: swipe4_left, swipe4_up, …)
//!     │  user-mappable (GestureMap, persisted JSON)
//!     ▼
//! GestureAction (intent: AppSwitchNext, TaskView, …)
//!     │  per-target-platform backend
//!     ▼
//! Vec<GestureKeyStep> (raw VK sequence, keymap-bypassed)
//! ```
//!
//! Adding a new gesture or a new intent is one enum variant plus one
//! backend branch; the config surface stays stable.

use serde::{Deserialize, Serialize};

/// A physical gesture this node can observe on its local trackpad.
/// Variants are named after what the fingers DO, never after what the
/// local OS does with them (the local meaning is whatever we suppress).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureId {
    /// Four-finger horizontal swipe, fingers moving left.
    Swipe4Left,
    /// Four-finger horizontal swipe, fingers moving right.
    Swipe4Right,
    /// Four-finger vertical swipe, fingers moving up.
    Swipe4Up,
    /// Four-finger vertical swipe, fingers moving down.
    Swipe4Down,
}

impl GestureId {
    /// All gestures, in a stable display order.
    pub const ALL: [GestureId; 4] = [
        GestureId::Swipe4Left,
        GestureId::Swipe4Right,
        GestureId::Swipe4Up,
        GestureId::Swipe4Down,
    ];

    /// Parse from the wire/UI form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "swipe4_left" => Some(Self::Swipe4Left),
            "swipe4_right" => Some(Self::Swipe4Right),
            "swipe4_up" => Some(Self::Swipe4Up),
            "swipe4_down" => Some(Self::Swipe4Down),
            _ => None,
        }
    }

    /// Serialize to the wire/UI form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Swipe4Left => "swipe4_left",
            Self::Swipe4Right => "swipe4_right",
            Self::Swipe4Up => "swipe4_up",
            Self::Swipe4Down => "swipe4_down",
        }
    }

    /// Stable one-line description for UI listing.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Swipe4Left => "four-finger swipe left",
            Self::Swipe4Right => "four-finger swipe right",
            Self::Swipe4Up => "four-finger swipe up",
            Self::Swipe4Down => "four-finger swipe down",
        }
    }
}

/// How the Windows app-switch gesture (swipe mapped to AppSwitchNext/Prev)
/// injects its key sequence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppSwitchMode {
    /// Per-swipe complete Alt+Tab (with Alt-up): each swipe commits one
    /// switch immediately. The classic single-step behavior — and also the
    /// reason consecutive swipes bounce between two windows (Windows'
    /// commit key is Alt-up, so a completed sequence restarts the cycle
    /// from the fresh focus every time).
    #[default]
    PerSwipe,
    /// Sticky chord: the first swipe presses Alt and HOLDS it; extending
    /// swipes tap only Tab (Shift joins and stays when direction flips);
    /// silence commits (Alt-up) via the controller's watcher tick. Walks
    /// the window cycle continuously, macOS-Mission-Control style.
    StickyChord,
}

impl AppSwitchMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "per_swipe" => Some(Self::PerSwipe),
            "sticky_chord" => Some(Self::StickyChord),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PerSwipe => "per_swipe",
            Self::StickyChord => "sticky_chord",
        }
    }
}

/// What the user wants to happen on the CONTROLLING target. Intents are
/// platform-neutral nouns modeled on Touchégg's action taxonomy plus the
/// window-manager verbs BetterTouchTool exposes; each resolves to a concrete
/// key sequence per target platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GestureAction {
    /// Cycle to the next window (Mission Control's row traversal).
    AppSwitchNext,
    /// Cycle to the previous window.
    AppSwitchPrev,
    /// Open/close the desktop+window overview (Mission Control ↔ Task View).
    OverviewToggle,
    /// Move one virtual desktop toward the content's left.
    DesktopLeft,
    /// Move one virtual desktop toward the content's right.
    DesktopRight,
    /// Show the bare desktop (F11 / Show Desktop).
    ShowDesktop,
    /// Do nothing (a gesture can be silenced without unbinding the rest).
    None,
}

impl GestureAction {
    /// Every intent, in stable display order.
    pub const ALL: [GestureAction; 7] = [
        GestureAction::AppSwitchNext,
        GestureAction::AppSwitchPrev,
        GestureAction::OverviewToggle,
        GestureAction::DesktopLeft,
        GestureAction::DesktopRight,
        GestureAction::ShowDesktop,
        GestureAction::None,
    ];

    /// Parse from the wire/UI form.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "app_switch_next" => Some(Self::AppSwitchNext),
            "app_switch_prev" => Some(Self::AppSwitchPrev),
            "overview_toggle" => Some(Self::OverviewToggle),
            "desktop_left" => Some(Self::DesktopLeft),
            "desktop_right" => Some(Self::DesktopRight),
            "show_desktop" => Some(Self::ShowDesktop),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    /// Serialize to the wire/UI form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AppSwitchNext => "app_switch_next",
            Self::AppSwitchPrev => "app_switch_prev",
            Self::OverviewToggle => "overview_toggle",
            Self::DesktopLeft => "desktop_left",
            Self::DesktopRight => "desktop_right",
            Self::ShowDesktop => "show_desktop",
            Self::None => "none",
        }
    }

    /// Human label for the UI.
    pub fn describe(self) -> &'static str {
        match self {
            Self::AppSwitchNext => "next app window",
            Self::AppSwitchPrev => "previous app window",
            Self::OverviewToggle => "desktop overview (Mission Control / Task View)",
            Self::DesktopLeft => "switch desktop left",
            Self::DesktopRight => "switch desktop right",
            Self::ShowDesktop => "show desktop",
            Self::None => "do nothing",
        }
    }
}

/// Win32 virtual-key codes used by the backends (duplicated locally — the
/// keymap module owns its own set and gesture sequences must NOT be routed
/// through keymap remapping, so the layer stays self-contained).
mod vk {
    pub const TAB: u16 = 0x09;
    pub const LWIN: u16 = 0x5B;
    pub const LSHIFT: u16 = 0xA0;
    pub const LMENU: u16 = 0xA4;
    pub const LCONTROL: u16 = 0xA2;
    pub const D: u16 = 0x44;
    pub const LEFT: u16 = 0x25;
    pub const RIGHT: u16 = 0x27;
    /// ` / ~ key (VK_OEM_3) — the backtick of the macOS Cmd+` cycle.
    pub const OEM_3: u16 = 0xC0;
}

/// One key event of an injected sequence: modifier or normal key, down or
/// up. A gesture sequence is [down modifiers…] [tap keys…] [up modifiers…]
/// — the same shape the Alt+Esc / Win+Tab sequences already use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GestureKeyStep {
    pub code: u16,
    pub down: bool,
}

/// Gesture sequences for a WINDOWS target. Raw Win32 VKs, deliberately
/// outside the keymap: mac_to_windows remaps Meta→Ctrl, which would turn
/// Win+Tab into a browser tab switch. Hotkeys backed by RegisterHotKey or
/// direct window-manager handling (Win+Tab, Win+Ctrl+Arrow, Win+D) consume
/// synthetic input reliably.
///
/// Alt+Tab for the app switch (v0.2.27): the previous Alt+Esc sequence
/// looked "window-manager handled" but is NOT a RegisterHotKey — Alt+Esc is
/// consumed by the FOREGROUND window's `DefWindowProc`, so the ESC tap is
/// delivered to the focused app as `WM_SYSKEYDOWN` before the switch
/// happens. A browser in HTML5 fullscreen reacts to that ESC by leaving
/// fullscreen: the switch still completed, but the video dropped out
/// (production 2026-11-13, Mac→Windows swipe with a fullscreen browser
/// video; a physical Alt+Esc on the Windows keyboard does not reproduce it,
/// and deactivation itself is proven safe — the switch moves focus away and
/// the fullscreen survives). Alt+Tab removes ESC from the sequence entirely:
/// the overlay selects on the Tab tap and commits when Alt is released, so
/// the switch is the only observable effect. The Alt+Tab overlay state
/// machine concern that originally avoided this combo is bounded by the
/// same shape as the physical key: Alt release always commits, there is no
/// lingering state.
pub fn windows_backend(action: GestureAction) -> Vec<GestureKeyStep> {
    /// helper: modifier-tap-modifier shape
    fn tap(mods_down: &[u16], taps: &[u16]) -> Vec<GestureKeyStep> {
        let mut seq = Vec::with_capacity(mods_down.len() * 2 + taps.len() * 2);
        for m in mods_down {
            seq.push(GestureKeyStep {
                code: *m,
                down: true,
            });
        }
        for t in taps {
            seq.push(GestureKeyStep {
                code: *t,
                down: true,
            });
            seq.push(GestureKeyStep {
                code: *t,
                down: false,
            });
        }
        for m in mods_down.iter().rev() {
            seq.push(GestureKeyStep {
                code: *m,
                down: false,
            });
        }
        seq
    }
    match action {
        // Alt+Tab: switch to the next window. No ESC in the sequence — the
        // ESC tap of the old Alt+Esc shape reached the focused app as
        // WM_SYSKEYDOWN (DefWindowProc-mediated, not a RegisterHotKey) and
        // exited browser HTML5 fullscreen on arrival.
        GestureAction::AppSwitchNext => tap(&[vk::LMENU], &[vk::TAB]),
        // Shift+Alt+Tab: the reverse cycle.
        GestureAction::AppSwitchPrev => tap(&[vk::LSHIFT, vk::LMENU], &[vk::TAB]),
        // Win+Tab: Task View toggle (explorer RegisterHotKey).
        GestureAction::OverviewToggle => tap(&[vk::LWIN], &[vk::TAB]),
        // Win+Ctrl+Left/Right: virtual desktop switch.
        GestureAction::DesktopLeft => tap(&[vk::LWIN, vk::LCONTROL], &[vk::LEFT]),
        GestureAction::DesktopRight => tap(&[vk::LWIN, vk::LCONTROL], &[vk::RIGHT]),
        // Win+D: show desktop.
        GestureAction::ShowDesktop => tap(&[vk::LWIN], &[vk::D]),
        GestureAction::None => Vec::new(),
    }
}

/// Gesture sequences for a macOS target (this node controlling another Mac).
/// VK codes here are the wire convention (Win32 VK) — the target's platform
/// layer translates them to mac keycodes on arrival and, since v0.2.27,
/// tracks injected modifier state to set the matching `CGEventFlags`. That
/// flag tracking is what makes these sequences land as real shortcuts: a
/// synthetic Cmd keydown followed by a synthetic grave tap is two unrelated
/// keys to macOS unless the grave event carries `kCGEventFlagMaskCommand`.
pub fn macos_backend(action: GestureAction) -> Vec<GestureKeyStep> {
    // VK codes here are the wire convention (Win32 VK) — the target's
    // platform layer translates them to mac keycodes on arrival.
    const VK_CONTROL: u16 = 0xA2;
    const VK_UP: u16 = 0x26;
    match action {
        // Cmd+` : cycle windows of the frontmost app.
        GestureAction::AppSwitchNext => tap_mods_macos(&[vk::LWIN], &[vk::OEM_3]),
        // Cmd+Shift+` : the reverse cycle.
        GestureAction::AppSwitchPrev => tap_mods_macos(&[vk::LSHIFT, vk::LWIN], &[vk::OEM_3]),
        // Ctrl+Up: Mission Control keyboard surrogate.
        GestureAction::OverviewToggle => tap_mods_macos(&[VK_CONTROL], &[VK_UP]),
        // Ctrl+Left/Right: real Space (virtual desktop) switch — the plain
        // Control form macOS actually binds, NOT the Windows Win+Ctrl+Arrow.
        GestureAction::DesktopLeft => tap_mods_macos(&[VK_CONTROL], &[vk::LEFT]),
        GestureAction::DesktopRight => tap_mods_macos(&[VK_CONTROL], &[vk::RIGHT]),
        // No VK-reachable Show Desktop on macOS (F11 is volume down by
        // default; delegating to the Windows Win+D backend would arrive as
        // Cmd+D — "Don't Save" — now that LWIN carries the Command flag on
        // the target). Intentionally unmapped rather than harmful.
        GestureAction::ShowDesktop => Vec::new(),
        GestureAction::None => Vec::new(),
    }
}

fn tap_mods_macos(mods_down: &[u16], taps: &[u16]) -> Vec<GestureKeyStep> {
    let mut seq = Vec::with_capacity(mods_down.len() * 2 + taps.len() * 2);
    for m in mods_down {
        seq.push(GestureKeyStep {
            code: *m,
            down: true,
        });
    }
    for t in taps {
        seq.push(GestureKeyStep {
            code: *t,
            down: true,
        });
        seq.push(GestureKeyStep {
            code: *t,
            down: false,
        });
    }
    for m in mods_down.iter().rev() {
        seq.push(GestureKeyStep {
            code: *m,
            down: false,
        });
    }
    seq
}

/// Silence window after the last swipe within which another swipe EXTENDS
/// the held app-switch chord instead of starting a fresh one. Chosen to sit
/// between consecutive deliberate swipes (hundreds of ms apart) and a new
/// unrelated interaction; the Windows Alt+Tab overlay itself commits on Alt
/// release, so a slightly late release only means the overlay lingers.
pub const STICKY_CHORD_SILENCE_MS: u64 = 1_200;

/// State of the sticky app-switch chord (macOS-continuity semantics on a
/// Windows target): hold the modifier, tap Tab per swipe, commit on silence.
///
/// Windows semantics made per-swipe full sequences wrong for continuous
/// switching: Alt release COMMITS the switch, so a per-swipe Alt+Tab+Alt-up
/// bounces between two windows forever (each swipe is a complete commit).
/// The macOS gesture means "keep going one step in this direction", which on
/// Windows maps to HOLDING Alt and tapping Tab once per swipe.
///
/// The controller (`drive_gesture_swipe`) owns advancing this state; the
/// state itself is passive — held keys as ordered `GestureKeyStep`s so a
/// forced commit replays exact key-UPs in reverse-press order, including a
/// Shift that was injected mid-chord to reverse direction.
#[derive(Debug, Clone, Default)]
pub struct StickyChordState {
    /// Currently held modifier/tap steps, in press order. Empty = idle.
    /// A hold is [Alt down] or [Shift down, Alt down]; a mid-chord direction
    /// flip appends Shift down to the HELD set so the reverse overlay works.
    pub held: Vec<GestureKeyStep>,
    /// Wall-clock (controller clock) of the last chord-advancing swipe.
    pub last_swipe_ms: u64,
}

impl StickyChordState {
    pub fn is_active(&self) -> bool {
        !self.held.is_empty()
    }

    /// Key-UP steps that commit the chord, in reverse press order. Clears
    /// the state — call once per commit.
    pub fn take_commit(&mut self) -> Vec<GestureKeyStep> {
        let ups = self
            .held
            .drain(..)
            .rev()
            .map(|s| GestureKeyStep {
                code: s.code,
                down: false,
            })
            .collect();
        ups
    }
}

/// Resolve the events a swipe should inject given the current sticky-chord
/// state, for actions that support the sticky shape (currently the Windows
/// AppSwitchNext / AppSwitchPrev pair).
///
/// Returns `(steps, advanced)`:
/// - `advanced == false` means the action does not participate in the chord;
///   `steps` is the plain per-swipe sequence (a non-app-switch action, or
///   any action on a non-Windows target — the caller decides via the action
///   and platform) and the chord MUST be committed first by the caller if
///   it was active (an interrupting gesture releases the held modifier).
/// - `advanced == true` means the chord consumed this swipe: `steps` holds
///   only the incremental events (first swipe: modifier(s) down + Tab tap;
///   later swipes: optional Shift adjust + Tab tap), and `state` was updated
///   (`last_swipe_ms`, held set grown for a direction flip).
///
/// `now_ms` stamps `last_swipe_ms` so the caller's silence-commit timer can
/// measure against it.
pub fn sticky_chord_steps(
    state: &mut StickyChordState,
    action: GestureAction,
    now_ms: u64,
) -> (Vec<GestureKeyStep>, bool) {
    match action {
        GestureAction::AppSwitchNext | GestureAction::AppSwitchPrev => {}
        _ => return (Vec::new(), false),
    }
    let shift = matches!(action, GestureAction::AppSwitchPrev);
    let mut steps = Vec::new();
    if state.is_active() {
        // Extending swipe. Direction is a TWO-WAY transfer: flipping to
        // reverse adds Shift to the HELD set and keeps it (Windows overlay:
        // Shift held while Tabbing walks backwards; a per-swipe Shift
        // down/up would flip the next tap forward again), and flipping BACK
        // to forward releases it — otherwise one accidental reverse swipe
        // would pin the chord backwards until the next commit.
        let shift_held = state.held.iter().any(|s| s.code == vk::LSHIFT);
        if shift && !shift_held {
            state.held.push(GestureKeyStep {
                code: vk::LSHIFT,
                down: true,
            });
            steps.push(GestureKeyStep {
                code: vk::LSHIFT,
                down: true,
            });
        } else if !shift && shift_held {
            // Release BEFORE the Tab tap so this tap already walks forward.
            state.held.retain(|s| s.code != vk::LSHIFT);
            steps.push(GestureKeyStep {
                code: vk::LSHIFT,
                down: false,
            });
        }
        steps.push(GestureKeyStep {
            code: vk::TAB,
            down: true,
        });
        steps.push(GestureKeyStep {
            code: vk::TAB,
            down: false,
        });
    } else {
        // First swipe of a chord: press modifiers and HOLD them. Tab's tap
        // selects; the overlay commits only when the modifiers release.
        state.held.push(GestureKeyStep {
            code: vk::LMENU,
            down: true,
        });
        steps.push(GestureKeyStep {
            code: vk::LMENU,
            down: true,
        });
        if shift {
            state.held.push(GestureKeyStep {
                code: vk::LSHIFT,
                down: true,
            });
            steps.push(GestureKeyStep {
                code: vk::LSHIFT,
                down: true,
            });
        }
        steps.push(GestureKeyStep {
            code: vk::TAB,
            down: true,
        });
        steps.push(GestureKeyStep {
            code: vk::TAB,
            down: false,
        });
    }
    state.last_swipe_ms = now_ms;
    (steps, true)
}

/// The user-facing gesture→intent table. Defaults mirror macOS's own
/// trackpad semantics so the out-of-box behavior matches what the fingers
/// "mean" on a Mac — the swipe moves content, so `left` exposes what is to
/// the LEFT of the current window (the previous one), matching how the
/// Spaces swipe reads on the local machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GestureMap {
    pub swipe4_left: GestureAction,
    pub swipe4_right: GestureAction,
    pub swipe4_up: GestureAction,
    pub swipe4_down: GestureAction,
    /// Windows app-switch injection shape. Defaults to `per_swipe` so the
    /// classic single-step Alt+Tab behavior is preserved for existing
    /// configs (a missing field deserializes to the default); opt into the
    /// sticky chord for continuous same-direction cycling.
    #[serde(default)]
    pub app_switch_mode: AppSwitchMode,
}

impl Default for GestureMap {
    fn default() -> Self {
        Self {
            swipe4_left: GestureAction::AppSwitchNext,
            swipe4_right: GestureAction::AppSwitchPrev,
            swipe4_up: GestureAction::OverviewToggle,
            // macOS has no default "swipe down" meaning in this family;
            // leave it silenced rather than surprising.
            swipe4_down: GestureAction::None,
            app_switch_mode: AppSwitchMode::default(),
        }
    }
}

impl GestureMap {
    /// Resolve a physical gesture to its intent.
    pub fn action_for(&self, gesture: GestureId) -> GestureAction {
        match gesture {
            GestureId::Swipe4Left => self.swipe4_left,
            GestureId::Swipe4Right => self.swipe4_right,
            GestureId::Swipe4Up => self.swipe4_up,
            GestureId::Swipe4Down => self.swipe4_down,
        }
    }

    /// Full sequence for a gesture on a target platform, honoring the map.
    /// `None` steps produce an empty vec — callers treat that as "no-op".
    ///
    /// §192: platform matching must test "mac"/"darwin" BEFORE "win" — the
    /// wire platform string is `std::env::consts::OS`, so a macOS peer
    /// announces "macos" (or "darwin" via the hello override), and
    /// `"darwin".contains("win")` is true: matching "win" first would
    /// misroute every such peer to the Windows backend, whose Alt+Tab shape
    /// has no macOS meaning.
    pub fn sequence_for(&self, gesture: GestureId, target_platform: &str) -> Vec<GestureKeyStep> {
        let action = self.action_for(gesture);
        let p = target_platform.to_ascii_lowercase();
        if p.contains("mac") || p.contains("darwin") || p.contains("ios") {
            macos_backend(action)
        } else if p.contains("win") {
            windows_backend(action)
        } else {
            // Unknown platform: Windows-style is the wire default.
            windows_backend(action)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_shipped_behavior() {
        let m = GestureMap::default();
        assert_eq!(
            m.action_for(GestureId::Swipe4Left),
            GestureAction::AppSwitchNext
        );
        assert_eq!(
            m.action_for(GestureId::Swipe4Right),
            GestureAction::AppSwitchPrev
        );
        assert_eq!(
            m.action_for(GestureId::Swipe4Up),
            GestureAction::OverviewToggle
        );
        assert_eq!(m.action_for(GestureId::Swipe4Down), GestureAction::None);
    }

    #[test]
    fn win_sequences_match_shipped_behavior() {
        let m = GestureMap::default();
        // Left swipe == Alt down, Tab tap, Alt up. v0.2.27 replaced the old
        // Alt+Esc shape: its ESC tap reached the focused app as
        // WM_SYSKEYDOWN (Alt+Esc is DefWindowProc-mediated, NOT a
        // RegisterHotKey) and exited browser HTML5 fullscreen on arrival.
        assert_eq!(
            m.sequence_for(GestureId::Swipe4Left, "windows"),
            vec![
                GestureKeyStep {
                    code: 0xA4,
                    down: true
                },
                GestureKeyStep {
                    code: 0x09,
                    down: true
                },
                GestureKeyStep {
                    code: 0x09,
                    down: false
                },
                GestureKeyStep {
                    code: 0xA4,
                    down: false
                },
            ]
        );
        // Right swipe == Shift+Alt down, Tab tap, ups.
        assert_eq!(
            m.sequence_for(GestureId::Swipe4Right, "windows"),
            vec![
                GestureKeyStep {
                    code: 0xA0,
                    down: true
                },
                GestureKeyStep {
                    code: 0xA4,
                    down: true
                },
                GestureKeyStep {
                    code: 0x09,
                    down: true
                },
                GestureKeyStep {
                    code: 0x09,
                    down: false
                },
                GestureKeyStep {
                    code: 0xA4,
                    down: false
                },
                GestureKeyStep {
                    code: 0xA0,
                    down: false
                },
            ]
        );
        // Up swipe == old direction 0: Win+Tab.
        assert_eq!(
            m.sequence_for(GestureId::Swipe4Up, "windows"),
            vec![
                GestureKeyStep {
                    code: 0x5B,
                    down: true
                },
                GestureKeyStep {
                    code: 0x09,
                    down: true
                },
                GestureKeyStep {
                    code: 0x09,
                    down: false
                },
                GestureKeyStep {
                    code: 0x5B,
                    down: false
                },
            ]
        );
        // The app-switch sequence must never contain ESC: the whole point of
        // the v0.2.27 swap is that no Esc may reach the focused fullscreen
        // app.
        for gesture in [GestureId::Swipe4Left, GestureId::Swipe4Right] {
            for step in m.sequence_for(gesture, "windows") {
                assert_ne!(step.code, 0x1B, "gesture {gesture:?} must not send ESC");
            }
        }
    }

    #[test]
    fn none_action_yields_empty_sequence() {
        let m = GestureMap::default();
        assert!(m.sequence_for(GestureId::Swipe4Down, "windows").is_empty());
    }

    #[test]
    fn remap_roundtrip_and_unknown_rejected() {
        for g in GestureId::ALL {
            assert_eq!(GestureId::parse(g.as_str()), Some(g));
        }
        for a in GestureAction::ALL {
            assert_eq!(GestureAction::parse(a.as_str()), Some(a));
        }
        assert_eq!(GestureId::parse("swipe5_up"), None);
        assert_eq!(GestureAction::parse("launch_missiles"), None);
    }

    #[test]
    fn user_remap_changes_resolution() {
        let m = GestureMap {
            swipe4_down: GestureAction::ShowDesktop,
            ..GestureMap::default()
        };
        assert_eq!(
            m.sequence_for(GestureId::Swipe4Down, "windows"),
            vec![
                GestureKeyStep {
                    code: 0x5B,
                    down: true
                },
                GestureKeyStep {
                    code: 0x44,
                    down: true
                },
                GestureKeyStep {
                    code: 0x44,
                    down: false
                },
                GestureKeyStep {
                    code: 0x5B,
                    down: false
                },
            ]
        );
    }

    fn step(code: u16, down: bool) -> GestureKeyStep {
        GestureKeyStep { code, down }
    }

    /// v0.2.27: the macOS backend ships REAL shortcuts (Cmd+` / Cmd+Shift+`
    /// / plain Ctrl+Arrow) instead of delegating to the Windows wire
    /// sequences, whose Alt+Tab shape has no macOS meaning.
    #[test]
    fn mac_sequences_are_real_shortcuts() {
        let m = GestureMap::default();
        // AppSwitchNext == Cmd+` : LWIN down, grave tap, LWIN up.
        assert_eq!(
            m.sequence_for(GestureId::Swipe4Left, "macos"),
            vec![
                step(0x5B, true),
                step(0xC0, true),
                step(0xC0, false),
                step(0x5B, false)
            ]
        );
        // AppSwitchPrev == Cmd+Shift+` : downs [Shift, Win], ups [Win, Shift].
        assert_eq!(
            m.sequence_for(GestureId::Swipe4Right, "macos"),
            vec![
                step(0xA0, true),
                step(0x5B, true),
                step(0xC0, true),
                step(0xC0, false),
                step(0x5B, false),
                step(0xA0, false),
            ]
        );
        // Remapped DesktopLeft == plain Ctrl+Left (the Space switch macOS
        // actually binds — NOT the Windows Win+Ctrl+Left).
        let remapped = GestureMap {
            swipe4_down: GestureAction::DesktopLeft,
            ..GestureMap::default()
        };
        assert_eq!(
            remapped.sequence_for(GestureId::Swipe4Down, "macos"),
            vec![
                step(0xA2, true),
                step(0x25, true),
                step(0x25, false),
                step(0xA2, false)
            ]
        );
        // ShowDesktop is honestly unmapped on macOS (F11 is volume down by
        // default; the old Win+D delegation would arrive as Cmd+D).
        let remapped = GestureMap {
            swipe4_down: GestureAction::ShowDesktop,
            ..GestureMap::default()
        };
        assert!(remapped
            .sequence_for(GestureId::Swipe4Down, "macos")
            .is_empty());
    }

    #[test]
    fn json_roundtrip_preserves_table() {
        let m = GestureMap::default();
        let s = serde_json::to_string(&m).unwrap();
        let back: GestureMap = serde_json::from_str(&s).unwrap();
        assert_eq!(m, back);
        // serde renames are the storage format — snake_case strings.
        assert!(s.contains("app_switch_next"));
    }

    /// A gesture_map.json written before `app_switch_mode` existed (v0.2.78
    /// and earlier) has no such field — it must deserialize to the
    /// `per_swipe` default, not fail, so an upgrade never breaks an
    /// existing config.
    #[test]
    fn legacy_map_without_mode_defaults_to_per_swipe() {
        let legacy = r#"{"swipe4_left":"app_switch_next","swipe4_right":"app_switch_prev","swipe4_up":"overview_toggle","swipe4_down":"none"}"#;
        let m: GestureMap = serde_json::from_str(legacy).unwrap();
        assert_eq!(m.app_switch_mode, AppSwitchMode::PerSwipe);
        assert_eq!(m, GestureMap::default());
    }

    /// The mode round-trips through the storage form and unknown values are
    /// rejected (an operator typo must not silently fall back).
    #[test]
    fn app_switch_mode_roundtrip_and_unknown_rejected() {
        assert_eq!(
            AppSwitchMode::parse("per_swipe"),
            Some(AppSwitchMode::PerSwipe)
        );
        assert_eq!(
            AppSwitchMode::parse("sticky_chord"),
            Some(AppSwitchMode::StickyChord)
        );
        assert_eq!(AppSwitchMode::parse("hold_alt"), None);
        let m = GestureMap {
            app_switch_mode: AppSwitchMode::StickyChord,
            ..GestureMap::default()
        };
        let s = serde_json::to_string(&m).unwrap();
        let back: GestureMap = serde_json::from_str(&s).unwrap();
        assert_eq!(back.app_switch_mode, AppSwitchMode::StickyChord);
        assert!(s.contains("sticky_chord"));
    }

    // ---- sticky chord (macOS continuity app switch on Windows) ----

    fn chord(code: u16, down: bool) -> GestureKeyStep {
        GestureKeyStep { code, down }
    }

    /// First swipe of a chord: Alt down HELD, Tab tap; state active.
    #[test]
    fn sticky_chord_first_swipe_holds_alt() {
        let mut state = StickyChordState::default();
        let (steps, advanced) = sticky_chord_steps(&mut state, GestureAction::AppSwitchNext, 1_000);
        assert!(advanced);
        assert_eq!(
            steps,
            vec![
                chord(vk::LMENU, true),
                chord(vk::TAB, true),
                chord(vk::TAB, false)
            ]
        );
        assert!(state.is_active());
        assert_eq!(state.last_swipe_ms, 1_000);
    }

    /// Extending swipe: ONLY another Tab tap — the held Alt must not be
    /// re-pressed or released. This is the whole point: consecutive swipes
    /// walk the cycle instead of committing on every Alt release.
    #[test]
    fn sticky_chord_extending_swipe_is_pure_tab() {
        let mut state = StickyChordState::default();
        let _ = sticky_chord_steps(&mut state, GestureAction::AppSwitchNext, 1_000);
        let (steps, advanced) = sticky_chord_steps(&mut state, GestureAction::AppSwitchNext, 1_400);
        assert!(advanced);
        assert_eq!(steps, vec![chord(vk::TAB, true), chord(vk::TAB, false)]);
        // Still exactly Alt held — no duplicate presses, no Shift yet.
        assert_eq!(state.held, vec![chord(vk::LMENU, true)]);
    }

    /// Direction flip mid-chord: Shift goes down and STAYS down (held) so
    /// every subsequent Tab walks backwards, and the commit releases
    /// Shift-then-Alt in reverse press order.
    #[test]
    fn sticky_chord_direction_flip_holds_shift_and_releases_in_reverse() {
        let mut state = StickyChordState::default();
        let _ = sticky_chord_steps(&mut state, GestureAction::AppSwitchNext, 1_000);
        let (steps, advanced) = sticky_chord_steps(&mut state, GestureAction::AppSwitchPrev, 1_400);
        assert!(advanced);
        // Shift down + Tab tap; Alt must NOT appear again.
        assert_eq!(
            steps,
            vec![
                chord(vk::LSHIFT, true),
                chord(vk::TAB, true),
                chord(vk::TAB, false)
            ]
        );
        assert_eq!(
            state.held,
            vec![chord(vk::LMENU, true), chord(vk::LSHIFT, true)]
        );
        let ups = state.take_commit();
        assert_eq!(ups, vec![chord(vk::LSHIFT, false), chord(vk::LMENU, false)]);
        assert!(!state.is_active());
    }

    /// Direction flip is TWO-WAY: after a reverse swipe, swiping forward
    /// again must RELEASE Shift (before the Tab tap, so this tap already
    /// walks forward), keep only Alt held, and the final commit is a bare
    /// Alt-up.
    #[test]
    fn sticky_chord_flip_back_to_forward_releases_shift() {
        let mut state = StickyChordState::default();
        let _ = sticky_chord_steps(&mut state, GestureAction::AppSwitchNext, 1_000);
        let _ = sticky_chord_steps(&mut state, GestureAction::AppSwitchPrev, 1_400);
        assert_eq!(
            state.held,
            vec![chord(vk::LMENU, true), chord(vk::LSHIFT, true)]
        );
        let (steps, advanced) = sticky_chord_steps(&mut state, GestureAction::AppSwitchNext, 1_800);
        assert!(advanced);
        // Shift-up precedes the Tab tap; no duplicate Shift or Alt events.
        assert_eq!(
            steps,
            vec![
                chord(vk::LSHIFT, false),
                chord(vk::TAB, true),
                chord(vk::TAB, false)
            ]
        );
        assert_eq!(state.held, vec![chord(vk::LMENU, true)]);
        let ups = state.take_commit();
        assert_eq!(ups, vec![chord(vk::LMENU, false)]);
    }

    /// First swipe in the PREV direction: Shift+Alt both held from the start.
    #[test]
    fn sticky_chord_prev_first_swipe_holds_shift_and_alt() {
        let mut state = StickyChordState::default();
        let (steps, advanced) = sticky_chord_steps(&mut state, GestureAction::AppSwitchPrev, 1_000);
        assert!(advanced);
        assert_eq!(
            steps,
            vec![
                chord(vk::LMENU, true),
                chord(vk::LSHIFT, true),
                chord(vk::TAB, true),
                chord(vk::TAB, false)
            ]
        );
        assert_eq!(
            state.held,
            vec![chord(vk::LMENU, true), chord(vk::LSHIFT, true)]
        );
    }

    /// Non-app-switch actions do not participate: empty steps, advanced
    /// false, state untouched — the caller commits the chord first.
    #[test]
    fn sticky_chord_non_switch_actions_do_not_participate() {
        let mut state = StickyChordState::default();
        let _ = sticky_chord_steps(&mut state, GestureAction::AppSwitchNext, 1_000);
        for action in [
            GestureAction::OverviewToggle,
            GestureAction::DesktopLeft,
            GestureAction::ShowDesktop,
            GestureAction::None,
        ] {
            let (steps, advanced) = sticky_chord_steps(&mut state, action, 2_000);
            assert!(!advanced, "{action:?} must not join the chord");
            assert!(steps.is_empty());
        }
        // Untouched: still holding Alt from the first swipe, last_swipe_ms
        // unchanged.
        assert_eq!(state.held, vec![chord(vk::LMENU, true)]);
        assert_eq!(state.last_swipe_ms, 1_000);
    }

    /// §192: the wire platform string is `std::env::consts::OS` ("macos"),
    /// but the peer table can also carry "darwin" (hello override / mDNS
    /// text records). "dar**win**" contains "win", so testing "win" before
    /// "mac" misrouted macOS peers to the Windows backend — whose Alt+Tab
    /// shape has no macOS meaning and ships no Cmd+` shortcut.
    #[test]
    fn darwin_platform_routes_to_macos_backend() {
        let m = GestureMap::default();
        // Cmd+` shape (LWIN down, grave tap, LWIN up) — NOT Windows Alt+Tab.
        assert_eq!(
            m.sequence_for(GestureId::Swipe4Left, "darwin"),
            vec![
                step(0x5B, true),
                step(0xC0, true),
                step(0xC0, false),
                step(0x5B, false)
            ]
        );
        // ShowDesktop stays honestly unmapped on macOS for darwin too.
        let remapped = GestureMap {
            swipe4_down: GestureAction::ShowDesktop,
            ..GestureMap::default()
        };
        assert!(remapped
            .sequence_for(GestureId::Swipe4Down, "darwin")
            .is_empty());
    }
}
