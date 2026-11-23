// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Daemon discovery for the shell: where the API token lives and which port
//! the loopback UI listens on.
//!
//! These mirror the daemon's own resolution (`tetherly-node` `store.rs` /
//! `cli.rs`) so the shell reads exactly the files the daemon writes without
//! linking it. The token never leaves this process: the WebView speaks only
//! through [`crate::proxy`], which attaches the `Authorization` header.

use std::path::PathBuf;

/// Loopback UI port the daemon binds by default (`--ui-port`, cli.rs).
pub const DEFAULT_UI_PORT: u16 = 45716;

/// The daemon's data directory, resolved the same way as
/// `tetherly_node::store::default_data_dir`: `TETHERLY_DATA_DIR` overrides,
/// else `%LOCALAPPDATA%\tetherly` (Windows), else `~/.local/share/tetherly`.
pub fn daemon_data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("TETHERLY_DATA_DIR") {
        return PathBuf::from(p);
    }
    if let Some(base) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(base).join("tetherly");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".local/share/tetherly");
    }
    PathBuf::from(".tetherly")
}

/// Read the API token minted by the daemon. `TETHERLY_API_TOKEN` overrides,
/// matching the CLI. Otherwise prefer the §4.4 client token
/// (`<data_dir>/client.token`, Write tier) the Windows SYSTEM daemon mints
/// for the interactive user, falling back to the daemon token
/// (`api.token`, Admin) that macOS user-session daemons rely on.
pub fn read_api_token() -> std::io::Result<String> {
    if let Ok(t) = std::env::var("TETHERLY_API_TOKEN") {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let dir = daemon_data_dir();
    for name in ["client.token", "api.token"] {
        let path = dir.join(name);
        if let Ok(raw) = std::fs::read_to_string(&path) {
            let t = raw.trim().to_string();
            if !t.is_empty() {
                return Ok(t);
            }
        }
    }
    Err(std::io::Error::other(format!(
        "{}: no client.token or api.token",
        dir.display()
    )))
}

/// The token as it is on disk NOW, else `fallback`. The daemon mints a new
/// token on every restart (deploy, /system/restart), so anything that
/// reconnects must re-read: a token cached at startup 401s forever and the
/// shell silently stays offline (production 2026-11-23: after a daemon
/// restart the client never relinked and the built-in tray reappeared).
pub fn fresh_or(fallback: &str) -> String {
    read_api_token().unwrap_or_else(|_| fallback.to_string())
}

/// The loopback UI port: `TETHERLY_UI_PORT` overrides, else the default.
pub fn ui_port() -> u16 {
    std::env::var("TETHERLY_UI_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_UI_PORT)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    // Env vars are process-global: every test touching them takes this lock,
    // so parallel cargo tests serialize instead of racing (§124 flake family).
    pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn fresh_token_follows_daemon_rotation() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("tetherly-tok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        unsafe {
            std::env::remove_var("TETHERLY_API_TOKEN");
            std::env::set_var("TETHERLY_DATA_DIR", &dir);
        }
        std::fs::write(dir.join("api.token"), "first\n").unwrap();
        assert_eq!(fresh_or("stale"), "first");
        std::fs::write(dir.join("api.token"), "rotated\n").unwrap();
        assert_eq!(
            fresh_or("stale"),
            "rotated",
            "restart rotation is picked up"
        );
        std::fs::remove_file(dir.join("api.token")).unwrap();
        assert_eq!(
            fresh_or("stale"),
            "stale",
            "missing file keeps the last token"
        );
        unsafe { std::env::remove_var("TETHERLY_DATA_DIR") };
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn env_discovery_sequential() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Port: default when unset; a bogus value falls back, a number wins.
        unsafe { std::env::remove_var("TETHERLY_UI_PORT") };
        assert_eq!(ui_port(), DEFAULT_UI_PORT);
        unsafe { std::env::set_var("TETHERLY_UI_PORT", "bogus") };
        assert_eq!(ui_port(), DEFAULT_UI_PORT);
        unsafe { std::env::set_var("TETHERLY_UI_PORT", "12345") };
        assert_eq!(ui_port(), 12345);
        unsafe { std::env::remove_var("TETHERLY_UI_PORT") };

        // Token: env override wins, then the data-dir file, trimmed.
        unsafe { std::env::set_var("TETHERLY_API_TOKEN", "envtok") };
        assert_eq!(read_api_token().unwrap(), "envtok");
        unsafe { std::env::remove_var("TETHERLY_API_TOKEN") };

        let dir = std::env::temp_dir().join(format!("shell-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("api.token"), "abc123\n").unwrap();
        unsafe { std::env::set_var("TETHERLY_DATA_DIR", &dir) };
        assert_eq!(read_api_token().unwrap(), "abc123");

        // §4.4: the client token wins when both exist (Windows SYSTEM
        // daemon mints it into the user profile), and a client.token alone
        // is enough.
        std::fs::write(dir.join("client.token"), "clitok").unwrap();
        assert_eq!(read_api_token().unwrap(), "clitok");
        std::fs::remove_file(dir.join("api.token")).unwrap();
        assert_eq!(read_api_token().unwrap(), "clitok");
        std::fs::remove_file(dir.join("client.token")).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        // Missing file: an error carrying the data dir, never a panic.
        unsafe { std::env::set_var("TETHERLY_DATA_DIR", "/nonexistent-tetherly-shell-test") };
        let err = read_api_token().unwrap_err().to_string();
        assert!(err.contains("client.token"), "{err}");
        unsafe { std::env::remove_var("TETHERLY_DATA_DIR") };
    }
}
