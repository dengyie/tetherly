// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Tray icon runtime: creation, real-state sync, and the menu actions.
//!
//! The icon mirrors what the event stream says (see [`tray::TrayFold`] /
//! [`tray::TrayState`]): keyboard-Local / Remote / Paused / Offline. Menu
//! items show the live peer line and let the user toggle sharing, open the
//! main window, or quit — no hardcoded "running" status (§1 problem list).
//!
//! macOS note: the template icon variant is used so the system renders it
//! correctly on light and dark menu bars.

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime,
};
use tauri_plugin_autostart::ManagerExt as _;

use crate::tray::{TrayFold, TrayState};

pub const MENU_TOGGLE: &str = "toggle-share";
pub const MENU_OPEN: &str = "open-main";
pub const MENU_AUTOSTART: &str = "toggle-autostart";
pub const MENU_QUIT: &str = "quit";
/// Payload channel the tray uses to tell the app "user clicked pause/resume".
pub const TOGGLE_REQUEST: &str = "tetherly://toggle-share";

/// Everything the tray needs to re-render on a state change.
pub struct TrayUi<R: Runtime> {
    icon_local: tauri::image::Image<'static>,
    icon_remote: tauri::image::Image<'static>,
    icon_paused: tauri::image::Image<'static>,
    icon_offline: tauri::image::Image<'static>,
    icon: TrayIcon<R>,
    status_item: MenuItem<R>,
    toggle_item: MenuItem<R>,
    fold: Mutex<TrayFold>,
}

impl<R: Runtime> TrayUi<R> {
    /// Build the tray with the menu and icon set; initial state is offline
    /// until the first snapshot lands.
    pub fn build(app: &AppHandle<R>) -> tauri::Result<Self> {
        let status = MenuItem::with_id(app, "status", "daemon 未连接", false, None::<&str>)?;
        let toggle = MenuItem::with_id(app, MENU_TOGGLE, "暂停共享", true, None::<&str>)?;
        let open = MenuItem::with_id(app, MENU_OPEN, "打开主窗口", true, None::<&str>)?;
        // 自启 (§6.3): label reflects the REAL OS state, refreshed on build.
        let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
        let autostart = MenuItem::with_id(
            app,
            MENU_AUTOSTART,
            if autostart_on {
                "取消开机自启"
            } else {
                "开机自启"
            },
            true,
            None::<&str>,
        )?;
        let quit = MenuItem::with_id(app, MENU_QUIT, "退出 Tetherly", true, None::<&str>)?;
        let menu = Menu::with_items(
            app,
            &[
                &status,
                &PredefinedMenuItem::separator(app)?,
                &toggle,
                &open,
                &autostart,
                &PredefinedMenuItem::separator(app)?,
                &quit,
            ],
        )?;

        let icon = TrayIconBuilder::with_id("main")
            .icon(icon_for(app, TrayState::Offline)?)
            .menu(&menu)
            .show_menu_on_left_click(false)
            .tooltip(TrayState::Offline.tooltip(None))
            .on_menu_event(|app, event| {
                if event.id() == MENU_TOGGLE {
                    let _ = app.emit(TOGGLE_REQUEST, ());
                } else if event.id() == MENU_OPEN {
                    show_main_window(app);
                } else if event.id() == MENU_AUTOSTART {
                    toggle_autostart(app);
                } else if event.id() == MENU_QUIT {
                    app.exit(0);
                }
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button,
                    button_state,
                    position,
                    ..
                } = event
                {
                    if button == tauri::tray::MouseButton::Left
                        && button_state == tauri::tray::MouseButtonState::Up
                    {
                        // Left click toggles the panel popover (§6.3); the
                        // main window stays behind the 打开主窗口 menu item.
                        toggle_panel(tray.app_handle(), &position);
                    }
                }
            })
            .build(app)?;

