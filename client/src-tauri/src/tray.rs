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
                    if let Some(g) = v.general {
                        self.advertise = Some(g.advertise);
                    }
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
    /// The v1 `Snapshot` is `{system, devices, pairing, input}` — it carries
    /// no settings section, so this is absent on the real wire and only
    /// present on richer mock payloads. Requiring it made the whole snapshot
    /// fail to deserialize and left the tray fold empty until the first
    /// `input.changed`.
    #[serde(default)]
    general: Option<GeneralSlice>,
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

/// The 32×32 embedded tray glyph: the mango brand mark
/// (`assets/brand/tray-32x32.rgba`, straight-alpha RGBA, top-down), copied
/// into a `const` so no asset file is loaded at runtime.
/// [`TrayState::tinted_icon`] paints the state badge over its corner.
pub const GLYPH: [u8; 32 * 32 * 4] = *include_bytes!("../../../assets/brand/tray-32x32.rgba");

/// State badge geometry: centre and radius (pixels) of the dot painted by
/// [`TrayState::tinted_icon`]. A 1px dark ring keeps it legible on both light
/// and dark menu bars.
pub const BADGE_CENTER: (f32, f32) = (24.5, 24.5);
pub const BADGE_RADIUS: f32 = 6.0;
const BADGE_RING: (u8, u8, u8) = (16, 42, 60);

