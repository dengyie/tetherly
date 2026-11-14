//! Local API token for privileged loopback endpoints (P0-1, v0.2.52).
//!
//! The UI/API listener binds 127.0.0.1 and rejects foreign Host/Origin, which
//! stops browsers and remote peers — but not another *local* process. On
//! Windows the daemon runs as LocalSystem, so an unprivileged local process
//! calling `POST /api/action/local` or `/api/daemon/restart` would execute as
//! SYSTEM. A small set of privileged routes therefore requires the
//! `X-Tetherly-Token` header (or `Authorization: Bearer`).
//!
//! The token is minted fresh on every daemon start and written to
//! `<data_dir>/api.token`, readable only by the daemon's principal and
//! administrators (Windows: protected DACL SYSTEM + Administrators + owner,
//! created atomically with `CREATE_NEW`; Unix: 0600 inside the 0700 data
//! dir). Rotating per start means a file planted by a non-admin user in the
//! (user-writable) data dir can never pin a known token.

use std::path::{Path, PathBuf};

/// File name of the token inside the data dir.
pub const TOKEN_FILE: &str = "api.token";
/// Request header carrying the token.
pub const TOKEN_HEADER: &str = "x-tetherly-token";

/// Path of the token file for `data_dir`.
pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

/// True when `(method, path)` is a privileged route that needs the token.
///
/// `path` may carry a query string; only the path part is classified, with
/// the same exact-match semantics as the router. Scope (user decision
/// 2026-11-15, "privileged endpoints only"): process lifecycle, local command
/// definitions and the policy that lets peers run them, config and log-level
/// writes, opening received files, and sending files from arbitrary paths.
pub fn requires_token(method: &str, path: &str) -> bool {
    let path = path.split_once('?').map_or(path, |(p, _)| p);
    matches!(
        (method, path),
        (
            "POST",
            "/api/daemon/shutdown"
                | "/api/daemon/stop"
                | "/api/daemon/restart"
                | "/api/restart"
                | "/api/file/inbox/open"
                | "/api/inbox/open"
                | "/api/handoff/inbox/open"
                | "/api/handoff/open"
                | "/api/file/send"
                | "/api/file/send-multi"
                | "/api/file/send-batch"
                | "/api/file/batch"
                | "/api/file/drag/intent"
                | "/api/files/drag/intent"
        ) | (
            "POST" | "PUT",
            "/api/action/policy" | "/api/config" | "/api/log/level"
        ) | ("POST" | "PUT" | "DELETE", "/api/action/local")
    )
}