        Ok(Self {
            icon_local: icon_for(app, TrayState::Local)?,
            icon_remote: icon_for(app, TrayState::Remote)?,
            icon_paused: icon_for(app, TrayState::Paused)?,
            icon_offline: icon_for(app, TrayState::Offline)?,
            icon,
            status_item: status,
            toggle_item: toggle,
            fold: Mutex::new(TrayFold::default()),
        })
    }

    /// Fold one forwarded event frame (bare payload, §4.3) and re-render if
    /// anything changed. `link_up` comes from the SSE connection state.
    /// Returns whether the seat WAS remote before this frame — the caller
    /// needs the old value to classify takeover vs. return notifications.
    pub fn on_event(&self, link_up: bool, ty: &str, data: &serde_json::Value) -> bool {
        let was_remote = {
            let fold = self.fold.lock().expect("tray fold");
            fold.state(link_up) == TrayState::Remote
        };
        {
            let mut fold = self.fold.lock().expect("tray fold");
            fold.apply(ty, data);
        }
        self.render(link_up);
        was_remote
    }

    /// Whether the CURRENT folded state is remote (a peer holds the wheel).
    pub fn is_remote(&self) -> bool {
        let fold = self.fold.lock().expect("tray fold");
        fold.state(true) == TrayState::Remote
    }

    /// The SSE link changed state; offline masks a stale fold.
    pub fn on_link(&self, up: bool) {
        self.render(up);
    }

    fn render(&self, link_up: bool) {
        let fold = self.fold.lock().expect("tray fold");
        let state = fold.state(link_up);
        let peer_name = fold.peer().map(str::to_string);
        drop(fold);
        let icon = match state {
            TrayState::Local => &self.icon_local,
            TrayState::Remote => &self.icon_remote,
            TrayState::Paused => &self.icon_paused,
            TrayState::Offline => &self.icon_offline,
        }
        .clone();
        let _ = self.icon.set_icon(Some(icon));
        let _ = self
            .icon
            .set_tooltip(Some(state.tooltip(peer_name.as_deref())));
        // Status line + pause label follow the live state.
        let status_text = match (&state, peer_name.as_deref()) {
            (TrayState::Local, _) => "键鼠在本机".to_string(),
            (TrayState::Remote, Some(p)) => format!("键鼠在 {p}"),
            (TrayState::Remote, None) => "键鼠被远端接管".to_string(),
            (TrayState::Paused, _) => "共享已暂停".to_string(),
            (TrayState::Offline, _) => "daemon 未连接".to_string(),
        };
        let _ = self.status_item.set_text(status_text);
        let toggle_text = match state {
            TrayState::Paused => "恢复共享",
            _ => "暂停共享",
        };
        let _ = self.toggle_item.set_text(toggle_text);
        let _ = self.toggle_item.set_enabled(state != TrayState::Offline);
    }
}

fn icon_for<R: Runtime>(
    app: &AppHandle<R>,
    state: TrayState,
) -> tauri::Result<tauri::image::Image<'static>> {
    let _ = app; // reserved for future per-platform asset lookup
    Ok(state.tinted_icon(crate::tray_icon_rgba()))
}

/// Show (or focus) the main window; tray double/left click and menu share it.
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Flip the OS autostart entry and rewrite the menu label from the NEW state.
pub fn toggle_autostart<R: Runtime>(app: &AppHandle<R>) {
    let mgr = app.autolaunch();
    let turn_on = !mgr.is_enabled().unwrap_or(false);
    let res = if turn_on { mgr.enable() } else { mgr.disable() };
    match res {
        Ok(_) => {
            if let Some(item) = app.menu().and_then(|m| m.get(MENU_AUTOSTART)) {
                if let Some(mi) = item.as_menuitem() {
                    let _ = mi.set_text(if turn_on {
                        "取消开机自启"
                    } else {
                        "开机自启"
                    });
                }
            }
            tracing::info!("autostart -> {turn_on}");
        }
        Err(e) => tracing::warn!("autostart toggle failed: {e}"),
    }
}

/// Tray panel geometry (§225: Magpie-style 380-wide panel). The SPA reports
/// its content height over the tray-action channel (`resize`); the window
/// follows it within [`PANEL_MIN_H`, `PANEL_MAX_H`].
pub const PANEL_W: f64 = 380.0;
const PANEL_H: f64 = 468.0;
pub const PANEL_MIN_H: f64 = 200.0;
pub const PANEL_MAX_H: f64 = 640.0;

/// Where the tray panel hangs relative to the tray icon (§234).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelAnchor {
    /// macOS: the menu bar is on top — hang below the clicked icon.
    BelowCursor,
    /// Windows: the taskbar is at the bottom — pin to the work area's
    /// bottom-right corner, above the taskbar, like Magpie/the system flyouts.
    BottomRight,
}

/// The platform's panel anchor.
pub const PANEL_ANCHOR: PanelAnchor = if cfg!(target_os = "macos") {
    PanelAnchor::BelowCursor
} else {
    PanelAnchor::BottomRight
};

/// Margin between the panel and the work-area edge, logical px.
pub const PANEL_GAP: f64 = 12.0;

