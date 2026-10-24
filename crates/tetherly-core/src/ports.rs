// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::device_id::DeviceId;
use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    fn unix_ms(&self) -> u64;
    fn unix_us(&self) -> u64 {
        self.unix_ms() * 1000
    }
    fn now(&self) -> SystemTime {
        UNIX_EPOCH + std::time::Duration::from_millis(self.unix_ms())
    }
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn unix_us(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }
}

/// Injectable clock for production tests and lockout/TTL checks.
#[derive(Clone, Debug)]
pub struct ManualClock {
    ms: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl ManualClock {
    pub fn new(ms: u64) -> Self {
        Self {
            ms: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(ms)),
        }
    }

    pub fn set(&self, ms: u64) {
        self.ms.store(ms, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn unix_ms(&self) -> u64 {
        self.ms.load(std::sync::atomic::Ordering::SeqCst)
    }
}

pub trait Rng: Send + Sync {
    fn fill_bytes(&self, dest: &mut [u8]) -> Result<(), CoreError>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedPeer {
    pub device_id: DeviceId,
    pub id_pk: [u8; 32],
    pub n_pk: [u8; 32],
    pub alias: String,
    pub paired_at: u64,
    pub revoked: bool,
}

pub trait TrustStore: Send + Sync {
    fn get(&self, id: &DeviceId) -> Option<TrustedPeer>;
    fn put(&mut self, peer: TrustedPeer) -> Result<(), CoreError>;
    fn forget(&mut self, id: &DeviceId) -> Result<(), CoreError>;
    fn remove(&mut self, id: &DeviceId) -> Result<(), CoreError>;
    fn update_alias(&mut self, id: &DeviceId, alias: &str) -> Result<bool, CoreError>;
}

#[derive(Debug, Default)]
pub struct MemoryTrustStore {
    pub peers: std::collections::HashMap<DeviceId, TrustedPeer>,
}

impl MemoryTrustStore {
    pub fn all(&self) -> Vec<TrustedPeer> {
        self.peers.values().cloned().collect()
    }

    pub fn load(&mut self, peers: Vec<TrustedPeer>) {
        for p in peers {
            self.peers.insert(p.device_id.clone(), p);
        }
    }
}

impl TrustStore for MemoryTrustStore {
    fn get(&self, id: &DeviceId) -> Option<TrustedPeer> {
        self.peers.get(id).cloned()
    }

    fn put(&mut self, peer: TrustedPeer) -> Result<(), CoreError> {
        self.peers.insert(peer.device_id.clone(), peer);
        Ok(())
    }

    fn forget(&mut self, id: &DeviceId) -> Result<(), CoreError> {
        if let Some(p) = self.peers.get_mut(id) {
            p.revoked = true;
        } else {
            self.peers.remove(id);
        }
        Ok(())
    }

    fn remove(&mut self, id: &DeviceId) -> Result<(), CoreError> {
        self.peers.remove(id);
        Ok(())
    }

    fn update_alias(&mut self, id: &DeviceId, alias: &str) -> Result<bool, CoreError> {
        if let Some(p) = self.peers.get_mut(id) {
            p.alias = alias.to_string();
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

pub trait Insertor: Send + Sync {
    fn insert(&self, value: &str) -> Result<(), CoreError>;
}

/// Launch a local-scheme url on this machine. Callers must pass a url that came
/// from the allowlist — never one carried by a notification (spec §10).
pub trait Opener: Send + Sync {
    fn open(&self, url: &str) -> Result<(), CoreError>;
}

pub trait Clip: Send + Sync {
    fn set_text(&self, text: &str) -> Result<(), CoreError>;
    fn get_text(&self) -> Result<Option<String>, CoreError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotificationUrgency {
    Low,
    #[default]
    Normal,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPayload {
    pub title: String,
    pub body: String,
    pub app_name: Option<String>,
    pub sound: bool,
    pub urgency: NotificationUrgency,
}

impl NotificationPayload {
    pub fn new(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            app_name: None,
            sound: true,
            urgency: NotificationUrgency::Normal,
        }
    }

    pub fn with_app_name(mut self, app_name: impl Into<String>) -> Self {
        self.app_name = Some(app_name.into());
        self
    }

    pub fn with_sound(mut self, sound: bool) -> Self {
        self.sound = sound;
        self
    }

    pub fn with_urgency(mut self, urgency: NotificationUrgency) -> Self {
        self.urgency = urgency;
        self
    }
}

pub trait Notifier: Send + Sync {
    /// Show a notification banner on the local machine.
    fn notify(&self, payload: &NotificationPayload) -> Result<(), CoreError>;

    /// Legacy / lightweight show hook for backward compatibility.
    fn show(&self, title: &str, body_len: usize, score: i32) -> Result<(), CoreError> {
        let _ = (title, body_len, score);
        Ok(())
    }
}

/// Pull one screen frame from the OS. Implemented in tetherly-node only; the
/// core never captures. `grab` is called on the sender's own cadence.
/// Returns Ok(None) when the desktop has not changed since the last call
/// (on-change source like DXGI Desktop Duplication). The caller may sleep
/// and retry. Memory sources always return Some for deterministic CI.
pub trait ScreenSource: Send + Sync {
    fn grab(&self) -> Result<Option<crate::screen::ScreenFrame>, CoreError>;

    /// Best-effort unblock of an in-flight `grab`. The caller invokes this
    /// when a grab overruns its budget (e.g. a device stream wedged without
    /// EOF); the next `grab` must behave as if the source had just been
    /// created. Default: nothing to do (sources that cannot hang).
    fn interrupt(&self) {}

    /// Best-effort on-demand keyframe / IDR request propagation into the source codec.
    fn force_idr(&self) {}

    /// Dynamic bitrate update propagation into the source codec (ADR-082).
    fn set_bitrate_bps(&self, _bps: u32) {}

    /// Target streaming frame rate update propagation (v1.97 / ADR-083).
    fn set_fps(&self, _fps: u32) {}

    /// Current target streaming frame rate.
    fn fps(&self) -> u32 {
        30
    }
}

/// Transform a raw pixel body to or form the wire format carried in
/// `ScreenFrame.bytes`. Lossless zstd is the default production codec;
/// lossy codecs (OpenH264, rav1e) drop in as `ScreenSource`-port
/// decorators later per the "先无损帧 + 后接编码器" decision.
pub trait ScreenCodec: Send + Sync {
    fn compress(&self, raw: &[u8]) -> Result<Vec<u8>, CoreError>;
    fn decompress(&self, compressed: &[u8], expected_len: usize) -> Result<Vec<u8>, CoreError>;

    /// Wire tag this codec stamps into the frame header
    /// (`crate::screen::SCREEN_COMPRESS_*`). Defaults to zstd.
    fn wire_tag(&self) -> u8 {
        1
    }

    /// Frame-geometry-aware encode; codecs that need dimensions (H.264)
    /// override this, byte-only codecs inherit the plain path.
    fn compress_frame(&self, raw: &[u8], w: u16, h: u16) -> Result<Vec<u8>, CoreError> {
        let _ = (w, h);
        self.compress(raw)
    }

    /// Frame-geometry-aware decode; mirrors [`Self::compress_frame`].
    fn decompress_frame(
        &self,
        body: &[u8],
        w: u16,
        h: u16,
        expected_len: usize,
    ) -> Result<Vec<u8>, CoreError> {
        let _ = (w, h);
        self.decompress(body, expected_len)
    }

    /// On-demand keyframe / IDR request (ADR-081): requests that the next
    /// encoded frame be an independent keyframe (IDR for H.264).
    /// Default implementation is a no-op for stateless codecs like zstd.
    fn force_idr(&self) {}

    /// Dynamic target bitrate update (ADR-082): updates the target bitrate of rate-controlled codecs (H.264).
    /// Default implementation is a no-op for stateless/lossless codecs like zstd.
    fn set_bitrate_bps(&self, _bps: u32) {}

    /// Current target bitrate in bps for rate-controlled codecs. Defaults to 0 for unthrottled/lossless.
    fn bitrate_bps(&self) -> u32 {
        0
    }

    /// Target streaming frame rate update propagation for rate-controlled codecs (v1.97 / ADR-083).
    fn set_fps(&self, _fps: u32) {}

    /// Current target streaming frame rate.
    fn fps(&self) -> u32 {
        30
    }
}

/// Present one screen frame. In-memory fake for CI; the OS path is Manual.
pub trait ScreenSink: Send + Sync {
    fn present(&self, frame: &crate::screen::ScreenFrame) -> Result<(), CoreError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionId {
    Copy,
    Dismiss,
    OpenApp,
}

#[derive(Debug, Clone)]
pub struct PhoneNotification {
    pub source: DeviceId,
    pub app_id: String,
    pub app_name: String,
    pub uid: String,
    pub title: String,
    pub body: String,
    pub received_at: SystemTime,
    pub actions: Vec<ActionId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_trust_revokes() {
        let mut s = MemoryTrustStore::default();
        let id = DeviceId::from_id_pk(&[3u8; 32]);
        s.put(TrustedPeer {
            device_id: id.clone(),
            id_pk: [3u8; 32],
            n_pk: [4u8; 32],
            alias: "phone".into(),
            paired_at: 1,
            revoked: false,
        })
        .unwrap();
        s.forget(&id).unwrap();
        assert!(s.get(&id).unwrap().revoked);
    }
}
