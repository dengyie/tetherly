// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Cross-platform modifier and virtual-key remapping engine.
//!
//! Protocol-pure and OS-free: maps virtual-key codes (Win32 VK over the wire)
//! and modifier flags between different operating system conventions
//! (e.g., macOS Command ↔ Windows Control, Option ↔ Alt).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Standard Win32 Virtual-Key codes for modifier keys.
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12; // Alt
pub const VK_CAPITAL: u16 = 0x14; // Caps Lock
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_LSHIFT: u16 = 0xA0;
pub const VK_RSHIFT: u16 = 0xA1;
pub const VK_LCONTROL: u16 = 0xA2;
pub const VK_RCONTROL: u16 = 0xA3;
pub const VK_LMENU: u16 = 0xA4; // Left Alt
pub const VK_RMENU: u16 = 0xA5; // Right Alt

/// Modifier bitflags matching wire/input conventions.
pub const MOD_SHIFT: u8 = 0x01;
pub const MOD_CONTROL: u8 = 0x02;
pub const MOD_ALT: u8 = 0x04;
pub const MOD_META: u8 = 0x08;

/// Generic modifier key categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModifierKey {
    Shift,
    Control,
    Alt,  // Option on macOS
    Meta, // Command on macOS, Windows Key on Windows
}

/// Modifier key lateral side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySide {
    Any,
    Left,
    Right,
}

impl ModifierKey {
    /// Return the canonical modifier bit for this modifier category.
    pub const fn flag_bit(self) -> u8 {
        match self {
            ModifierKey::Shift => MOD_SHIFT,
            ModifierKey::Control => MOD_CONTROL,
            ModifierKey::Alt => MOD_ALT,
            ModifierKey::Meta => MOD_META,
        }
    }
}

/// Identify if a Win32 virtual-key code is a modifier key and which side it represents.
pub fn modifier_for_vk(code: u16) -> Option<(ModifierKey, KeySide)> {
    match code {
        VK_SHIFT => Some((ModifierKey::Shift, KeySide::Any)),
        VK_LSHIFT => Some((ModifierKey::Shift, KeySide::Left)),
        VK_RSHIFT => Some((ModifierKey::Shift, KeySide::Right)),

        VK_CONTROL => Some((ModifierKey::Control, KeySide::Any)),
        VK_LCONTROL => Some((ModifierKey::Control, KeySide::Left)),
        VK_RCONTROL => Some((ModifierKey::Control, KeySide::Right)),

        VK_MENU => Some((ModifierKey::Alt, KeySide::Any)),
        VK_LMENU => Some((ModifierKey::Alt, KeySide::Left)),
        VK_RMENU => Some((ModifierKey::Alt, KeySide::Right)),

        VK_LWIN => Some((ModifierKey::Meta, KeySide::Left)),
        VK_RWIN => Some((ModifierKey::Meta, KeySide::Right)),

        _ => None,
    }
}

/// Get the corresponding Win32 virtual-key code for a modifier and side.
pub fn vk_for_modifier(modifier: ModifierKey, side: KeySide) -> u16 {
    match (modifier, side) {
        (ModifierKey::Shift, KeySide::Left) => VK_LSHIFT,
        (ModifierKey::Shift, KeySide::Right) => VK_RSHIFT,
        (ModifierKey::Shift, KeySide::Any) => VK_SHIFT,

        (ModifierKey::Control, KeySide::Left) => VK_LCONTROL,
        (ModifierKey::Control, KeySide::Right) => VK_RCONTROL,
        (ModifierKey::Control, KeySide::Any) => VK_CONTROL,

        (ModifierKey::Alt, KeySide::Left) => VK_LMENU,
        (ModifierKey::Alt, KeySide::Right) => VK_RMENU,
        (ModifierKey::Alt, KeySide::Any) => VK_MENU,

        (ModifierKey::Meta, KeySide::Left | KeySide::Any) => VK_LWIN,
        (ModifierKey::Meta, KeySide::Right) => VK_RWIN,
    }
}

/// Key remapping profile: specifies modifier substitutions and custom key mappings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyMapProfile {
    /// Identifier or preset name.
    pub name: String,
    /// Modifier translation table.
    pub modifiers: HashMap<ModifierKey, ModifierKey>,
    /// Exact VK-to-VK remappings (takes precedence over modifier table).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom_keys: HashMap<u16, u16>,
}

impl Default for KeyMapProfile {
    fn default() -> Self {
        Self::identity()
    }
}

impl KeyMapProfile {
    /// Identity profile: no remapping, keys pass through unchanged.
    pub fn identity() -> Self {
        Self {
            name: "identity".to_string(),
            modifiers: HashMap::new(),
            custom_keys: HashMap::new(),
        }
    }

    /// macOS host controlling a Windows client:
    /// - macOS Command (Meta) -> Windows Control (Ctrl)
    /// - macOS Control -> Windows Meta (Win)
    /// - macOS Option (Alt) -> Windows Alt
    /// - Shift -> Shift
    pub fn mac_to_windows() -> Self {
        let mut modifiers = HashMap::new();
        modifiers.insert(ModifierKey::Meta, ModifierKey::Control);
        modifiers.insert(ModifierKey::Control, ModifierKey::Meta);
        modifiers.insert(ModifierKey::Alt, ModifierKey::Alt);
        modifiers.insert(ModifierKey::Shift, ModifierKey::Shift);
        Self {
            name: "mac_to_windows".to_string(),
            modifiers,
            custom_keys: HashMap::new(),
        }
    }

