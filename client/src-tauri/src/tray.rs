// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Tray state derivation (ADR-139 §6.2, roadmap M3): the tray icon and
//! tooltip must show REAL daemon state, not a hardcoded "running" string.
//!
//! Three icon states (§6.1): this machine controls the seat (`local`),
//! a remote peer holds the input (`remote`), sharing paused (`paused`);
//! plus `offline` while the shell's SSE link is down. The derivation is a
//! pure function so unit tests pin it without a tray.
//!
//! Frame source: the shell's single SSE connection already forwards
//! `snapshot` / `input.changed` / `system.changed` frames; this module
//! folds only what it needs.

use serde::Deserialize;

/// Tray icon state, serialized lowercase for logging/tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    /// Seat is local: input stays on this machine.
    Local,
    /// A remote peer holds the input (`seat == "remote"`).
    Remote,
    /// Sharing paused (input disabled via the pause switch).
    Paused,
    /// The daemon link is down (SSE offline).
    Offline,
}

impl TrayState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrayState::Local => "local",
            TrayState::Remote => "remote",
            TrayState::Paused => "paused",
            TrayState::Offline => "offline",
        }
    }

    /// Human tooltip line; the peer alias resolves outside (device id here).
    pub fn tooltip(&self, peer: Option<&str>) -> String {
        match self {
            TrayState::Local => "Tetherly · 键鼠在本机".into(),
            TrayState::Remote => match peer {
                Some(p) => format!("Tetherly · 键鼠在 {p}"),
                None => "Tetherly · 键鼠被远端接管".into(),
            },
            TrayState::Paused => "Tetherly · 共享已暂停".into(),
            TrayState::Offline => "Tetherly · daemon 未连接".into(),
        }
    }
}

/// The slices of the snapshot/events this module folds.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TrayFold {
    /// `input.seat` from `snapshot` / `input.changed`.
    seat: Option<String>,
    /// `settings.general.advertise` from `snapshot` / `settings.changed` —
    /// the real share switch (§6.2 "暂停"): off stops mdns advertisement so
    /// no peer can take over. NOTE `input.channel_open` is NOT the pause
    /// switch: it is false on an idle node (no control channel attached) and
    /// would render a fresh install permanently "paused".
    advertise: Option<bool>,
    /// `input.peer` (device id) while remote.
    peer: Option<String>,
}

impl TrayFold {
    /// Fold one envelope frame's bare payload; unknown event types are
    /// ignored (the tray does not need every topic).
    pub fn apply(&mut self, ty: &str, data: &serde_json::Value) {
        match ty {
            "snapshot" => {
                if let Ok(v) = serde_json::from_value::<SnapshotSlice>(data.clone()) {
                    self.seat = Some(v.input.seat);
                    self.advertise = Some(v.general.advertise);
                    self.peer = v.input.peer;
                }
            }
            "input.changed" => {
                if let Ok(v) = serde_json::from_value::<InputSlice>(data.clone()) {
                    self.seat = Some(v.seat);
                    self.peer = v.peer;
                }
            }
            "settings.changed" => {
                if let Ok(v) = serde_json::from_value::<SettingsSlice>(data.clone()) {
                    self.advertise = Some(v.general.advertise);
                }
            }
            _ => {}
        }
    }

    /// Current tray state; offline overrides everything (the fold goes
    /// stale the moment the link drops). Remote outranks paused: someone
    /// else holds the wheel regardless of advertisement.
    pub fn state(&self, link_up: bool) -> TrayState {
        if !link_up {
            return TrayState::Offline;
        }
        if self.seat.as_deref() == Some("remote") {
            return TrayState::Remote;
        }
        if self.advertise == Some(false) {
            return TrayState::Paused;
        }
        TrayState::Local
    }

    pub fn peer(&self) -> Option<&str> {
        self.peer.as_deref()
    }
}

#[derive(Deserialize)]
struct SnapshotSlice {
    input: InputSlice,
    general: GeneralSlice,
}

