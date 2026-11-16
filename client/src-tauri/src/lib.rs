// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Tauri 2 desktop shell (M3, ADR-139 §5.1/§5.2).
//!
//! The WebView loads the same SPA the daemon serves at `/next/`, but every
//! API call goes through [`proxy::request`] (the token stays in this
//! process) and every server event arrives on the single SSE connection
//! owned by [`events::run`]. No daemon code is linked here — the shell
//! discovers it by data-dir conventions ([`token`]).

pub mod events;
pub mod proxy;
pub mod token;
pub mod tray;
pub mod trayui;

use tauri::{AppHandle, Listener, Manager, State};

use crate::trayui::TrayUi;

/// Wire constant shared with the SPA (client/src/lib/transport/traybridge.ts):
/// the tray panel window emits its actions on this channel.
pub mod traybridge {
    pub const TRAY_ACTION_CHANNEL: &str = "tetherly://tray-action";
}

/// The embedded tray glyph ([`tray::GLYPH`]); the runtime tints it per state.
pub fn tray_icon_rgba() -> &'static [u8] {
    &tray::GLYPH
}

/// State handed to the `api` command: daemon port + token, read once at
/// startup. The token lives here only; the WebView cannot read app state.
#[derive(Debug)]
pub struct Daemon {
    pub port: u16,
    pub token: String,
}

