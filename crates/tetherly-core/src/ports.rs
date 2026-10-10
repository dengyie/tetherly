// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::device_id::DeviceId;
use crate::error::CoreError;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait Clock: Send + Sync {
    fn unix_ms(&self) -> u64;
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

pub trait Notifier: Send + Sync {
    fn show(&self, title: &str, body_len: usize, score: i32) -> Result<(), CoreError>;
}

/// Pull one screen frame from the OS. Implemented in tetherly-node only; the
/// core never captures. `grab` is called on the sender's own cadence.
pub trait ScreenSource: Send + Sync {
    fn grab(&self) -> Result<crate::screen::ScreenFrame, CoreError>;
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