#[derive(Deserialize)]
struct GeneralSlice {
    advertise: bool,
}

#[derive(Deserialize)]
struct SettingsSlice {
    general: GeneralSlice,
}

#[derive(Deserialize)]
struct InputSlice {
    seat: String,
    peer: Option<String>,
}

/// The status-tint colors per tray state, as (r, g, b): green = all good on
/// this machine, amber = a peer holds the wheel, gray = paused, red = the
/// daemon link is down.
pub fn tint(state: TrayState) -> (u8, u8, u8) {
    match state {
        TrayState::Local => (94, 214, 178),
        TrayState::Remote => (255, 176, 59),
        TrayState::Paused => (120, 130, 145),
        TrayState::Offline => (170, 60, 60),
    }
}

/// The 32×32 embedded tray glyph: a base plate plus two bars (rows 13–18 and
/// 21–26, columns 4–18). Committed as a `const` so no asset file is loaded at
/// runtime; `base` pixels in the bar bands are the tint target.
pub const GLYPH: [u8; 32 * 32 * 4] = build_glyph();

const fn build_glyph() -> [u8; 32 * 32 * 4] {
    let mut px = [0u8; 32 * 32 * 4];
    // Base plate: a soft dark disc, alpha falls off at the rim. Distance is
    // compared squared — `sqrt` is not const-callable.
    let r2 = 13.0 * 13.0;
    let r2_rim = 14.5 * 14.5;
    let mut y = 0;
    while y < 32 {
        let mut x = 0;
        while x < 32 {
            let dx = x as f32 - 15.5;
            let dy = y as f32 - 15.5;
            let d2 = dx * dx + dy * dy;
            let (r, g, b, a) = if d2 <= r2 {
                (16, 42, 60, 235)
            } else if d2 <= r2_rim {
                (16, 42, 60, 140)
            } else {
                (0, 0, 0, 0)
            };
            let i = (y * 32 + x) * 4;
            px[i] = r;
            px[i + 1] = g;
            px[i + 2] = b;
            px[i + 3] = a;
            x += 1;
        }
        y += 1;
    }
    // The two bars keep the base color; [`TrayState::tinted_icon`] recolors
    // them per state at runtime.
    px
}

impl TrayState {
    /// Recolor the bar band of [`GLYPH`] with this state's [`tint`].
    ///
    /// Pure so tests can pin the tint behavior without a tray.
    pub fn tinted_icon(&self, base: &[u8]) -> tauri::image::Image<'static> {
        let mut rgba = base.to_vec();
        let (r, g, b) = tint(*self);
        // The glyph bars sit in rows 0.42–0.48 and 0.52–0.58 of the icon.
        for y in [13, 14, 15, 16, 17, 18, 21, 22, 23, 24, 25, 26] {
            for x in 4..=18 {
                let i = (y * 32 + x) * 4;
                if rgba[i] != 16 || rgba[i + 1] != 42 {
                    continue; // not a base pixel of the bar band
                }
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
            }
        }
        tauri::image::Image::new_owned(rgba, 32, 32)
    }
}

/// One desktop notification worth surfacing, derived from a folded frame.
/// The derivation is pure so tests pin WHEN the shell buzzes the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
}

