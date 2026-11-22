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

/// Toggle the 360×480 frameless panel. It is created lazily near the tray
/// icon (the click's physical cursor position), then shown/hidden. Focus-out
/// hides it (the window-event hook installs once at creation).
pub fn toggle_panel<R: Runtime>(app: &AppHandle<R>, cursor: &tauri::PhysicalPosition<f64>) {
    const LABEL: &str = "tray-panel";
    // Anchor: horizontally centered under the cursor (the tray icon), a few
    // px below it. The click position is physical; the builder wants logical.
    let anchor = |app: &AppHandle<R>| {
        let scale = app
            .primary_monitor()
            .ok()
            .flatten()
            .map(|m| m.scale_factor())
            .unwrap_or(1.0);
        (
            (cursor.x / scale - PANEL_W / 2.0).max(0.0),
            (cursor.y / scale + 6.0).max(0.0),
        )
    };
    let panel = match app.get_webview_window(LABEL) {
        Some(w) => w,
        None => {
            let mut b = tauri::WebviewWindowBuilder::new(
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
            .inner_size(PANEL_W, PANEL_H);
            let (x, y) = anchor(app);
            b = b.position(x, y);
            match b.build() {
                Ok(w) => {
                    let wh = w.clone();
                    w.on_window_event(move |e| {
                        if let tauri::WindowEvent::Focused(false) = e {
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
    if panel.is_visible().unwrap_or(false) {
        let _ = panel.hide();
    } else {
        // Re-anchor on every open so multi-monitor moves keep it near the icon.
        let (x, y) = anchor(app);
        let _ = panel.set_position(tauri::LogicalPosition::new(x, y));
        let _ = panel.show();
        let _ = panel.set_focus();
    }
}
