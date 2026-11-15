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

use tauri::{AppHandle, Manager, State};

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
        .manage(daemon)
        .invoke_handler(tauri::generate_handler![api])
        .setup(|app| {
            let handle: AppHandle = app.handle().clone();
            let port = handle.state::<Daemon>().port;
            let token = handle.state::<Daemon>().token.clone();
            // The single SSE connection for every window (main + tray panel).
            tauri::async_runtime::spawn(events::run(handle, port, token));
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
