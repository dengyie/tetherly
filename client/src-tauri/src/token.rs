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

/// Read the API token minted by the daemon: `<data_dir>/api.token`, 64 hex
/// chars, mode 0600. `TETHERLY_API_TOKEN` overrides, matching the CLI.
pub fn read_api_token() -> std::io::Result<String> {
    if let Ok(t) = std::env::var("TETHERLY_API_TOKEN") {
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let path = daemon_data_dir().join("api.token");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| std::io::Error::other(format!("{}: {e}", path.display())))?;
    let t = raw.trim().to_string();
    if t.is_empty() {
        return Err(std::io::Error::other(format!(
            "{}: empty token",
            path.display()
        )));
    }
    Ok(t)
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
        let _ = std::fs::remove_dir_all(&dir);

        // Missing file: an error carrying the path, never a panic.
        unsafe { std::env::set_var("TETHERLY_DATA_DIR", "/nonexistent-tetherly-shell-test") };
        let err = read_api_token().unwrap_err().to_string();
        assert!(err.contains("api.token"), "{err}");
        unsafe { std::env::remove_var("TETHERLY_DATA_DIR") };
    }
}
