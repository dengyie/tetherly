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
/// File name of the client token inside the data dir (§4.4).
pub const CLIENT_TOKEN_FILE: &str = "client.token";
/// Request header carrying the token.
pub const TOKEN_HEADER: &str = "x-tetherly-token";

/// Path of the token file for `data_dir`.
pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

/// Path of the client token file for `data_dir` (§4.4). Same directory as
/// [`token_path`]; on Windows the SYSTEM-launched daemon passes the console
/// user's LOCALAPPDATA as `data_dir` (`--data-dir` from the launcher), so
/// the file lands in the user's profile with a user-SID-only DACL.
pub fn client_token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(CLIENT_TOKEN_FILE)
}

/// Access tier of a `/api/v1` request (ADR-139). Ordered: a grant of a
/// higher tier satisfies every lower one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    /// Observe state: every `GET`.
    Read,
    /// Change state: transfers, settings, layout, peers.
    Write,
    /// Process lifecycle and diagnostics knobs.
    Admin,
}

/// Paths whose non-`GET` methods need [`Tier::Admin`].
const V1_ADMIN_PATHS: &[&str] = &[
    "/api/v1/system/restart",
    "/api/v1/system/shutdown",
    "/api/v1/diagnostics/log-level",
    // A transfer path is read by the daemon under its own OS identity. Keep
    // the client token from turning that into a privileged file-read proxy.
    "/api/v1/transfers",
    "/api/v1/transfers/multi",
    "/api/v1/transfers/batch",
    "/api/v1/screen/snapshot",
    "/api/v1/screen/whiteboard/export",
];

/// Tier a `/api/v1` request needs, or `None` when it is open. Only
/// `GET /api/v1/health` is open, so a client can tell "daemon down" from
/// "token wrong". `path` may carry a query string.
pub fn v1_required_tier(method: &str, path: &str) -> Option<Tier> {
    let (path_only, query) = path
        .split_once('?')
        .map_or((path, None), |(p, q)| (p, Some(q)));
    let output_requested = query.is_some_and(|q| {
        q.split('&').any(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            key == "out" && !value.is_empty()
        })
    });
    if method == "GET" {
        if path_only == "/api/v1/health" {
            return None;
        }
        // Exporting to a caller-selected path is a privileged filesystem
        // side effect, even though the endpoint also has a read-only form.
        if output_requested
            && matches!(
                path_only,
                "/api/v1/screen/snapshot"
                    | "/api/v1/screen/recording/snapshot"
                    | "/api/v1/screen/whiteboard/export"
            )
        {
            return Some(Tier::Admin);
        }
        return Some(Tier::Read);
    }
    Some(
        if V1_ADMIN_PATHS.contains(&path_only) || path_only == "/api/v1/screen/recording/snapshot" {
            Tier::Admin
        } else {
            Tier::Write
        },
    )
}