/// An axis-aligned rectangle in physical pixels (a monitor's work area).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PxRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Panel top-left corner, all values physical px. Pure so tests pin the
/// geometry: the panel always lands fully inside `work` (never behind the
/// taskbar, never off-screen).
pub fn panel_origin(
    anchor: PanelAnchor,
    cursor: (f64, f64),
    work: PxRect,
    size: (f64, f64),
    gap: f64,
) -> (f64, f64) {
    let (w, h) = size;
    let max_x = (work.x + work.w - w - gap).max(work.x);
    let max_y = (work.y + work.h - h - gap).max(work.y);
    match anchor {
        PanelAnchor::BottomRight => (max_x, max_y),
        PanelAnchor::BelowCursor => (
            (cursor.0 - w / 2.0).clamp(work.x.min(max_x), max_x),
            (cursor.1 + 6.0).clamp(work.y, max_y),
        ),
    }
}

/// Position `panel` (logical size `w`×`h`) for a tray click at `cursor`
/// (physical) on the monitor that contains the click — falling back to the
/// panel's current, then the primary monitor.
fn place_panel<R: Runtime>(
    app: &AppHandle<R>,
    panel: &tauri::WebviewWindow<R>,
    cursor: Option<(f64, f64)>,
    w: f64,
    h: f64,
) {
    let monitor = cursor
        .and_then(|(x, y)| app.monitor_from_point(x, y).ok().flatten())
        .or_else(|| panel.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let scale = m.scale_factor();
    let wa = m.work_area();
    let work = PxRect {
        x: f64::from(wa.position.x),
        y: f64::from(wa.position.y),
        w: f64::from(wa.size.width),
        h: f64::from(wa.size.height),
    };
    // No click (resize re-anchor): keep the current horizontal anchor.
    let cursor = cursor.unwrap_or_else(|| {
        panel
            .outer_position()
            .map(|p| (f64::from(p.x) + w * scale / 2.0, f64::from(p.y) - 6.0))
            .unwrap_or((work.x + work.w, work.y))
    });
    let (x, y) = panel_origin(
        PANEL_ANCHOR,
        cursor,
        work,
        (w * scale, h * scale),
        PANEL_GAP * scale,
    );
    let _ = panel.set_position(tauri::PhysicalPosition::new(
        x.round() as i32,
        y.round() as i32,
    ));
}

/// Apply a content-reported height to the open panel and re-anchor it so a
/// bottom-anchored (Windows) panel grows upward instead of under the taskbar.
pub fn resize_panel<R: Runtime>(app: &AppHandle<R>, height: f64) {
    let Some(panel) = app.get_webview_window("tray-panel") else {
        return;
    };
    let h = height.clamp(PANEL_MIN_H, PANEL_MAX_H);
    let _ = panel.set_size(tauri::LogicalSize::new(PANEL_W, h));
    if PANEL_ANCHOR == PanelAnchor::BottomRight {
        place_panel(app, &panel, None, PANEL_W, h);
    }
}

/// What a left click on the tray icon does to the panel (§238).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelAction {
    Hide,
    Show,
}

/// Hide only a panel that is both visible and focused — i.e. one the user is
/// looking at. A "visible" but unfocused panel (left on another Space, behind
/// a fullscreen app, or after a missed focus-out) is re-shown instead, so a
/// click can never toggle between two invisible states.
pub fn panel_click_action(visible: bool, focused: bool) -> PanelAction {
    if visible && focused {
        PanelAction::Hide
    } else {
        PanelAction::Show
    }
}

/// Toggle the frameless tray panel. It is created lazily (hidden), anchored
/// per [`PANEL_ANCHOR`] on the monitor of the click, then shown/hidden.
/// Focus-out hides it (the window-event hook installs once at creation).
pub fn toggle_panel<R: Runtime>(app: &AppHandle<R>, cursor: &tauri::PhysicalPosition<f64>) {
    const LABEL: &str = "tray-panel";
    let panel = match app.get_webview_window(LABEL) {
        Some(w) => w,
        None => {
            let b = tauri::WebviewWindowBuilder::new(
                app,
                LABEL,
                tauri::WebviewUrl::App("#tray".into()),
            )
            .title("Tetherly")
            .decorations(false)
            .resizable(false)
            .skip_taskbar(true)
            .shadow(true)
            .always_on_top(true)
            // §238: follow the user onto any Space (fullscreen apps live in
            // their own Spaces) instead of opening off-screen.
            .visible_on_all_workspaces(true)
            .visible(false)
            .inner_size(PANEL_W, PANEL_H);
            match b.build() {
                Ok(w) => {
                    let wh = w.clone();
                    w.on_window_event(move |e| {
                        if let tauri::WindowEvent::Focused(false) = e {
                            tracing::info!("tray panel: focus lost -> hide");
                            let _ = wh.hide();
                        }
                    });
                    w
                }
                Err(e) => {
                    tracing::warn!("tray panel build failed: {e}");
                    return;
                }
            }
        }
    };
    let visible = panel.is_visible().unwrap_or(false);
    let focused = panel.is_focused().unwrap_or(false);
    let action = panel_click_action(visible, focused);
    tracing::info!(
        visible,
        focused,
        ?action,
        x = cursor.x,
        y = cursor.y,
        "tray panel: click"
    );
    if action == PanelAction::Hide {
        let _ = panel.hide();
    } else {
        // Re-anchor on every open so multi-monitor moves follow the click.
        let h = panel
            .inner_size()
            .ok()
            .zip(panel.scale_factor().ok())
            .map(|(s, k)| f64::from(s.height) / k)
            .filter(|h| *h > 0.0)
            .unwrap_or(PANEL_H);
        place_panel(app, &panel, Some((cursor.x, cursor.y)), PANEL_W, h);
        if let Err(e) = panel.show() {
            tracing::warn!("tray panel show failed: {e}");
        }
        if let Err(e) = panel.set_focus() {
            tracing::warn!("tray panel focus failed: {e}");
        }
    }
}

