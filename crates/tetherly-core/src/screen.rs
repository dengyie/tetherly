// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Remote-screen protocol. Binary frames only — never JSON pixels.
//! OS capture and presentation live in tetherly-node; this module is OS-free.
//!
//! One direction: the controlled side grabs frames and pushes them; the
//! controlling side presents them. Consent is explicit and local — a peer that
//! reaches the channel may NOT start a stream until this machine has allowed
//! it. Frames carry no executable instruction, so a peer URL can never ride in
//! on a screen payload (spec §10 / §9.7).
//!
//! Layout: every feature area is its own submodule (frame wire codec, control
//! envelope, adaptation, resilience, tiles, recording, …). Public names are
//! re-exported here, so `tetherly_core::screen::X` keeps working regardless of
//! which submodule defines `X`.

mod adapt;
mod annotation;
mod audio_sync;
mod color;
mod control;
mod cursor;
mod damage;
mod delta;
mod fec;
mod motion;
mod overlay;
mod pacing;
mod playback;
mod playout;
mod probe;
mod profiler;
mod qoe;
mod receiver;
mod recording;
mod resilience;
mod retransmit;
mod roi;
mod sender;
mod sink;
mod squelch;
mod tile;
mod tile_compress;
mod tile_lod;
mod tile_schedule;
mod tuner;
mod twcc;
mod watermark;
mod wire;

pub use adapt::*;
pub use annotation::*;
pub use audio_sync::*;
pub use color::*;
pub use control::*;
pub use cursor::*;
pub use damage::*;
pub use delta::*;
pub use fec::*;
pub use motion::*;
pub use overlay::*;
pub use pacing::*;
pub use playback::*;
pub use playout::*;
pub use probe::*;
pub use profiler::*;
pub use qoe::*;
pub use receiver::*;
pub use recording::*;
pub use resilience::*;
pub use retransmit::*;
pub use roi::*;
pub use sender::*;
pub use sink::*;
pub use squelch::*;
pub use tile::*;
pub use tile_compress::*;
pub use tile_lod::*;
pub use tile_schedule::*;
pub use tuner::*;
pub use twcc::*;
pub use watermark::*;
pub use wire::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_pipelines;