/// Decide whether a forwarded frame deserves a system notification.
/// Rules (§6.2 acceptance: 系统通知):
/// - seat → remote: a peer took over the keyboard/mouse NOW;
/// - seat → local after remote: the wheel came back;
/// - `activity.changed`: new unread inbound messages / handoffs (the
///   snapshot resource carries everything, so count by arrival here).
pub fn notice_for(seat_was_remote: bool, ty: &str, data: &serde_json::Value) -> Option<Notice> {
    match ty {
        "input.changed" => {
            let seat = data.get("seat").and_then(|v| v.as_str())?;
            match (seat_was_remote, seat) {
                (false, "remote") => Some(Notice {
                    title: "Tetherly".into(),
                    body: "键鼠已被远端接管".into(),
                }),
                (true, "local") => Some(Notice {
                    title: "Tetherly".into(),
                    body: "键鼠已回到本机".into(),
                }),
                _ => None,
            }
        }
        "activity.changed" => {
            let unread_msgs = data["messages"]
                .as_array()
                .map(|m| m.iter().filter(|x| x["read"] == false).count())
                .unwrap_or(0);
            let pending_handoffs = data["handoffs"]
                .as_array()
                .map(|h| h.iter().filter(|x| x["opened_ms"].is_null()).count())
                .unwrap_or(0);
            let total = unread_msgs + pending_handoffs;
            if total == 0 {
                return None;
            }
            let mut parts = Vec::new();
            if unread_msgs > 0 {
                parts.push(format!("{unread_msgs} 条新消息"));
            }
            if pending_handoffs > 0 {
                parts.push(format!("{pending_handoffs} 个待打开的接续"));
            }
            Some(Notice {
                title: "Tetherly".into(),
                body: parts.join("，"),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn offline_overrides_stale_fold() {
        let mut f = TrayFold::default();
        f.apply("input.changed", &json!({"seat": "remote", "peer": "d1"}));
        assert_eq!(f.state(false), TrayState::Offline);
        assert_eq!(f.state(true), TrayState::Remote);
    }

    #[test]
    fn snapshot_and_input_changed_fold_alike() {
        let snap = json!({
            "system": {"name": "mac"},
            "input": {"seat": "local", "channel_open": false, "peer": null},
            "general": {"advertise": true}
        });
        let mut f = TrayFold::default();
        f.apply("snapshot", &snap);
        assert_eq!(f.state(true), TrayState::Local);
        f.apply("input.changed", &json!({"seat": "remote", "peer": "d1"}));
        assert_eq!(f.state(true), TrayState::Remote);
        assert_eq!(f.peer(), Some("d1"));
    }

    #[test]
    fn settings_changed_flips_pause_state() {
        let mut f = TrayFold::default();
        f.apply(
            "snapshot",
            &json!({
                "input": {"seat": "local", "channel_open": false, "peer": null},
                "general": {"advertise": true}
            }),
        );
        assert_eq!(f.state(true), TrayState::Local);
        f.apply(
            "settings.changed",
            &json!({"general": {"advertise": false, "name": "Desk"}}),
        );
        assert_eq!(f.state(true), TrayState::Paused);
        f.apply(
            "settings.changed",
            &json!({"general": {"advertise": true, "name": "Desk"}}),
        );
        assert_eq!(f.state(true), TrayState::Local);
    }

    #[test]
    fn idle_channel_open_is_not_paused() {
        // Regression: a fresh idle node reports channel_open=false; that is
        // the no-channel-attached normal, not the pause switch.
        let mut f = TrayFold::default();
        f.apply(
            "snapshot",
            &json!({
                "input": {"seat": "local", "channel_open": false, "peer": null},
                "general": {"advertise": true}
            }),
        );
        assert_eq!(f.state(true), TrayState::Local);
    }

    #[test]
    fn remote_outranks_paused() {
        let mut f = TrayFold::default();
        f.apply(
            "snapshot",
            &json!({
                "input": {"seat": "remote", "channel_open": true, "peer": "d2"},
                "general": {"advertise": false}
            }),
        );
        assert_eq!(f.state(true), TrayState::Remote, "someone has the wheel");
    }

    #[test]
    fn unknown_events_and_malformed_payloads_are_ignored() {
        let mut f = TrayFold::default();
        f.apply("activity.changed", &json!({"activity": {}}));
        f.apply("input.changed", &json!({"seat": 42}));
        assert_eq!(f.state(true), TrayState::Local, "no valid fold yet");
    }

    #[test]
    fn tooltips_cover_all_states_with_peer_name() {
        assert_eq!(TrayState::Local.tooltip(None), "Tetherly · 键鼠在本机");
        assert_eq!(
            TrayState::Remote.tooltip(Some("Windows")),
            "Tetherly · 键鼠在 Windows"
        );
        assert_eq!(TrayState::Remote.tooltip(None), "Tetherly · 键鼠被远端接管");
        assert_eq!(TrayState::Paused.tooltip(None), "Tetherly · 共享已暂停");
        assert_eq!(TrayState::Offline.tooltip(None), "Tetherly · daemon 未连接");
    }

    #[test]
    fn state_names_are_stable() {
        assert_eq!(TrayState::Local.as_str(), "local");
        assert_eq!(TrayState::Remote.as_str(), "remote");
        assert_eq!(TrayState::Paused.as_str(), "paused");
        assert_eq!(TrayState::Offline.as_str(), "offline");
    }

    #[test]
    fn glyph_is_32x32_rgba_and_opaque_in_bars() {
        assert_eq!(GLYPH.len(), 32 * 32 * 4);
        // The bar band pixels the tint targets exist and are the base color.
        let i = (14 * 32 + 10) * 4;
        assert_eq!((GLYPH[i], GLYPH[i + 1], GLYPH[i + 2]), (16, 42, 60));
        assert!(GLYPH[i + 3] > 0, "bar pixel is visible");
        // Outside the icon the plate is transparent.
        assert_eq!(GLYPH[3], 0);
    }

    #[test]
    fn tint_colors_are_distinct_per_state() {
        let ts = [
            tint(TrayState::Local),
            tint(TrayState::Remote),
            tint(TrayState::Paused),
            tint(TrayState::Offline),
        ];
        for (a, b) in ts.iter().zip(ts.iter().skip(1)) {
            assert_ne!(a, b, "every state needs a distinguishable tint");
        }
    }

    #[test]
    fn tinted_icon_recovers_state_color_in_bars() {
        let icon = TrayState::Remote.tinted_icon(&GLYPH);
        let rgba = icon.rgba();
        let i = (14 * 32 + 10) * 4;
        assert_eq!((rgba[i], rgba[i + 1], rgba[i + 2]), tint(TrayState::Remote));
        // A non-bar pixel keeps the base color.
        let j = (5 * 32 + 10) * 4;
        assert_eq!(rgba[j], 16);
    }

    #[test]
    fn takeover_and_return_seat_changes_notify() {
        let n = notice_for(
            false,
            "input.changed",
            &json!({"seat": "remote", "peer": "d1"}),
        )
        .expect("takeover must notify");
        assert_eq!(n.body, "键鼠已被远端接管");
        let n = notice_for(
            true,
            "input.changed",
            &json!({"seat": "local", "peer": null}),
        )
        .expect("return must notify");
        assert_eq!(n.body, "键鼠已回到本机");
    }

    #[test]
    fn local_seat_transitions_do_not_notify() {
        assert!(notice_for(
            false,
            "input.changed",
            &json!({"seat": "local", "peer": null})
        )
        .is_none());
        assert!(notice_for(
            true,
            "input.changed",
            &json!({"seat": "remote", "peer": "d"})
        )
        .is_none());
    }

    #[test]
    fn unread_activity_notifies_only_when_new() {
        let data = json!({
            "messages": [
                {"id": "m1", "read": false, "text": "hi"},
                {"id": "m2", "read": true, "text": "old"}
            ],
            "handoffs": [{"id": "h1", "opened_ms": null, "url": "https://x"}]
        });
        let n = notice_for(false, "activity.changed", &data).expect("unread exists");
        assert_eq!(n.body, "1 条新消息，1 个待打开的接续");
        // Everything read/opened: silent.
        let quiet = json!({
            "messages": [{"id": "m1", "read": true}],
            "handoffs": [{"id": "h1", "opened_ms": 123}]
        });
        assert!(notice_for(false, "activity.changed", &quiet).is_none());
        // Unrelated events never notify.
        assert!(notice_for(false, "transfers.changed", &data).is_none());
    }
}
