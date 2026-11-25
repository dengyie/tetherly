// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Tauri 2 desktop shell (M3, ADR-139 §5.1/§5.2).
//!
//! The WebView loads the same SPA the daemon serves at `/next/`, but every
//! API call goes through [`proxy::request`] (the token stays in this
//! process) and every server event arrives on the single SSE connection
//! owned by [`events::run`]. No daemon code is linked here — the shell
//! discovers it by data-dir conventions ([`token`]).

pub mod dragbadge;
pub mod events;
pub mod proxy;
pub mod shelllog;
pub mod token;
pub mod tray;
pub mod trayowner;
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

/// State handed to the `api` command: daemon port + token. The token is
/// re-read from disk on every use ([`token::fresh_or`]) because the daemon
/// rotates it on restart; the startup copy is only the fallback. It lives
/// here only; the WebView cannot read app state.
#[derive(Debug)]
pub struct Daemon {
    pub port: u16,
    startup_token: String,
}

impl Daemon {
    /// Current daemon token (follows restart rotation).
    pub fn token(&self) -> String {
        token::fresh_or(&self.startup_token)
    }
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
    proxy::request(daemon.port, &daemon.token(), &method, &path, body.as_ref())
        .await
        .map_err(|e| e.to_string())
}

/// The current SSE connection state, PULLED by the SPA right after it
/// subscribes. Tauri emits are broadcast-and-forget and the WebView's JS
/// listeners register long after the loopback SSE handshake — the opening
/// Connecting→Live pair would land in the void and the shell would show
/// "连接中" forever while every page stayed on its waiting placeholder
/// (production 2026-11-23). The push keeps driving later transitions; this
/// pull only closes the startup race.
#[tauri::command]
fn link_state(
    events: State<'_, std::sync::Arc<events::LinkStateCell>>,
) -> Result<&'static str, String> {
    let cell = events.inner();
    Ok(match cell.get() {
        Some(events::LinkState::Live) => "live",
        Some(events::LinkState::Connecting) => "connecting",
        Some(events::LinkState::Resyncing) => "resyncing",
        Some(events::LinkState::Offline) => "offline",
        None => "connecting",
    })
}

/// Read daemon discovery settings at startup. Called from `run` and tests.
///
/// Production 2026-11-25: this used to fail the whole shell when the token
/// could not be read (`exit(1)` in `run`). With daemon autostart (launchd
/// LaunchAgent / scheduled task) installed, login can race: the shell may
/// come up BEFORE the daemon has minted its first `api.token`, and a dead
/// tray is strictly worse than an offline tray. The shell now always starts;
/// the SSE loop retries with backoff and re-reads the token every attempt
/// (`token::fresh_or`), so it links up as soon as the daemon is serving.
pub fn build_daemon_state() -> Daemon {
    // Empty fallback: until the daemon mints a token, every loopback call
    // 401s/REFUSES and the UI shows the existing offline state; nothing to
    // recover manually — `fresh_or` picks up the real token on the next
    // backoff attempt.
    Daemon {
        port: token::ui_port(),
        startup_token: token::read_api_token().unwrap_or_default(),
    }
}