impl TrayState {
    /// Paint this state's [`tint`] badge over the bottom-right corner of
    /// `base`; every pixel outside the badge is left as the brand mark.
    ///
    /// Pure so tests can pin the badge behavior without a tray.
    pub fn tinted_icon(&self, base: &[u8]) -> tauri::image::Image<'static> {
        let mut rgba = base.to_vec();
        let (r, g, b) = tint(*self);
        let (cx, cy) = BADGE_CENTER;
        let dot2 = BADGE_RADIUS * BADGE_RADIUS;
        let ring2 = (BADGE_RADIUS + 1.0) * (BADGE_RADIUS + 1.0);
        for y in 0..32usize {
            for x in 0..32usize {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let d2 = dx * dx + dy * dy;
                let i = (y * 32 + x) * 4;
                if d2 <= dot2 {
                    rgba[i..i + 4].copy_from_slice(&[r, g, b, 255]);
                } else if d2 <= ring2 {
                    rgba[i..i + 4].copy_from_slice(&[
                        BADGE_RING.0,
                        BADGE_RING.1,
                        BADGE_RING.2,
                        255,
                    ]);
                }
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

/// Identity-based notification dedup for `activity.changed` (§177).
///
/// The daemon publishes the FULL Activity resource on every Activity-topic
/// change (message sent/received, handoff dispatched, candidate expiry …),
/// so a plain "unread > 0" test re-buzzes the SAME unread message on every
/// unrelated frame for as long as the user leaves it unread. The gate
/// remembers which item identities have already been surfaced; only a
/// never-seen-before unread message / un-opened handoff notifies again.
/// Reconnecting resets nothing deliberately: identities are stable.
#[derive(Debug, Default)]
pub struct NoticeSeen {
    seen: std::collections::HashSet<String>,
}

impl NoticeSeen {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Decide whether a forwarded frame deserves a system notification.
/// Rules (§6.2 acceptance: 系统通知):
/// - seat → remote: a peer took over the keyboard/mouse NOW;
/// - seat → local after remote: the wheel came back;
/// - `activity.changed`: new unread inbound messages / handoffs — gated by
///   [`NoticeSeen`] so a full-section republication never re-buzzes items
///   the user already had surfaced (§177).
pub fn notice_for(
    seat_was_remote: bool,
    ty: &str,
    data: &serde_json::Value,
    seen: &mut NoticeSeen,
) -> Option<Notice> {
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
            // Collect identities of items worth surfacing THIS frame. Ids
            // may be absent in degenerate payloads: fall back to content
            // identity (text/url) so malformed entries still dedup by value.
            let id_of = |x: &serde_json::Value, alt: &str| -> String {
                match x["id"].as_str() {
                    Some(id) => format!("{alt}:{id}"),
                    None => format!("{alt}:~{}", x[alt]),
                }
            };
            let mut fresh_msgs: Vec<&serde_json::Value> = Vec::new();
            for m in data["messages"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                if m["read"] == false {
                    let key = id_of(m, "msg");
                    if seen.seen.insert(key) {
                        fresh_msgs.push(m);
                    }
                }
            }
            let mut fresh_handoffs: Vec<&serde_json::Value> = Vec::new();
            for h in data["handoffs"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                if h["opened_ms"].is_null() {
                    let key = id_of(h, "hand");
                    if seen.seen.insert(key) {
                        fresh_handoffs.push(h);
                    }
                }
            }
            if fresh_msgs.is_empty() && fresh_handoffs.is_empty() {
                return None;
            }
            let mut parts = Vec::new();
            if !fresh_msgs.is_empty() {
                parts.push(format!("{} 条新消息", fresh_msgs.len()));
            }
            if !fresh_handoffs.is_empty() {
                parts.push(format!("{} 个待打开的接续", fresh_handoffs.len()));
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
    fn real_snapshot_without_general_still_folds_the_seat() {
        // Live v1 snapshot shape: no `general` section at all.
        let mut f = TrayFold::default();
        f.apply(
            "snapshot",
            &json!({
                "system": {"name": "mac"},
                "devices": [],
                "pairing": {"mode": "idle"},
                "input": {"seat": "remote", "channel_open": true, "peer": "d9"}
            }),
        );
        assert_eq!(f.state(true), TrayState::Remote);
        assert_eq!(f.peer(), Some("d9"));
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
    fn glyph_is_32x32_rgba_brand_mark() {
        assert_eq!(GLYPH.len(), 32 * 32 * 4);
        // Outside the mark the icon is transparent …
        assert_eq!(GLYPH[3], 0);
        // … and the mark itself is opaque where the brand asset draws.
        let i = (8 * 32 + 10) * 4;
        assert!(GLYPH[i + 3] > 240, "brand pixel is visible");
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
    fn tinted_icon_paints_state_badge_and_keeps_mark() {
        let icon = TrayState::Remote.tinted_icon(&GLYPH);
        let rgba = icon.rgba();
        // Badge centre carries the state tint, fully opaque.
        let i = (24 * 32 + 24) * 4;
        let (r, g, b) = tint(TrayState::Remote);
        assert_eq!(
            (rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]),
            (r, g, b, 255)
        );
        // The ring just outside the dot is dark so the badge reads on any bar.
        let k = (24 * 32 + 31) * 4;
        assert_eq!((rgba[k], rgba[k + 1], rgba[k + 2]), BADGE_RING);
        // A pixel far from the badge is the untouched brand mark.
        let j = (5 * 32 + 10) * 4;
        assert_eq!(&rgba[j..j + 4], &GLYPH[j..j + 4]);
    }

    #[test]
    fn takeover_and_return_seat_changes_notify() {
        let mut seen = NoticeSeen::new();
        let n = notice_for(
            false,
            "input.changed",
            &json!({"seat": "remote", "peer": "d1"}),
            &mut seen,
        )
        .expect("takeover must notify");
        assert_eq!(n.body, "键鼠已被远端接管");
        let n = notice_for(
            true,
            "input.changed",
            &json!({"seat": "local", "peer": null}),
            &mut seen,
        )
        .expect("return must notify");
        assert_eq!(n.body, "键鼠已回到本机");
    }

    #[test]
    fn local_seat_transitions_do_not_notify() {
        let mut seen = NoticeSeen::new();
        assert!(notice_for(
            false,
            "input.changed",
            &json!({"seat": "local", "peer": null}),
            &mut seen
        )
        .is_none());
        assert!(notice_for(
            true,
            "input.changed",
            &json!({"seat": "remote", "peer": "d"}),
            &mut seen
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
        let mut seen = NoticeSeen::new();
        let n = notice_for(false, "activity.changed", &data, &mut seen).expect("unread exists");
        assert_eq!(n.body, "1 条新消息，1 个待打开的接续");
        // Everything read/opened: silent.
        let quiet = json!({
            "messages": [{"id": "m1", "read": true}],
            "handoffs": [{"id": "h1", "opened_ms": 123}]
        });
        assert!(notice_for(false, "activity.changed", &quiet, &mut seen).is_none());
        // Unrelated events never notify.
        assert!(notice_for(false, "transfers.changed", &data, &mut seen).is_none());
    }

    /// §177: the daemon republishes the FULL Activity resource on every
    /// Activity-topic change. Without identity dedup, an unread message the
    /// user leaves alone re-buzzed on each unrelated frame (per-frame count
    /// was stateless). The seen-set must collapse the republish to silence
    /// and surface only genuinely NEW identities.
    #[test]
    fn activity_republish_does_not_rebuzz_known_items() {
        let frame = |extra: serde_json::Value| {
            json!({
                "messages": [
                    {"id": "m1", "read": false, "text": "hi"},
                    extra
                ],
                "handoffs": [{"id": "h1", "opened_ms": null, "url": "https://x"}]
            })
        };
        let mut seen = NoticeSeen::new();
        // First frame: both items surface.
        let n = notice_for(
            false,
            "activity.changed",
            &frame(json!({"id": "mX", "read": true})),
            &mut seen,
        )
        .expect("first frame notifies");
        assert_eq!(n.body, "1 条新消息，1 个待打开的接续");
        // Same identities republished on an unrelated topic change (e.g. a
        // message SENT elsewhere): silent.
        assert!(
            notice_for(
                false,
                "activity.changed",
                &frame(json!({"id": "mX", "read": true})),
                &mut seen
            )
            .is_none(),
            "republished identical unread identities must not re-buzz"
        );
        // A genuinely NEW unread message notifies — with only the new count.
        let n = notice_for(
            false,
            "activity.changed",
            &frame(json!({"id": "m2", "read": false})),
            &mut seen,
        )
        .expect("new message notifies");
        assert_eq!(n.body, "1 条新消息");
        // And it too stays silent from then on.
        assert!(notice_for(
            false,
            "activity.changed",
            &frame(json!({"id": "m2", "read": false})),
            &mut seen
        )
        .is_none());
        // A handoff opened then a NEW one arriving: only the new one counts.
        let open1 = json!({
            "messages": [{"id": "m1", "read": false}],
            "handoffs": [{"id": "h1", "opened_ms": 999, "url": "u"}]
        });
        assert!(notice_for(false, "activity.changed", &open1, &mut seen).is_none());
        let new_hand = json!({
            "messages": [{"id": "m1", "read": false}],
            "handoffs": [
                {"id": "h1", "opened_ms": 999, "url": "u"},
                {"id": "h2", "opened_ms": null, "url": "u2"}
            ]
        });
        let n = notice_for(false, "activity.changed", &new_hand, &mut seen)
            .expect("new handoff notifies");
        assert_eq!(n.body, "1 个待打开的接续");
    }
}
