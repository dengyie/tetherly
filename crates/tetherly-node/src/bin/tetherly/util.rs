// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Re-exports of the formatting helpers this binary prints with.
//!
//! The bodies used to live here and, in a second copy, in `tetherly-cli` — so a
//! fix to one printer silently left the other stale. They now live in
//! `tetherly-core::text`; this module keeps the `super::util::{...}` import
//! shape the handler modules already use.

pub(crate) use tetherly_core::text::{
    encode_path as urlencode_path, encode_query_value as url_encode, format_bytes, format_speed,
    format_timestamp_ago, format_uptime,
};
pub(crate) use tetherly_core::{now_ms, now_secs};
