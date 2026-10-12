// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Linux/BSD Secret Service (libsecret) backend for `cred`.
//!
//! Stores the 64 secret bytes as an item in the user's default keyring over
//! D-Bus via `secret-service`, labelled `tetherly.identity` with the attribute
//! `device_id=<id>`. The blocking API is used, so calls are made off the async
//! runtime (identity load happens during `Node::start`, before any session).
//!
//! Availability is runtime-dependent: a headless server or a session with no
//! `org.freedesktop.secrets` provider (no gnome-keyring/KWallet) returns
//! `false` and the caller transparently uses `identity.bin`. A locked keyring
//! makes a read return `None` (again: file fallback).
//!
//! Self-contained: imports only absolute paths so this file compiles for the
//! Linux target on its own (see the module docs in the parent `cred`).

use secret_service::blocking::SecretService;
use secret_service::{EncryptionType, Error as SsError};
use std::collections::HashMap;

/// Attribute key that identifies our items.
const ATTR: &str = "device_id";
/// Item label (human-facing in Seahorse/KWallet).
const LABEL: &str = "tetherly.identity";

fn service() -> Option<SecretService<'static>> {
    SecretService::connect(EncryptionType::Plain).ok()
}

/// Whether a Secret Service provider is reachable over the session bus.
pub fn available() -> bool {
    service().is_some()
}

fn split(target: &str) -> &str {
    target.split_once(':').map(|(_, a)| a).unwrap_or(target)
}

pub fn write(target: &str, blob: &[u8]) -> bool {
    let device_id = split(target);
    let Some(svc) = service() else {
        return false;
    };
    let Ok(collection) = svc.get_default_collection() else {
        return false;
    };
    if matches!(collection.ensure_unlocked(), Err(SsError::Locked)) {
        return false;
    }
    let mut attrs = HashMap::new();
    attrs.insert(ATTR, device_id);
    // `replace = true` makes this an upsert keyed by the attributes. Bind the
    // result before the block ends: the returned `Item` borrows `collection`,
    // so the temporary must be dropped while `collection` is still alive.
    let created = collection.create_item(LABEL, attrs, blob, true, "application/octet-stream");
    created.is_ok()
}

pub fn read(target: &str) -> Option<Vec<u8>> {
    let device_id = split(target);
    let svc = service()?;
    let collection = svc.get_default_collection().ok()?;
    let mut attrs = HashMap::new();
    attrs.insert(ATTR, device_id);
    let items = collection.search_items(attrs).ok()?;
    let item = items.into_iter().next()?;
    item.get_secret().ok()
}

pub fn delete(target: &str) -> bool {
    let device_id = split(target);
    let Some(svc) = service() else {
        return true; // nothing we can reach is nothing we must delete
    };
    let Ok(collection) = svc.get_default_collection() else {
        return false;
    };
    let mut attrs = HashMap::new();
    attrs.insert(ATTR, device_id);
    let Ok(items) = collection.search_items(attrs) else {
        return false;
    };
    let mut all_ok = true;
    for item in items {
        if item.delete().is_err() {
            all_ok = false;
        }
    }
    all_ok
}
