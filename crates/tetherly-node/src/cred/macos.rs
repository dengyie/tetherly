// SPDX-License-Identifier: Apache-2.0 OR MIT
//! macOS Keychain backend for `cred`.
//!
//! Stores the 64 secret bytes as a keychain **generic password** via
//! `security-framework` (`set_generic_password`/`generic_password`/
//! `delete_generic_password`), keyed by service `tetherly.identity` and account
//! `<device_id>`. On first use the login keychain may prompt for access; we
//! treat any failure as "vault unavailable" and let the caller fall back to the
//! on-disk file so startup never blocks on the keychain.
//!
//! Self-contained: imports only absolute paths so this file compiles for the
//! macOS target on its own (see the module docs in the parent `cred`).

use security_framework::passwords::{
    delete_generic_password, generic_password, set_generic_password, PasswordOptions,
};
use security_framework_sys::base::errSecItemNotFound;

/// The vault is *available* (the API exists), but a specific call may still
/// fail — e.g. a headless session with a locked/absent login keychain. Callers
/// fall back to the file when a read returns `None`.
pub fn available() -> bool {
    true
}

/// Split `tetherly.identity:<device_id>` into `(service, account)`.
fn split(target: &str) -> (&str, &str) {
    match target.split_once(':') {
        Some((service, account)) => (service, account),
        None => (target, ""),
    }
}

pub fn write(target: &str, blob: &[u8]) -> bool {
    let (service, account) = split(target);
    set_generic_password(service, account, blob).is_ok()
}

pub fn read(target: &str) -> Option<Vec<u8>> {
    let (service, account) = split(target);
    // Distinguish "not found" (normal: caller falls back) from any other error
    // (also treated as unavailable here, never fatal).
    match generic_password(PasswordOptions::new_generic_password(service, account)) {
        Ok(bytes) => Some(bytes),
        Err(err) if err.code() == errSecItemNotFound => None,
        Err(_) => None,
    }
}

pub fn delete(target: &str) -> bool {
    let (service, account) = split(target);
    match delete_generic_password(service, account) {
        Ok(()) => true,
        Err(err) if err.code() == errSecItemNotFound => true,
        Err(_) => false,
    }
}