pub fn run() {
    let daemon = build_daemon_state();
    tracing::info!(port = daemon.port, "shell starting against loopback daemon");

    tauri::Builder::default()
        // §230: must be the first plugin. A second launch (the daemon's
        // tray "open main window" ShellExecutes tetherly://open, which on
        // Windows starts a NEW process) forwards its deep link to this
        // instance — the deep-link feature re-emits it to on_open_url — and
        // exits, so the click always surfaces the one running client.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            trayui::show_main_window(app);
        }))
        // §239: closing the main window hides it (tray app) so 打开主窗口
        // can bring it back; destroying it left the menu item dead.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if trayui::hide_instead_of_close(window.label()) {
                    api.prevent_close();
                    let _ = window.hide();
                    tracing::info!("main window: close -> hide");
                }
            }
        })
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .manage(daemon)
        .manage(std::sync::Arc::new(events::LinkStateCell::default()))
        .manage(trayowner::Claimed(std::sync::atomic::AtomicBool::new(
            false,
        )))
        .invoke_handler(tauri::generate_handler![api, link_state])
        .setup(|app| {
            let handle: AppHandle = app.handle().clone();
            let port = handle.state::<Daemon>().port;
            let token = handle.state::<Daemon>().token();
            // The single SSE connection for every window (main + tray panel).
            // §147: BOTH channel listeners register BEFORE the SSE task
            // spawns. The loopback handshake completes in milliseconds —
            // with listeners registered after the spawn, the first
            // `Live`/snapshot lands in the void and (because a healthy
            // connection never reconnects) the shell would never claim the
            // tray (production 2026-11-17).
            let fwd = handle.clone();
            // §177: identity dedup for activity notifications — the daemon
            // republishes the full Activity section on every related change,
            // so the seen-set must live ACROSS frames (owned by this
            // listener), not inside notice_for.
            let notice_seen = std::sync::Mutex::new(tray::NoticeSeen::new());
            handle.listen(events::EVENT_CHANNEL, move |ev| {
                if let Ok(frame) = serde_json::from_str::<events::WireEvent>(ev.payload()) {
                    if let Some(carry) = dragbadge::carry_from_frame(&frame.ty, &frame.data) {
                        dragbadge::apply(&fwd, carry);
                        return;
                    }
                    // Desktop notifications (§6.2): seat takeover/return and
                    // unread activity. `was_remote` is the fold state BEFORE
                    // this frame so repeated remote frames stay silent.
                    let was_remote = fwd
                        .try_state::<TrayUi<tauri::Wry>>()
                        .map(|ui| ui.on_event(true, &frame.ty, &frame.data))
                        .unwrap_or(false);
                    let notice = notice_seen
                        .lock()
                        .map(|mut seen| {
                            tray::notice_for(was_remote, &frame.ty, &frame.data, &mut seen)
                        })
                        .unwrap_or_else(|_| {
                            tray::notice_for(
                                was_remote,
                                &frame.ty,
                                &frame.data,
                                &mut tray::NoticeSeen::new(),
                            )
                        });
                    if let Some(notice) = notice {
                        // §234: desktop notifications are opt-in. The daemon's
                        // banner switch (default OFF) is the single source of
                        // truth — read it per notice so a settings change
                        // applies at once without a cache to invalidate.
                        let app = fwd.clone();
                        tauri::async_runtime::spawn(async move {
                            let d = app.state::<Daemon>();
                            let enabled = matches!(
                                proxy::request(d.port, &d.token(), "GET", "/api/v1/notify/config", None).await,
                                Ok(r) if r.status == 200 && tray::notices_enabled(&r.body)
                            );
                            if enabled {
                                use tauri_plugin_notification::NotificationExt;
                                let _ = app
                                    .notification()
                                    .builder()
                                    .title(notice.title)
                                    .body(notice.body)
                                    .show();
                            }
                        });
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
                // §6.3 tray takeover: every down→up edge (re)claims the
                // surface — a restarted daemon or a watchdog reclaim resets
                // the owner, so a once-per-process claim left two icons.
                if fwd.state::<trayowner::Claimed>().on_link(up) {
                    let d = fwd.state::<Daemon>();
                    let (port, token) = (d.port, d.token());
                    tauri::async_runtime::spawn(async move {
                        trayowner::claim(port, &token).await;
                    });
                }
            });
            // §6.3: watchdog reclaims without a link drop (sleep/wake) are
            // invisible to the edge claim above — poll while linked.
            let rc = handle.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(trayowner::RECHECK).await;
                    let linked = rc
                        .state::<trayowner::Claimed>()
                        .0
                        .load(std::sync::atomic::Ordering::SeqCst);
                    if linked {
                        let d = rc.state::<Daemon>();
                        let (port, token) = (d.port, d.token());
                        trayowner::recheck(port, &token).await;
                    }
                }
            });
            let cell = handle.state::<std::sync::Arc<events::LinkStateCell>>().inner().clone();
            tauri::async_runtime::spawn(events::run(handle.clone(), port, token, cell));

            // Tray: real-state sync (§6.2). The SSE task forwards frames to
            // the tray through app events; the tray answers menu actions the
            // same way so no shared state crosses the runtime boundary.
            let tray = TrayUi::build(&handle).map_err(|e| format!("tray init: {e}"))?;
            handle.manage(tray);
            // Menu action: the share pause switch is settings.general.advertise
            // (PATCH /api/v1/settings) — the same switch the SPA settings page
            // exposes. Read-modify-write so the patch always sets a field.
            let act = handle.clone();
            handle.listen(trayui::TOGGLE_REQUEST, move |_| {
                let app = act.clone();
                tauri::async_runtime::spawn(async move {
                    let d = app.state::<Daemon>();
                    let current =
                        match proxy::request(d.port, &d.token(), "GET", "/api/v1/settings", None)
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
                    // SettingsPatch is FLAT (deny_unknown_fields): the old
                    // nested `{"general": {...}}` body parsed as a 400.
                    let patch = serde_json::json!({"advertise": !on});
                    match proxy::request(
                        d.port,
                        &d.token(),
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
            // Tray panel SPA actions (§6.3/§225): 打开主窗口 shows the main
            // shell and hides the popover; 关闭 only hides; 退出 ends the
            // shell. Toggles/errors are already reflected via the
            // settings.changed event loop, so they need no hook here.
            let act = handle.clone();
            handle.listen(crate::traybridge::TRAY_ACTION_CHANNEL, move |ev| {
                if let Ok(payload) = serde_json::from_str::<serde_json::Value>(ev.payload()) {
                    let hide_panel = || {
                        if let Some(panel) = act.get_webview_window("tray-panel") {
                            let _ = panel.hide();
                        }
                    };
                    match payload["action"].as_str().unwrap_or_default() {
                        "open-main" => {
                            trayui::show_main_window(&act);
                            hide_panel();
                        }
                        "close" => hide_panel(),
                        "quit" => act.exit(0),
                        "resize" => {
                            if let Some(h) = payload["height"].as_f64() {
                                trayui::resize_panel(&act, h);
                            }
                        }
                        _ => {}
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
        .build(tauri::generate_context!())
        .expect("error while building tetherly shell")
        .run(|app, event| {
            // §6.3 tray takeover: clean exit releases the claim so the
            // built-in tray returns immediately, not after the daemon's
            // 10s reclaim window. Best-effort: the watchdog covers crashes.
            // §239: Dock icon click with the main window hidden reopens it.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                trayui::show_main_window(app);
                return;
            }
            if let tauri::RunEvent::ExitRequested { .. } = event {
                let d = app.state::<Daemon>();
                let once = app.state::<trayowner::Claimed>();
                if once.0.load(std::sync::atomic::Ordering::SeqCst) {
                    // Synchronous best-effort on a fresh runtime; the
                    // process is exiting, so a short deadline is plenty.
                    let port = d.port;
                    let token = d.token();
                    if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        let _ = rt.block_on(async move {
                            tokio::time::timeout(
                                std::time::Duration::from_secs(2),
                                trayowner::release(port, &token),
                            )
                            .await
                            .ok()
                        });
                    }
                }
            }
        });
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
        // No env token, no data dir: the shell must STILL build state
        // (degraded mode) — it used to exit(1) and die before the tray
        // existed when login raced the daemon's first token mint
        // (production 2026-11-25).
        unsafe { std::env::remove_var("TETHERLY_API_TOKEN") };
        unsafe { std::env::set_var("TETHERLY_DATA_DIR", "/nonexistent-shell-test") };
        let d = build_daemon_state();
        assert_eq!(d.token(), "");
        assert_eq!(d.port, token::DEFAULT_UI_PORT);

        // Env token present: state builds with it, port falls back to default.
        unsafe { std::env::set_var("TETHERLY_API_TOKEN", "tok") };
        let d = build_daemon_state();
        assert_eq!(d.token(), "tok");
        assert_eq!(d.port, token::DEFAULT_UI_PORT);
        unsafe { std::env::remove_var("TETHERLY_API_TOKEN") };
        unsafe { std::env::remove_var("TETHERLY_DATA_DIR") };
    }
}
