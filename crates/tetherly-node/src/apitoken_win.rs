//! Windows half of [`super::open_client_or_dashboard`].
//!
//! Two Windows-specific facts shape this file. First, both `cmd /C start`
//! and `ShellExecute` report *success* for a URL scheme nobody handles:
//! the shell pops the "You'll need a new app to open this tetherly link"
//! dialog and returns 0, so the deep link has to be gated on the scheme
//! actually being registered (`HKCR\tetherly\shell\open\command`, which
//! merges the per-user and machine class hives the Tauri bundler writes
//! to). Second, the tray panel child runs with `CREATE_NO_WINDOW`, so
//! spawning `cmd` from it would flash a console; `ShellExecuteW` launches
//! the handler or the default browser directly.

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Whether a `tetherly://` handler is registered for the calling user.
pub(super) fn client_scheme_registered() -> bool {
    let sub = wide("tetherly\\shell\\open\\command");
    let mut key: HKEY = std::ptr::null_mut();
    let rc = unsafe { RegOpenKeyExW(HKEY_CLASSES_ROOT, sub.as_ptr(), 0, KEY_READ, &mut key) };
    if rc != ERROR_SUCCESS {
        return false;
    }
    unsafe {
        RegCloseKey(key);
    }
    true
}

/// `ShellExecuteW("open", target)`: a deep link or an `http://` URL. The
/// API contract is "greater than 32 means the shell accepted it".
pub(super) fn shell_open(target: &str) -> bool {
    let verb = wide("open");
    let target = wide(target);
    let h = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    (h as isize) > 32
}
