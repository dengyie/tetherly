// SPDX-License-Identifier: Apache-2.0 OR MIT
//! §238 shell file log: tray clicks and panel decisions must survive a
//! Finder/autostart launch (stderr goes nowhere there), so a dead-tray
//! report can be traced after the fact.

use std::path::{Path, PathBuf};

/// Rotate once the log passes this size (keeps one `.1` generation).
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Per-platform log file: macOS `~/Library/Logs/Tetherly/shell.log`,
/// Windows `%LOCALAPPDATA%\Tetherly\logs\shell.log`, else
/// `~/.local/share/tetherly/shell.log`. `None` without a home.
pub fn log_path(os: &str, home: Option<&Path>, local_app_data: Option<&Path>) -> Option<PathBuf> {
    match os {
        "macos" => home.map(|h| h.join("Library/Logs/Tetherly/shell.log")),
        "windows" => local_app_data.map(|d| d.join("Tetherly").join("logs").join("shell.log")),
        _ => home.map(|h| h.join(".local/share/tetherly/shell.log")),
    }
}

/// Whether an existing log of `len` bytes should be rotated before append.
pub fn needs_rotation(len: u64) -> bool {
    len > MAX_BYTES
}

/// Open the shell log for append (creating parents, rotating when large).
pub fn open() -> Option<(PathBuf, std::fs::File)> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let lad = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let path = log_path(std::env::consts::OS, home.as_deref(), lad.as_deref())?;
    std::fs::create_dir_all(path.parent()?).ok()?;
    if std::fs::metadata(&path)
        .map(|m| needs_rotation(m.len()))
        .unwrap_or(false)
    {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    let f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()?;
    Some((path, f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_log_lives_in_library_logs() {
        assert_eq!(
            log_path("macos", Some(Path::new("/Users/u")), None),
            Some(PathBuf::from("/Users/u/Library/Logs/Tetherly/shell.log"))
        );
    }

    #[test]
    fn windows_log_lives_in_local_app_data() {
        assert_eq!(
            log_path("windows", None, Some(Path::new("C:/L"))),
            Some(
                PathBuf::from("C:/L")
                    .join("Tetherly")
                    .join("logs")
                    .join("shell.log")
            )
        );
        assert_eq!(log_path("windows", Some(Path::new("/h")), None), None);
    }

    #[test]
    fn rotation_threshold() {
        assert!(!needs_rotation(MAX_BYTES));
        assert!(needs_rotation(MAX_BYTES + 1));
    }
}
