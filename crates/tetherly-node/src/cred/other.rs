// SPDX-License-Identifier: Apache-2.0 OR MIT
//! No-vault backend for targets with no supported credential store.
//!
//! `available()` returns false, so `cred` transparently uses the on-disk
//! `identity.bin`. Self-contained: imports only absolute paths so this file
//! compiles for its target on its own.

pub fn available() -> bool {
    false
}

pub fn write(_target: &str, _blob: &[u8]) -> bool {
    false
}

pub fn read(_target: &str) -> Option<Vec<u8>> {
    None
}

pub fn delete(_target: &str) -> bool {
    true
}
