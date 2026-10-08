// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Workspace root of Tetherly. Phase 0 has no GUI binary here; the CLI lives in
//! `bins/tetherly-cli`. This crate exists so `tests/loopback.rs` is a first-class
//! integration target as specified in `docs/DEVELOPMENT.md` §8.

pub use tetherly_core as core;
pub use tetherly_crypto as crypto;
pub use tetherly_net as net;
