// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Shared shelf ("共享架", v0.2.89): a per-device catalog of files the user
//! dragged onto the Transfers page. Only metadata crosses the wire; the file
//! itself is pulled on demand with [`ShelfMsg::Fetch`] and delivered by the
//! ordinary file-offer path.
//!
//! INVARIANTS:
//! - The catalog never carries local filesystem paths — only an opaque id,
//!   the display name, size and mtime.
//! - Every message is bounded by [`crate::frame::INNER_PAYLOAD_MAX`]; an
//!   oversized catalog is truncated (newest entries first) on encode and
//!   rejected on decode.

use crate::error::CoreError;
use serde::{Deserialize, Serialize};

/// Most entries one catalog frame may carry.
pub const SHELF_MAX_ENTRIES: usize = 200;
/// Longest display name accepted (bytes, UTF-8).
pub const SHELF_NAME_MAX: usize = 255;
/// Longest entry id accepted (bytes).
pub const SHELF_ID_MAX: usize = 32;

/// One shared file as seen by peers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShelfEntry {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mtime_ms: u64,
    /// False once the owner noticed the file changed or vanished.
    pub available: bool,
}

/// Why an owner refused a fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShelfRefusal {
    NotFound,
    Changed,
    Untrusted,
    Unreadable,
}

/// Shelf control plane, carried on [`crate::frame::TYPE_SHELF`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ShelfMsg {
    /// Full replacement of the sender's catalog.
    Catalog { entries: Vec<ShelfEntry> },
    /// Ask the owner to push one entry through a normal file offer.
    Fetch { id: String },
    /// The owner could not serve a fetch.
    FetchRefused { id: String, reason: ShelfRefusal },
}

fn entry_ok(e: &ShelfEntry) -> bool {
    !e.id.is_empty()
        && e.id.len() <= SHELF_ID_MAX
        && !e.name.is_empty()
        && e.name.len() <= SHELF_NAME_MAX
}

impl ShelfMsg {
    /// Serialize into an inner frame. A catalog that does not fit is trimmed
    /// from the end (callers order newest first).
    pub fn to_frame(&self) -> Result<crate::frame::InnerFrame, CoreError> {
        let mut msg = self.clone();
        if let ShelfMsg::Catalog { entries } = &mut msg {
            entries.retain(entry_ok);
            entries.truncate(SHELF_MAX_ENTRIES);
        }
        loop {
            let payload = serde_json::to_vec(&msg)?;
            if payload.len() <= crate::frame::INNER_PAYLOAD_MAX {
                return crate::frame::InnerFrame::new(crate::frame::TYPE_SHELF, payload);
            }
            match &mut msg {
                ShelfMsg::Catalog { entries } if !entries.is_empty() => {
                    let keep = entries.len() * 3 / 4;
                    entries.truncate(keep);
                }
                _ => return Err(CoreError::PayloadTooLarge),
            }
        }
    }

    /// Decode and validate an inbound payload.
    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        if payload.len() > crate::frame::INNER_PAYLOAD_MAX {
            return Err(CoreError::PayloadTooLarge);
        }
        let msg: ShelfMsg = serde_json::from_slice(payload)?;
        match &msg {
            ShelfMsg::Catalog { entries } => {
                if entries.len() > SHELF_MAX_ENTRIES || !entries.iter().all(entry_ok) {
                    return Err(CoreError::InvalidPayload("shelf catalog".into()));
                }
            }
            ShelfMsg::Fetch { id } | ShelfMsg::FetchRefused { id, .. } => {
                if id.is_empty() || id.len() > SHELF_ID_MAX {
                    return Err(CoreError::InvalidPayload("shelf id".into()));
                }
            }
        }
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(i: usize, name_len: usize) -> ShelfEntry {
        ShelfEntry {
            id: format!("{i:016x}"),
            name: "n".repeat(name_len),
            size: i as u64,
            mtime_ms: 1,
            available: true,
        }
    }

    #[test]
    fn catalog_round_trips() {
        let msg = ShelfMsg::Catalog {
            entries: vec![entry(1, 8), entry(2, 9)],
        };
        let frame = msg.to_frame().unwrap();
        assert_eq!(frame.ty, crate::frame::TYPE_SHELF);
        assert_eq!(ShelfMsg::from_payload(&frame.payload).unwrap(), msg);
    }

    #[test]
    fn oversized_catalog_is_trimmed_to_fit() {
        let entries: Vec<ShelfEntry> = (0..SHELF_MAX_ENTRIES + 50)
            .map(|i| entry(i, SHELF_NAME_MAX))
            .collect();
        let frame = ShelfMsg::Catalog { entries }.to_frame().unwrap();
        assert!(frame.payload.len() <= crate::frame::INNER_PAYLOAD_MAX);
        match ShelfMsg::from_payload(&frame.payload).unwrap() {
            ShelfMsg::Catalog { entries } => {
                assert!(!entries.is_empty() && entries.len() <= SHELF_MAX_ENTRIES);
                assert_eq!(entries[0].id, format!("{:016x}", 0));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn invalid_entries_are_rejected_on_decode() {
        let bad = serde_json::to_vec(&ShelfMsg::Catalog {
            entries: vec![entry(1, SHELF_NAME_MAX + 1)],
        })
        .unwrap();
        assert!(ShelfMsg::from_payload(&bad).is_err());
        let bad_id = serde_json::to_vec(&ShelfMsg::Fetch { id: String::new() }).unwrap();
        assert!(ShelfMsg::from_payload(&bad_id).is_err());
    }
}
