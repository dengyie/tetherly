// SPDX-License-Identifier: Apache-2.0 OR MIT
//! OS Shell Reveal and Security Tagging (Mark of the Web) pipeline.
//!
//! Complies with ADR-137 / Module File Transfer Spec:
//! - NEVER executes received files directly (`ShellExecute "open"`).
//! - Invokes native file manager to highlight and focus the item:
//!   - macOS: `open -R <path>`
//!   - Windows: `explorer.exe /select,<path>`
//!   - Linux: `nautilus --select <path>` / `xdg-open <parent>`
//! - On Windows NTFS, applies Mark of the Web (`Zone.Identifier` stream with ZoneId=3)
//!   to provide audit trails and smart defense.

use std::path::Path;
use std::process::Command;
use tetherly_core::CoreError;

/// Reveal and highlight a target file in the operating system's native file manager.
///
/// Safe: does NOT execute or open the file with its default application.
pub fn reveal_in_file_manager(path: &Path) -> Result<(), CoreError> {
    if !path.exists() {
        return Err(CoreError::File("target path does not exist".into()));
    }

    // Windows: `Path::canonicalize` returns an extended-length path
    // (`\\?\C:\...`, `\\?\UNC\...`) which `explorer /select,` does not
    // accept — the reveal would silently no-op. `std::path::absolute` keeps
    // the plain Win32 form. macOS/Linux keep canonicalize (resolves symlinks
    // for a stable highlight target).
    #[cfg(target_os = "windows")]
    let abs_path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(not(target_os = "windows"))]
    let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

    let path_str = abs_path.to_str().ok_or(CoreError::UnsafeFileName)?;

    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("open");
        cmd.arg("-R")
            .arg(path_str)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        spawn_reaped(cmd, "failed to reveal in Finder", "open -R")
    }

    #[cfg(target_os = "windows")]
    {
        let mut cmd = Command::new("explorer.exe");
        cmd.arg(format!("/select,{}", path_str))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        spawn_reaped(cmd, "failed to reveal in Explorer", "explorer /select")
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // Linux / BSD fallback: highlight the containing directory instead of
        // opening the file itself (never execute received payloads).
        let parent_str = match path_str.rsplit_once('/') {
            Some(("", _)) => "/", // file sits at filesystem root
            Some((parent, _)) => parent,
            None => ".",
        };
        let mut cmd = Command::new("xdg-open");
        cmd.arg(parent_str)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        spawn_reaped(cmd, "failed to open file manager", "xdg-open")
    }
}

/// Fire-and-forget spawn with correct child hygiene for a long-lived daemon:
/// a reaper thread collects the short-lived child (no zombie accumulation)
/// and its non-zero exit is surfaced as a warning instead of being silently
/// swallowed (`spawn` success does not mean the reveal succeeded).
fn spawn_reaped(
    mut cmd: Command,
    spawn_err_label: &str,
    what: &'static str,
) -> Result<(), CoreError> {
    let label = spawn_err_label.to_string();
    let mut child = cmd
        .spawn()
        .map_err(|e| CoreError::OpenRefused(format!("{label}: {e}")))?;
    let pid = child.id();
    std::thread::Builder::new()
        .name("shell-reveal-reap".into())
        .spawn(move || match child.wait() {
            Ok(status) if !status.success() => {
                tracing::warn!(
                    "{what} exited non-zero ({status}) — reveal likely did not happen (pid {pid})"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("{what} wait failed: {e} (pid {pid})"),
        })
        .ok();
    Ok(())
}

/// Apply Mark of the Web (MOTW) Zone.Identifier stream on Windows NTFS files.
///
/// Marks the received file with ZoneId=3 (Internet / Remote Zone) and metadata
/// indicating it arrived via Tetherly P2P transfer from `source_peer`.
///
/// On non-Windows platforms, this is a safe no-op.
pub fn apply_motw(file_path: &Path, source_peer: &str) -> Result<(), CoreError> {
    if !file_path.exists() {
        return Err(CoreError::File(
            "target file does not exist for MOTW".into(),
        ));
    }

    #[cfg(target_os = "windows")]
    {
        let clean_peer =
            source_peer.replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "");
        let zone_path = format!("{}:Zone.Identifier", file_path.display());
        let content = format!(
            "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=tetherly://peer/{clean_peer}\r\nHostUrl=tetherly://peer/{clean_peer}\r\n"
        );
        std::fs::write(&zone_path, content)
            .map_err(|e| CoreError::File(format!("failed to write Zone.Identifier: {e}")))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (file_path, source_peer);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reveal_fails_on_nonexistent_path() {
        let fake = Path::new("/path/to/definitely/nonexistent/file_12345.xyz");
        let res = reveal_in_file_manager(fake);
        assert!(res.is_err());
    }

    #[test]
    fn test_apply_motw_on_temp_file() {
        let tmp =
            std::env::temp_dir().join(format!("tetherly_test_motw_{}.tmp", std::process::id()));
        std::fs::write(&tmp, b"sample tetherly payload").expect("write temp");
        let res = apply_motw(&tmp, "home-win");
        let _ = std::fs::remove_file(&tmp);
        assert!(res.is_ok());
    }
}