/// The one custom command the WebView may call: `invoke("api", {method,
/// path, body})`. Mirrors [`crate::proxy::request`]; non-2xx replies are
/// returned as a successful invoke carrying `status` — the TS layer turns
/// them into `ApiError`s so error semantics stay with the contract types.
#[tauri::command]
async fn api(
    daemon: State<'_, Daemon>,
    method: String,
    path: String,
    body: Option<serde_json::Value>,
) -> Result<proxy::ProxyReply, String> {
    proxy::request(daemon.port, &daemon.token, &method, &path, body.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// Read daemon discovery settings at startup. Called from `run` and tests.
pub fn build_daemon_state() -> Result<Daemon, String> {
    let token = token::read_api_token().map_err(|e| {
        format!(
            "cannot read the daemon API token ({e}). Is the daemon running? Start it with `tetherly run` first."
        )
    })?;
    Ok(Daemon {
        port: token::ui_port(),
        token,
    })
}

pub fn run() {
    let daemon = match build_daemon_state() {
        Ok(d) => d,
        Err(msg) => {
            eprintln!("tetherly-shell: {msg}");
            std::process::exit(1);
        }
    };
    tracing::info!(port = daemon.port, "shell starting against loopback daemon");

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .manage(daemon)
        .invoke_handler(tauri::generate_handler![api])
        .setup(|app| {
            let handle: AppHandle = app.handle().clone();
            let port = handle.state::<Daemon>().port;
            let token = handle.state::<Daemon>().token.clone();
            // The single SSE connection for every window (main + tray panel).
            tauri::async_runtime::spawn(events::run(handle.clone(), port, token));

            // Tray: real-state sync (§6.2). The SSE task forwards frames to
            // the tray through app events; the tray answers menu actions the
            // same way so no shared state crosses the runtime boundary.
            let tray = TrayUi::build(&handle).map_err(|e| format!("tray init: {e}"))?;
            handle.manage(tray);
            let fwd = handle.clone();
            handle.listen(events::EVENT_CHANNEL, move |ev| {
                if let Ok(frame) = serde_json::from_str::<events::WireEvent>(ev.payload()) {
                    // Desktop notifications (§6.2): seat takeover/return and
                    // unread activity. `was_remote` is the fold state BEFORE
                    // this frame so repeated remote frames stay silent.
                    let was_remote = fwd
                        .try_state::<TrayUi<tauri::Wry>>()
                        .map(|ui| ui.on_event(true, &frame.ty, &frame.data))
                        .unwrap_or(false);
                    if let Some(notice) = tray::notice_for(was_remote, &frame.ty, &frame.data) {
                        use tauri_plugin_notification::NotificationExt;
                        let _ = fwd
                            .notification()
                            .builder()
                            .title(notice.title)
                            .body(notice.body)
                            .show();
                    }
                }
            });
            let fwd = handle.clone();
            handle.listen(events::LINK_CHANNEL, move |ev| {
                let up = matches!(
                    serde_json::from_str::<events::LinkState>(ev.payload()),
                    Ok(events::LinkState::Live | events::LinkState::Resyncing)
                );
                if let Some(ui) = fwd.try_state::<TrayUi<tauri::Wry>>() {
                    ui.on_link(up);
                }
            });
            // Menu action: the share pause switch is settings.general.advertise
            // (PATCH /api/v1/settings) — the same switch the SPA settings page
            // exposes. Read-modify-write so the patch always sets a field.
            let act = handle.clone();
            handle.listen(trayui::TOGGLE_REQUEST, move |_| {
                let app = act.clone();
                tauri::async_runtime::spawn(async move {
                    let d = app.state::<Daemon>();
                    let current =
                        match proxy::request(d.port, &d.token, "GET", "/api/v1/settings", None)
                            .await
                        {
                            Ok(r) if r.status == 200 => r.body,
                            Ok(r) => {
                                tracing::warn!("tray toggle: settings read {} ", r.status);
                                return;
                            }
                            Err(e) => {
                                tracing::warn!("tray toggle failed: {e}");
                                return;
                            }
                        };
                    let on = current["general"]["advertise"].as_bool().unwrap_or(true);
                    let patch = serde_json::json!({"general": {"advertise": !on}});
                    match proxy::request(
                        d.port,
                        &d.token,
                        "PATCH",
                        "/api/v1/settings",
                        Some(&patch),
                    )
                    .await
                    {
                        Ok(_) => tracing::info!("tray toggle: advertise -> {}", !on),
                        Err(e) => tracing::warn!("tray toggle failed: {e}"),
                    }
                });
            });
            // Tray panel SPA actions (§6.3): 打开主窗口 shows the main shell
            // and hides the popover; toggles/errors are already reflected via
            // the settings.changed event loop, so only open-main needs a hook.
            let act = handle.clone();
            handle.listen(crate::traybridge::TRAY_ACTION_CHANNEL, move |ev| {
                if let Ok(payload) = serde_json::from_str::<serde_json::Value>(ev.payload()) {
                    if payload["action"] == "open-main" {
                        trayui::show_main_window(&act);
                        if let Some(panel) = act.get_webview_window("tray-panel") {
                            let _ = panel.hide();
                        }
                    }
                }
            });
            // 深链接 (§6.3): tetherly://open (any tetherly:// url) opens the
            // main window — covers cold start via get_current + runtime via
            // on_open_url. Scheme comes from plugins.deep-link.desktop in
            // tauri.conf.json; the bundler registers it per-platform.
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let dl = handle.clone();
                handle.deep_link().on_open_url(move |_| {
                    trayui::show_main_window(&dl);
                });
                if handle.deep_link().get_current().is_ok_and(|c| c.is_some()) {
                    trayui::show_main_window(&handle);
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tetherly shell");
}

#[cfg(test)]
mod tests {
    use super::*;

    // Env vars are process-global: take the shared lock from token::tests so
    // parallel cargo tests serialize instead of racing (§124 flake family).
    #[test]
    fn build_daemon_state_sequential() {
        let _guard = token::tests::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // No env token, no data dir: must fail with the guidance message.
        unsafe { std::env::remove_var("TETHERLY_API_TOKEN") };
        unsafe { std::env::set_var("TETHERLY_DATA_DIR", "/nonexistent-shell-test") };
        let err = build_daemon_state().unwrap_err();
        assert!(err.contains("API token"), "{err}");

        // Env token present: state builds, port falls back to the default.
        unsafe { std::env::set_var("TETHERLY_API_TOKEN", "tok") };
        let d = build_daemon_state().unwrap();
        assert_eq!(d.token, "tok");
        assert_eq!(d.port, token::DEFAULT_UI_PORT);
        unsafe { std::env::remove_var("TETHERLY_API_TOKEN") };
        unsafe { std::env::remove_var("TETHERLY_DATA_DIR") };
    }
}