/// Tier a presented credential grants. The daemon-minted token grants
/// [`Tier::Admin`]; the §4.4 client token grants [`Tier::Write`] (read +
/// write, never admin); anything else grants nothing.
pub fn granted_tier(daemon: &str, client: Option<&str>, presented: &str) -> Option<Tier> {
    if token_matches(daemon, presented) {
        return Some(Tier::Admin);
    }
    token_matches(client.unwrap_or(""), presented).then_some(Tier::Write)
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

/// Mint the §4.4 client token (grants [`Tier::Write`]) and replace
/// `<data_dir>/client.token` with it, same `CREATE_NEW` discipline as
/// [`issue`]. Unix: 0600. Windows: protected DACL granting full control to
/// the console user's SID and SYSTEM only — the caller passes the console
/// user's LOCALAPPDATA as `data_dir`, so the file is readable by the shell
/// running as that user and by the SYSTEM daemon, nobody else.
pub fn issue_client(data_dir: &Path) -> std::io::Result<String> {
    let mut raw = [0u8; 32];
    getrandom::getrandom(&mut raw).map_err(std::io::Error::other)?;
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    let path = client_token_path(data_dir);
    let mut last_err = None;
    for _ in 0..5 {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        match create_client_private(&path) {
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
    Err(last_err.unwrap_or_else(|| std::io::Error::other("client token create raced")))
}

/// Read the client token file for shell use. `None` when missing, unreadable
/// or malformed — or (Unix) when the file's permissions are wider than
/// owner-only, so a token a foreign principal could read is refused.
pub fn read_client(data_dir: &Path) -> Option<String> {
    let path = client_token_path(data_dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).ok()?.permissions().mode();
        if mode & 0o077 != 0 {
            return None;
        }
    }
    let s = std::fs::read_to_string(path).ok()?;
    let s = s.trim();
    is_token(s).then(|| s.to_string())
}

#[cfg(unix)]
fn create_client_private(path: &Path) -> std::io::Result<std::fs::File> {
    create_private(path)
}

/// Windows client-token DACL: full control for SYSTEM, Administrators and
/// the file owner (the console user, because the daemon writes into *their*
/// LOCALAPPDATA with `--data-dir`); no inherited ACEs. The generic
/// `api.token` DACL already is exactly that, so it is reused.
#[cfg(windows)]
fn create_client_private(path: &Path) -> std::io::Result<std::fs::File> {
    create_private(path)
}

#[cfg(not(any(unix, windows)))]
fn create_client_private(path: &Path) -> std::io::Result<std::fs::File> {
    create_private(path)
}

/// Dashboard URL that hands the token to the page via the URL fragment
/// (fragments never reach the server or its logs).
pub fn dashboard_url(ui_port: u16, token: Option<&str>) -> String {
    match token {
        Some(t) => format!("http://127.0.0.1:{ui_port}/#token={t}"),
        None => format!("http://127.0.0.1:{ui_port}/"),
    }
}

/// Same contract as [`dashboard_url`] for the ADR-139 SPA. Since M4 the
/// SPA IS the console at `/`; the CLI flag now exists only for
/// compatibility and both URLs point at the same entry.
pub fn next_url(ui_port: u16, token: Option<&str>) -> String {
    match token {
        Some(t) => format!("http://127.0.0.1:{ui_port}/#token={t}"),
        None => format!("http://127.0.0.1:{ui_port}/"),
    }
}

#[cfg(windows)]
#[path = "apitoken_win.rs"]
mod win;

/// M4: the shell owns the tray surface (§6.3), so tray clicks prefer the
/// Tauri client over the browser: open the `tetherly://open` deep link
/// first (the shell shows its main window; the bundler registered the
/// scheme), falling back to the tokenized dashboard URL when no client is
/// installed. On Windows the deep link is only attempted when the scheme
/// is registered, because `start`/`ShellExecute` answer "success" for an
/// unknown scheme while showing the "You'll need a new app" dialog (the
/// §228 "open main window errors" report); both launches go through
/// `ShellExecuteW` so the console-less tray panel child never flashes a
/// `cmd` window. The browser path never refuses, matching
/// `tetherly dashboard --open`.
pub fn open_client_or_dashboard(ui_port: u16, token: Option<&str>) -> bool {
    #[cfg(target_os = "macos")]
    let client_ok = std::process::Command::new("open")
        .arg("tetherly://open")
        .status()
        .is_ok_and(|s| s.success());
    #[cfg(windows)]
    let client_ok = win::client_scheme_registered() && win::shell_open("tetherly://open");
    #[cfg(not(any(target_os = "macos", windows)))]
    let client_ok = std::process::Command::new("xdg-open")
        .arg("tetherly://open")
        .status()
        .is_ok_and(|s| s.success());
    if client_ok {
        tracing::info!("open main window: desktop client via tetherly://open");
        return true;
    }
    #[cfg(windows)]
    tracing::warn!(
        scheme_registered = win::client_scheme_registered(),
        "open main window: desktop client unavailable; falling back to the dashboard"
    );
    #[cfg(not(windows))]
    tracing::warn!("open main window: desktop client unavailable; falling back to the dashboard");
    let url = dashboard_url(ui_port, token);
    #[cfg(target_os = "macos")]
    let opened = std::process::Command::new("open").arg(&url).spawn().is_ok();
    #[cfg(windows)]
    let opened = win::shell_open(&url);
    #[cfg(not(any(target_os = "macos", windows)))]
    let opened = std::process::Command::new("xdg-open")
        .arg(&url)
        .spawn()
        .is_ok();
    if !opened {
        tracing::warn!("open main window: dashboard fallback failed to launch");
    }
    opened
}

#[cfg(test)]
mod tests {
    #[test]
    fn v1_tiers_classify_health_open_reads_read_writes_write_admin_admin() {
        use super::{v1_required_tier as t, Tier};
        assert_eq!(t("GET", "/api/v1/health"), None);
        assert_eq!(t("GET", "/api/v1/health?x=1"), None);
        assert_eq!(t("GET", "/api/v1/system"), Some(Tier::Read));
        assert_eq!(t("GET", "/api/v1/events"), Some(Tier::Read));
        assert_eq!(t("POST", "/api/v1/transfers"), Some(Tier::Admin));
        assert_eq!(t("POST", "/api/v1/transfers/multi"), Some(Tier::Admin));
        assert_eq!(t("POST", "/api/v1/transfers/batch"), Some(Tier::Admin));
        assert_eq!(t("POST", "/api/v1/screen/snapshot"), Some(Tier::Admin));
        assert_eq!(
            t("POST", "/api/v1/screen/whiteboard/export"),
            Some(Tier::Admin)
        );
        assert_eq!(t("PATCH", "/api/v1/settings"), Some(Tier::Write));
        assert_eq!(
            t("GET", "/api/v1/screen/snapshot?out=escape.bmp"),
            Some(Tier::Admin)
        );
        assert_eq!(t("POST", "/api/v1/health"), Some(Tier::Write));
        assert_eq!(t("POST", "/api/v1/system/restart"), Some(Tier::Admin));
        assert_eq!(
            t("PATCH", "/api/v1/diagnostics/log-level"),
            Some(Tier::Admin)
        );
        assert!(Tier::Read < Tier::Write && Tier::Write < Tier::Admin);
    }

    #[test]
    fn granted_tier_is_admin_for_the_daemon_token_and_none_otherwise() {
        use super::{granted_tier, Tier};
        let tok = "a".repeat(64);
        assert_eq!(granted_tier(&tok, None, &tok), Some(Tier::Admin));
        assert_eq!(granted_tier(&tok, None, &"b".repeat(64)), None);
        assert_eq!(granted_tier(&tok, None, ""), None);
        assert_eq!(
            granted_tier("", None, ""),
            None,
            "unminted token fails closed"
        );
    }

    /// §4.4: the client token the daemon mints for the interactive user
    /// grants Write (read+write, no admin); the daemon token still grants
    /// Admin. Either token satisfies the tier the other cannot.
    #[test]
    fn granted_tier_is_write_for_the_client_token() {
        use super::{granted_tier, Tier};
        let (daemon, client) = ("a".repeat(64), "c".repeat(64));
        assert_eq!(
            granted_tier(&daemon, Some(&client), &client),
            Some(Tier::Write)
        );
        assert_eq!(
            granted_tier(&daemon, Some(&client), &daemon),
            Some(Tier::Admin)
        );
        assert_eq!(granted_tier(&daemon, Some(&client), &"b".repeat(64)), None);
        // An unminted client token slot fails closed.
        assert_eq!(granted_tier(&daemon, None, &client), None);
        assert_eq!(granted_tier(&daemon, Some(&client), ""), None);
    }

    #[test]
    fn v1_admin_routes_refuse_the_client_token_via_tier_math() {
        use super::{v1_required_tier as t, Tier};
        // Write-tier routes accept a Write grant; admin routes need Admin.
        assert!(t("PATCH", "/api/v1/settings").unwrap() <= Tier::Write);
        assert!(t("POST", "/api/v1/system/restart").unwrap() > Tier::Write);
    }

    /// Pin `V1_ADMIN_PATHS` to routes the v1 router actually answers. The
    /// list may name paths that are not wired yet (they gate future
    /// endpoints), but every admin-classified path must be reachable via
    /// `uihttp::v1::allowed` once it lands — and vice versa: a new admin
    /// route added to the router without tier classification would silently
    /// downgrade to Write. Kept in sync via this table.
    #[test]
    fn v1_admin_path_table_stays_pinned_to_the_tier_contract() {
        use super::V1_ADMIN_PATHS;
        // The only admin-classified v1 route that exists today.
        assert_eq!(
            v1_required_tier("PATCH", "/api/v1/diagnostics/log-level"),
            Some(Tier::Admin)
        );
        // Lifecycle routes are classified ahead of their router wiring so a
        // future endpoint cannot ship at Write tier by accident.
        assert_eq!(
            v1_required_tier("POST", "/api/v1/system/restart"),
            Some(Tier::Admin)
        );
        assert_eq!(
            v1_required_tier("POST", "/api/v1/system/shutdown"),
            Some(Tier::Admin)
        );
        assert_eq!(V1_ADMIN_PATHS.len(), 8, "new admin paths extend this pin");
    }

    #[test]
    fn client_token_issue_and_rotation() {
        let dir = temp_dir("client");
        let a = issue_client(&dir).expect("client token minted");
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(read_client(&dir).as_deref(), Some(a.as_str()));
        let b = issue_client(&dir).expect("rotated");
        assert_ne!(a, b, "every start rotates the client token");
        assert_eq!(read_client(&dir).as_deref(), Some(b.as_str()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(client_token_path(&dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "client.token stays owner-only");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The two token files are independent: minting one never clobbers the
    /// other, and a planted client.token is replaced like a planted api.token.
    #[test]
    fn client_token_is_independent_of_daemon_token() {
        let dir = temp_dir("independent");
        let daemon = issue(&dir).unwrap();
        let client = issue_client(&dir).unwrap();
        assert_ne!(daemon, client);
        assert_eq!(read(&dir).as_deref(), Some(daemon.as_str()));
        assert_eq!(read_client(&dir).as_deref(), Some(client.as_str()));
        std::fs::write(client_token_path(&dir), "0".repeat(64)).unwrap();
        let fresh = issue_client(&dir).unwrap();
        assert_ne!(fresh, "0".repeat(64));
        assert_eq!(read_client(&dir).as_deref(), Some(fresh.as_str()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §4.4: only user-writable files count as client tokens. A `client.token`
    /// whose owner differs from the reader (a foreign SID on Windows) must
    /// refuse — modeled on Unix by a group/world-readable file.
    #[cfg(unix)]
    #[test]
    fn read_client_rejects_a_world_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("foreign");
        std::fs::write(client_token_path(&dir), "d".repeat(64)).unwrap();
        let path = client_token_path(&dir);
        let mut perm = std::fs::metadata(&path).unwrap().permissions();
        perm.set_mode(0o644);
        std::fs::set_permissions(&path, perm).unwrap();
        assert_eq!(read_client(&dir), None, "world-readable file refused");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `read_client` follows the same trim/is_token rules as `read`.
    #[test]
    fn read_client_rejects_malformed_file() {
        let dir = temp_dir("client-malformed");
        assert_eq!(read_client(&dir), None);
        std::fs::write(client_token_path(&dir), "short").unwrap();
        assert_eq!(read_client(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Windows resolves the client token inside the *console user's* profile
    /// (%LOCALAPPDATA%\tetherly), not the SYSTEM systemprofile tree; Unix
    /// keeps it next to api.token. This test pins the Unix shape and the
    /// daemon-side env override (the Windows mapping is pinned in §4.4 docs).
    #[test]
    fn client_token_sits_beside_the_daemon_token_by_default() {
        let dir = temp_dir("colocated");
        assert_eq!(
            client_token_path(&dir),
            dir.join("client.token"),
            "same dir as api.token; the Windows SYSTEM daemon writes it to \
             the console user's LOCALAPPDATA via the caller-supplied data dir"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

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
        // M4: the SPA is the console; next_url kept for CLI compat and
        // points at the same entry as dashboard_url.
        assert_eq!(next_url(45716, None), "http://127.0.0.1:45716/");
        assert_eq!(
            next_url(45716, Some("abc")),
            "http://127.0.0.1:45716/#token=abc"
        );
    }
}