    /// Windows host controlling a macOS client:
    /// - Windows Control (Ctrl) -> macOS Command (Meta)
    /// - Windows Meta (Win) -> macOS Control
    /// - Windows Alt -> macOS Option (Alt)
    /// - Shift -> Shift
    pub fn windows_to_mac() -> Self {
        let mut modifiers = HashMap::new();
        modifiers.insert(ModifierKey::Control, ModifierKey::Meta);
        modifiers.insert(ModifierKey::Meta, ModifierKey::Control);
        modifiers.insert(ModifierKey::Alt, ModifierKey::Alt);
        modifiers.insert(ModifierKey::Shift, ModifierKey::Shift);
        Self {
            name: "windows_to_mac".to_string(),
            modifiers,
            custom_keys: HashMap::new(),
        }
    }

    /// Parse profile from preset name string (case-insensitive, normalized).
    pub fn from_preset(preset: &str) -> Option<Self> {
        let norm = preset.trim().to_lowercase().replace('-', "_");
        match norm.as_str() {
            "identity" | "none" | "off" | "passthrough" => Some(Self::identity()),
            "mac_to_windows" | "mac_to_win" | "m2w" | "mac2win" => Some(Self::mac_to_windows()),
            "windows_to_mac" | "win_to_mac" | "w2m" | "win2mac" => Some(Self::windows_to_mac()),
            _ => None,
        }
    }

    /// Remap a Win32 virtual-key code according to this profile.
    pub fn remap_vk(&self, code: u16) -> u16 {
        // 1. Check custom VK overrides first
        if let Some(&remapped) = self.custom_keys.get(&code) {
            return remapped;
        }

        // 2. Check modifier mapping
        if let Some((mod_key, side)) = modifier_for_vk(code) {
            if let Some(&target_mod) = self.modifiers.get(&mod_key) {
                return vk_for_modifier(target_mod, side);
            }
        }

        // 3. Fall through unchanged
        code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_preserves_keys_and_mods() {
        let p = KeyMapProfile::identity();
        assert_eq!(p.remap_vk(VK_LWIN), VK_LWIN);
        assert_eq!(p.remap_vk(VK_LCONTROL), VK_LCONTROL);
        assert_eq!(p.remap_vk(0x41), 0x41); // 'A'
    }

    #[test]
    fn mac_to_windows_maps_meta_to_control() {
        let p = KeyMapProfile::mac_to_windows();
        // Left Command -> Left Control
        assert_eq!(p.remap_vk(VK_LWIN), VK_LCONTROL);
        // Right Command -> Right Control
        assert_eq!(p.remap_vk(VK_RWIN), VK_RCONTROL);

        // Control -> Meta (Win)
        assert_eq!(p.remap_vk(VK_CONTROL), VK_LWIN);
        assert_eq!(p.remap_vk(VK_LCONTROL), VK_LWIN);
        assert_eq!(p.remap_vk(VK_RCONTROL), VK_RWIN);

        // Alt stays Alt
        assert_eq!(p.remap_vk(VK_LMENU), VK_LMENU);
        assert_eq!(p.remap_vk(VK_RMENU), VK_RMENU);

        // Shift stays Shift
        assert_eq!(p.remap_vk(VK_LSHIFT), VK_LSHIFT);

        // Non-modifier keys are unchanged
        assert_eq!(p.remap_vk(0x41), 0x41); // 'A'
        assert_eq!(p.remap_vk(0x20), 0x20); // Space
    }

    #[test]
    fn windows_to_mac_maps_control_to_meta() {
        let p = KeyMapProfile::windows_to_mac();
        // Control -> Meta (Command)
        assert_eq!(p.remap_vk(VK_LCONTROL), VK_LWIN);
        assert_eq!(p.remap_vk(VK_RCONTROL), VK_RWIN);

        // Win -> Control
        assert_eq!(p.remap_vk(VK_LWIN), VK_LCONTROL);
        assert_eq!(p.remap_vk(VK_RWIN), VK_RCONTROL);
    }

    #[test]
    fn custom_keys_override_modifiers() {
        let mut p = KeyMapProfile::mac_to_windows();
        p.custom_keys.insert(VK_LWIN, 0x1234);
        assert_eq!(p.remap_vk(VK_LWIN), 0x1234);
        // VK_RWIN still maps to VK_RCONTROL
        assert_eq!(p.remap_vk(VK_RWIN), VK_RCONTROL);
    }

    #[test]
    fn serde_roundtrip() {
        let p = KeyMapProfile::mac_to_windows();
        let json = serde_json::to_string(&p).unwrap();
        let decoded: KeyMapProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(p, decoded);
    }

    #[test]
    fn preset_parsing() {
        assert_eq!(
            KeyMapProfile::from_preset("mac-to-windows").unwrap(),
            KeyMapProfile::mac_to_windows()
        );
        assert_eq!(
            KeyMapProfile::from_preset("m2w").unwrap(),
            KeyMapProfile::mac_to_windows()
        );
        assert_eq!(
            KeyMapProfile::from_preset("windows_to_mac").unwrap(),
            KeyMapProfile::windows_to_mac()
        );
        assert_eq!(
            KeyMapProfile::from_preset("w2m").unwrap(),
            KeyMapProfile::windows_to_mac()
        );
        assert_eq!(
            KeyMapProfile::from_preset("none").unwrap(),
            KeyMapProfile::identity()
        );
        assert!(KeyMapProfile::from_preset("unknown_preset").is_none());
    }
}