/// Constant-time comparison of a presented token with the expected one.
/// An empty expected token never matches (fail closed).
pub fn token_matches(expected: &str, presented: &str) -> bool {
    let (a, b) = (expected.as_bytes(), presented.as_bytes());
    if a.is_empty() || a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn is_token(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

/// Mint a fresh token, replace `<data_dir>/api.token` with it and return it.
///
/// The old file is removed first and the new one is created with
/// `CREATE_NEW` semantics, so a pre-planted file (or an attacker-held handle
/// on it) can never observe the new token.
pub fn issue(data_dir: &Path) -> std::io::Result<String> {
    let mut raw = [0u8; 32];
    getrandom::getrandom(&mut raw).map_err(std::io::Error::other)?;
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let path = token_path(data_dir);
    let mut last_err = None;
    for _ in 0..5 {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match create_private(&path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(token.as_bytes())?;
                f.sync_all()?;
                return Ok(token);
            }
            // Someone re-created the file between remove and create: retry.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => last_err = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last_err.unwrap_or_else(|| std::io::Error::other("api token create raced")))
}

#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

/// Create `path` with a protected DACL (no inherited ACEs): full control for
/// SYSTEM, Administrators and the file owner only. Set atomically at creation
/// through `SECURITY_ATTRIBUTES`, so there is no window where the inherited
/// user-profile ACL (which grants the interactive user) applies.
#[cfg(windows)]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{LocalFree, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE,
    };

    let sddl: Vec<u16> = "D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;OW)\0"
        .encode_utf16()
        .collect();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: sddl is NUL-terminated; sd receives a LocalAlloc'd descriptor
    // that is freed below on every path.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd,
        bInheritHandle: 0,
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: wide is NUL-terminated; sa outlives the call.
    let h = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_GENERIC_WRITE,
            0,
            &sa,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    let err = std::io::Error::last_os_error();
    // SAFETY: sd came from ConvertStringSecurityDescriptor...W (LocalAlloc).
    unsafe { LocalFree(sd as _) };
    if h == INVALID_HANDLE_VALUE {
        return Err(err);
    }
    // SAFETY: h is a fresh, owned, valid file handle.
    Ok(unsafe { std::fs::File::from_raw_handle(h as _) })
}

#[cfg(not(any(unix, windows)))]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// Read the token file for client use (CLI / scripts). `None` when missing,
/// unreadable (e.g. a non-elevated Windows process) or malformed.
pub fn read(data_dir: &Path) -> Option<String> {
    let s = std::fs::read_to_string(token_path(data_dir)).ok()?;
    let s = s.trim();
    is_token(s).then(|| s.to_string())
}

/// Dashboard URL that hands the token to the page via the URL fragment
/// (fragments never reach the server or its logs).
pub fn dashboard_url(ui_port: u16, token: Option<&str>) -> String {
    match token {
        Some(t) => format!("http://127.0.0.1:{ui_port}/#token={t}"),
        None => format!("http://127.0.0.1:{ui_port}/"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "tetherly-apitoken-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn privileged_routes_require_token() {
        for (m, p) in [
            ("POST", "/api/daemon/shutdown"),
            ("POST", "/api/daemon/stop"),
            ("POST", "/api/daemon/restart"),
            ("POST", "/api/restart"),
            ("POST", "/api/action/local"),
            ("PUT", "/api/action/local"),
            ("DELETE", "/api/action/local?id=x"),
            ("POST", "/api/action/policy"),
            ("PUT", "/api/action/policy"),
            ("POST", "/api/config"),
            ("PUT", "/api/config"),
            ("POST", "/api/log/level"),
            ("PUT", "/api/log/level"),
            ("POST", "/api/file/inbox/open"),
            ("POST", "/api/inbox/open"),
            ("POST", "/api/handoff/inbox/open"),
            ("POST", "/api/handoff/open"),
            ("POST", "/api/file/send"),
            ("POST", "/api/file/send-multi"),
            ("POST", "/api/file/send-batch"),
            ("POST", "/api/file/batch"),
            ("POST", "/api/file/drag/intent"),
            ("POST", "/api/files/drag/intent"),
        ] {
            assert!(requires_token(m, p), "{m} {p} must be gated");
        }
    }

    #[test]
    fn everyday_routes_stay_open() {
        for (m, p) in [
            ("GET", "/"),
            ("GET", "/api/health"),
            ("GET", "/api/state"),
            ("GET", "/api/config"),
            ("GET", "/api/action/local"),
            ("GET", "/api/action/policy"),
            ("GET", "/api/log/tail?lines=5"),
            ("POST", "/api/action/trigger"),
            ("POST", "/api/handoff/send"),
            ("POST", "/api/copy"),
            ("POST", "/api/open"),
            ("POST", "/api/file/cancel"),
            ("GET", "/api/file/inbox"),
            ("POST", "/api/input/ensure"),
            ("POST", "/api/daemon/restart/"),
        ] {
            assert!(!requires_token(m, p), "{m} {p} must stay open");
        }
    }

    #[test]
    fn token_comparison_is_exact() {
        let t = "ab".repeat(32);
        assert!(token_matches(&t, &t));
        assert!(!token_matches(&t, &t[..63]));
        assert!(!token_matches(&t, &"ac".repeat(32)));
        assert!(!token_matches(&t, ""));
        assert!(!token_matches("", ""));
    }

    #[test]
    fn issue_writes_fresh_hex_token_readable_by_owner() {
        let dir = temp_dir("issue");
        let a = issue(&dir).unwrap();
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(read(&dir).as_deref(), Some(a.as_str()));
        let b = issue(&dir).unwrap();
        assert_ne!(a, b, "every start rotates the token");
        assert_eq!(read(&dir).as_deref(), Some(b.as_str()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(token_path(&dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn issue_replaces_a_planted_file() {
        let dir = temp_dir("planted");
        std::fs::write(token_path(&dir), "0".repeat(64)).unwrap();
        let t = issue(&dir).unwrap();
        assert_ne!(t, "0".repeat(64));
        assert_eq!(read(&dir).as_deref(), Some(t.as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_rejects_malformed_file() {
        let dir = temp_dir("malformed");
        assert_eq!(read(&dir), None);
        std::fs::write(token_path(&dir), "not-a-token").unwrap();
        assert_eq!(read(&dir), None);
        std::fs::write(token_path(&dir), format!("{}\n", "cd".repeat(32))).unwrap();
        assert_eq!(read(&dir), Some("cd".repeat(32)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dashboard_url_carries_token_in_fragment() {
        assert_eq!(dashboard_url(45716, None), "http://127.0.0.1:45716/");
        assert_eq!(
            dashboard_url(45716, Some("abc")),
            "http://127.0.0.1:45716/#token=abc"
        );
    }
}
