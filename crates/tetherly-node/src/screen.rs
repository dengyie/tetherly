// SPDX-License-Identifier: Apache-2.0 OR MIT
//! OS screen capture and presentation stubs.
//!
//! Real capture (DXGI Desktop Duplication on Windows, core-graphics on macOS,
//! x11rb on Linux) remains **Manual-required** — see docs §9.7 and the Phase 5
//! milestone table. These stubs return `ScreenRefused` honestly and are the
//! only code that lives in tetherly-node (core stays OS-free and
//! `#![forbid(unsafe_code)]`).
//!
//! Tests inject `tetherly_core::MemoryScreenSource` via
//! `NodeConfig::screen_source`; the receiver side always presents into an
//! in-memory sink observed through `Node::screen_sink_snapshot`. Both sides are
//! deterministic and carry no OS dependency.

use tetherly_core::{CoreError, ScreenFrame, ScreenSink, ScreenSource};

/// Production screen source. Returns `ScreenRefused` until per-platform DXGI /
/// CGImage / X11 SHM capture is implemented (Manual-required in CI-slice scope).
/// Tests should inject `MemoryScreenSource` instead.
#[derive(Debug)]
pub struct SystemSource;

impl ScreenSource for SystemSource {
    fn grab(&self) -> Result<ScreenFrame, CoreError> {
        Err(CoreError::ScreenRefused(
            "system screen capture is Manual-required; see tetherly docs §9.7".into(),
        ))
    }
}

/// Production screen sink. Returns `ScreenRefused` until per-platform window
/// display is implemented (Manual-required in CI-slice scope). Tests should
/// inject `MemoryScreenSink` instead.
#[derive(Debug)]
pub struct SystemSink;

impl ScreenSink for SystemSink {
    fn present(&self, _frame: &ScreenFrame) -> Result<(), CoreError> {
        Err(CoreError::ScreenRefused(
            "system screen present is Manual-required; see tetherly docs §9.7".into(),
        ))
    }
}
