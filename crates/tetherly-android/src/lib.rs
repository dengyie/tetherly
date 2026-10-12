// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Android client for Tetherly. Builds `libtetherly_android.so` for the phone
//! (`cargo ndk -t arm64-v8a build --release`) and compiles on the host for the
//! integration test that drives the real pairing path. All OS-independent: the
//! phone's TCP + the shared core/crypto/net stack. The only platform-specific
//! code is the JNI shim, gated to `target_os = "android"`.

#![deny(unsafe_code)]

pub mod client;
pub mod store;

#[cfg(target_os = "android")]
mod jni;

pub use client::{
    rtt_bench, AndroidClient, ClientError, SessionHandle, STATE_CONNECTED, STATE_CONNECTING,
    STATE_ERROR, STATE_IDLE,
};
pub use store::StoreError;

/// Percentile (nearest-rank, `p` in 0.0..=1.0) of a sample set, in the sample
/// unit. Returns `None` for an empty slice. Used by the M1.1 latency harness.
pub fn percentile(samples: &[u64], p: f64) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let mut s = samples.to_vec();
    s.sort_unstable();
    let rank = ((p * s.len() as f64).ceil() as usize).clamp(1, s.len());
    Some(s[rank - 1])
}
