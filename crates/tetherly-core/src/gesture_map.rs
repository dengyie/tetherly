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
    pub const ESCAPE: u16 = 0x1B;
    pub const LWIN: u16 = 0x5B;
    pub const LSHIFT: u16 = 0xA0;
    pub const LMENU: u16 = 0xA4;
    pub const LCONTROL: u16 = 0xA2;
    pub const D: u16 = 0x44;
    pub const LEFT: u16 = 0x25;
    pub const RIGHT: u16 = 0x27;
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
/// direct window-manager handling (Win+Tab, Alt+Esc, Win+Ctrl+Arrow,
/// Win+D) consume synthetic input reliably; the Alt+Tab overlay state
/// machine is the one known-unreliable surface and is avoided.
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
        // Alt+Esc: window-manager handled, no overlay.
        GestureAction::AppSwitchNext => tap(&[vk::LMENU], &[vk::ESCAPE]),
        // Shift+Alt+Esc: the reverse cycle.
        GestureAction::AppSwitchPrev => tap(&[vk::LSHIFT, vk::LMENU], &[vk::ESCAPE]),
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
/// macOS targets receive re-interpreted system shortcuts via the ordinary
/// key transport; QC widgets and Spotlight equivalents are reachable, but
/// Mission Control itself is not a hotkey-injectable surface on modern
/// macOS, so the overview maps to its keyboard surrogate.
pub fn macos_backend(action: GestureAction) -> Vec<GestureKeyStep> {
    // VK codes here are the wire convention (Win32 VK) — the target's
    // platform layer translates them to mac keycodes on arrival.
    const VK_CONTROL: u16 = 0xA2;
    const VK_UP: u16 = 0x26;
    match action {
        // Cmd+` (backtick): cycle windows of the frontmost app...
        GestureAction::AppSwitchNext => windows_backend(GestureAction::AppSwitchNext),
        GestureAction::AppSwitchPrev => windows_backend(GestureAction::AppSwitchPrev),
        // Ctrl+Up: Mission Control keyboard surrogate.
        GestureAction::OverviewToggle => tap_mods_macos(&[VK_CONTROL], &[VK_UP]),
        // Ctrl+Left/Right: Space switch.
        GestureAction::DesktopLeft => windows_backend(GestureAction::DesktopLeft),
        GestureAction::DesktopRight => windows_backend(GestureAction::DesktopRight),
        // F11 surrogate via Win+D keeps parity; HIDE is preferable but not
        // a VK-reachable hotkey — keep D and let the target's keymap handle it.
        GestureAction::ShowDesktop => windows_backend(GestureAction::ShowDesktop),
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
    pub fn sequence_for(&self, gesture: GestureId, target_platform: &str) -> Vec<GestureKeyStep> {
        let action = self.action_for(gesture);
        if target_platform.contains("win") {
            windows_backend(action)
        } else if target_platform.contains("mac") {
            macos_backend(action)
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
    fn win_sequences_match_v023_hardcoded_behavior() {
        let m = GestureMap::default();
        // Left swipe == old direction<0: Alt down, Esc tap, Alt up.
        assert_eq!(
            m.sequence_for(GestureId::Swipe4Left, "windows"),
            vec![
                GestureKeyStep {
                    code: 0xA4,
                    down: true
                },
                GestureKeyStep {
                    code: 0x1B,
                    down: true
                },
                GestureKeyStep {
                    code: 0x1B,
                    down: false
                },
                GestureKeyStep {
                    code: 0xA4,
                    down: false
                },
            ]
        );
        // Right swipe == old direction>0: Shift+Alt down, Esc tap, ups.
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
                    code: 0x1B,
                    down: true
                },
                GestureKeyStep {
                    code: 0x1B,
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

    #[test]
    fn json_roundtrip_preserves_table() {
        let m = GestureMap::default();
        let s = serde_json::to_string(&m).unwrap();
        let back: GestureMap = serde_json::from_str(&s).unwrap();
        assert_eq!(m, back);
        // serde renames are the storage format — snake_case strings.
        assert!(s.contains("app_switch_next"));
    }
}