#[cfg(test)]
mod placement_tests {
    use super::{panel_origin, PanelAnchor, PxRect};

    // home-win: 1920×1080 screen, taskbar 48 px at the bottom → work area
    // 0,0–1920×1032 (production probe 2026-11-23).
    const WIN_WORK: PxRect = PxRect {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1032.0,
    };

    #[test]
    fn windows_panel_sits_in_the_bottom_right_corner_above_the_taskbar() {
        // Click on the tray icon (inside the taskbar, y > work bottom).
        let (x, y) = panel_origin(
            PanelAnchor::BottomRight,
            (1700.0, 1056.0),
            WIN_WORK,
            (380.0, 468.0),
            12.0,
        );
        assert_eq!((x, y), (1920.0 - 380.0 - 12.0, 1032.0 - 468.0 - 12.0));
        // Fully inside the work area: nothing hidden behind the taskbar.
        assert!(y + 468.0 <= 1032.0 && x + 380.0 <= 1920.0);
    }

    #[test]
    fn windows_corner_ignores_where_on_the_taskbar_the_icon_was_clicked() {
        let a = panel_origin(
            PanelAnchor::BottomRight,
            (40.0, 1060.0),
            WIN_WORK,
            (380.0, 300.0),
            12.0,
        );
        let b = panel_origin(
            PanelAnchor::BottomRight,
            (1880.0, 1040.0),
            WIN_WORK,
            (380.0, 300.0),
            12.0,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn windows_corner_respects_a_secondary_monitor_offset_and_hidpi() {
        // Monitor to the right at 150%: physical work area 1920..4800 × 0..2088.
        let work = PxRect {
            x: 1920.0,
            y: 0.0,
            w: 2880.0,
            h: 2088.0,
        };
        let (x, y) = panel_origin(
            PanelAnchor::BottomRight,
            (4500.0, 2140.0),
            work,
            (570.0, 702.0),
            18.0,
        );
        assert_eq!((x, y), (4800.0 - 570.0 - 18.0, 2088.0 - 702.0 - 18.0));
    }

    #[test]
    fn macos_panel_hangs_below_the_menubar_icon_clamped_to_the_screen() {
        let work = PxRect {
            x: 0.0,
            y: 50.0,
            w: 3024.0,
            h: 1914.0,
        };
        let (x, y) = panel_origin(
            PanelAnchor::BelowCursor,
            (1500.0, 20.0),
            work,
            (760.0, 936.0),
            12.0,
        );
        assert_eq!((x, y), (1500.0 - 380.0, 50.0));
        // An icon at the far right edge must not push the panel off-screen.
        let (x, _) = panel_origin(
            PanelAnchor::BelowCursor,
            (3010.0, 20.0),
            work,
            (760.0, 936.0),
            12.0,
        );
        assert_eq!(x, 3024.0 - 760.0 - 12.0);
    }
}

#[cfg(test)]
mod toggle_tests {
    use super::{panel_click_action, PanelAction};

    #[test]
    fn click_hides_only_a_panel_the_user_is_actually_looking_at() {
        assert_eq!(panel_click_action(true, true), PanelAction::Hide);
    }

    #[test]
    fn click_on_a_visible_but_unfocused_panel_reopens_it_instead_of_hiding() {
        // §238: a panel left "visible" off-screen (other Space / fullscreen
        // app / lost focus without the hide hook firing) must not swallow
        // the click — otherwise clicks alternate between two invisible
        // states and the tray looks dead.
        assert_eq!(panel_click_action(true, false), PanelAction::Show);
    }

    #[test]
    fn click_on_a_hidden_panel_shows_it() {
        assert_eq!(panel_click_action(false, false), PanelAction::Show);
        assert_eq!(panel_click_action(false, true), PanelAction::Show);
    }
}
