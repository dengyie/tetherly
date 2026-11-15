// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Control API v1 event pipeline (ADR-139 M1): internal [`UiEvent`]s are
//! projected to stale-resource marks, coalesced, resolved to full-state
//! [`tetherly_api::Event`]s and fanned out with replayable sequence numbers.
//!
//! [`UiEvent`]: crate::events::UiEvent

pub mod coalesce;
pub mod hub;
pub mod journal;
pub mod model;
pub mod project;
