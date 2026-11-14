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

/// Mark a file received from a peer as coming from outside this machine:
/// MOTW on Windows, `com.apple.quarantine` on macOS. Best effort — a failure
/// is logged, the transfer itself already succeeded.
pub fn mark_received_file(file_path: &Path, source_peer: &str) {
    #[cfg(target_os = "windows")]
    {
        if let Err(e) = apply_motw(file_path, source_peer) {
            tracing::warn!(path = %file_path.display(), error = %e, "MOTW not applied to received file");
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        let _ = source_peer;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        // 0081 = downloaded + user-approved-open not yet granted.
        let value = format!("0081;{secs:x};Tetherly;");
        // Direct setxattr(2): no subprocess on the receive path.
        let rc = match std::ffi::CString::new(file_path.as_os_str().as_bytes()) {
            // SAFETY: both strings are NUL-terminated and outlive the call;
            // `value` is passed with its exact length.
            Ok(cpath) => unsafe {
                libc::setxattr(
                    cpath.as_ptr(),
                    c"com.apple.quarantine".as_ptr(),
                    value.as_ptr().cast(),
                    value.len(),
                    0,
                    0,
                )
            },
            Err(_) => -1,
        };
        if rc != 0 {
            tracing::warn!(
                path = %file_path.display(),
                error = %std::io::Error::last_os_error(),
                "quarantine attribute not applied to received file"
            );
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let _ = (file_path, source_peer);
    }
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

#[cfg(all(test, target_os = "macos"))]
mod quarantine_tests {
    /// Review R2: inbox files carried no origin mark, so Gatekeeper never
    /// applied to them on open.
    #[test]
    fn received_file_gets_the_quarantine_attribute() {
        let p = std::env::temp_dir().join(format!("tetherly-q-{}.bin", std::process::id()));
        std::fs::write(&p, b"x").unwrap();
        super::mark_received_file(&p, "tdev_aaaaaaaaaaaaaaaa");
        let out = std::process::Command::new("xattr")
            .args(["-p", "com.apple.quarantine"])
            .arg(&p)
            .output()
            .unwrap();
        let _ = std::fs::remove_file(&p);
        assert!(out.status.success(), "quarantine attribute missing");
        assert!(String::from_utf8_lossy(&out.stdout).contains("Tetherly"));
    }
}
