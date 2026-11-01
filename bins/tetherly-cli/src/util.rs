// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Re-exports of the formatting and loopback-client helpers this binary prints
//! with.
//!
//! These used to be bodies: the byte/duration formatters, the percent-encoders
//! and the whole loopback HTTP/SSE client were copy-pasted from `tetherly`'s
//! binaries, so each copy drifted (and neither had a timeout). The bodies now
//! live in `tetherly-core::text` / `tetherly-core::ports` and
//! `tetherly-loopback`; this module keeps the `super::util::{...}` import shape
//! the 29 handler modules already use.

pub(crate) use tetherly_core::text::{
    encode_path as urlencode_path, encode_query_value as url_encode, format_bytes, format_speed,
    format_timestamp_ago, format_uptime,
};
pub(crate) use tetherly_core::{now_ms, now_secs};
pub(crate) use tetherly_loopback::{
    loopback_delete, loopback_events_connect, loopback_get, loopback_post,
};
