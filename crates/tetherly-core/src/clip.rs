// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Clipboard hub. `clip_seq` + last-hash prevent echo loops. 4/s per source.
//! Text larger than 1 MiB is refused (spec: go via file).

use std::collections::VecDeque;

use crate::dedup::ClipLimiter;
use crate::device_id::DeviceId;
use crate::error::CoreError;
use crate::frame::{hex_sha256, ClipSet, CLIP_TEXT_MAX};

pub const MAX_CLIP_HISTORY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ClipMetaEntry {
    pub seq: u64,
    pub source: Option<DeviceId>,
    pub direction: ClipDirection,
    pub bytes: usize,
    pub hash: String,
    pub timestamp_ms: u64,
}

#[derive(Debug, Default)]
pub struct ClipHub {
    last_hash: Option<String>,
    last_seq: u64,
    limiter: ClipLimiter,
    history: VecDeque<ClipMetaEntry>,
    inbound_count: u64,
    outbound_count: u64,
}

pub enum ClipApply {
    Apply(String),
    Echo,
    RateLimited,
}

impl ClipHub {
    pub fn last_hash(&self) -> Option<&str> {
        self.last_hash.as_deref()
    }

    pub fn last_seq(&self) -> u64 {
        self.last_seq
    }

    pub fn inbound_count(&self) -> u64 {
        self.inbound_count
    }

    pub fn outbound_count(&self) -> u64 {
        self.outbound_count
    }

    pub fn history_count(&self) -> usize {
        self.history.len()
    }

    pub fn stats(&self) -> (u64, u64) {
        (self.inbound_count, self.outbound_count)
    }

    pub fn record_entry(&mut self, entry: ClipMetaEntry) {
        match entry.direction {
            ClipDirection::Inbound => self.inbound_count = self.inbound_count.saturating_add(1),
            ClipDirection::Outbound => self.outbound_count = self.outbound_count.saturating_add(1),
        }
        if self.history.len() >= MAX_CLIP_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(entry);
    }

    pub fn history(&self, limit: Option<usize>) -> Vec<ClipMetaEntry> {
        let max = limit.unwrap_or(MAX_CLIP_HISTORY).min(MAX_CLIP_HISTORY);
        self.history.iter().rev().take(max).cloned().collect()
    }

    pub fn clear_history(&mut self) -> usize {
        let count = self.history.len();
        self.history.clear();
        count
    }

    pub fn reset_state(&mut self) -> usize {
        self.last_hash = None;
        self.clear_history()
    }

    /// Local user copied text. None if it is the same bytes we just wrote.
    pub fn prepare_local(&mut self, text: &str) -> Result<Option<ClipSet>, CoreError> {
        if text.len() > CLIP_TEXT_MAX {
            return Err(CoreError::ClipboardTooLarge);
        }
        let hash = hex_sha256(text.as_bytes());
        if self.last_hash.as_deref() == Some(hash.as_str()) {
            return Ok(None);
        }
        self.last_seq = self.last_seq.saturating_add(1);
        self.last_hash = Some(hash);
        Ok(Some(ClipSet::text(self.last_seq, text)?))
    }

    /// User or loopback client explicitly requested to broadcast `text`.
    /// Generates a new `ClipSet` and advances `last_seq` even if the hash matches.
    pub fn force_prepare_local(&mut self, text: &str) -> Result<ClipSet, CoreError> {
        if text.len() > CLIP_TEXT_MAX {
            return Err(CoreError::ClipboardTooLarge);
        }
        let hash = hex_sha256(text.as_bytes());
        self.last_seq = self.last_seq.saturating_add(1);
        self.last_hash = Some(hash);
        ClipSet::text(self.last_seq, text)
    }

    /// Record that we wrote `text` locally (OTP copy, insert side-effect).
    pub fn note_local_write(&mut self, text: &str) {
        self.last_hash = Some(hex_sha256(text.as_bytes()));
    }

    pub fn apply_remote(
        &mut self,
        source: &DeviceId,
        clip: ClipSet,
        now_ms: u64,
    ) -> Result<ClipApply, CoreError> {
        if !self.limiter.allow(source, now_ms) {
            return Ok(ClipApply::RateLimited);
        }
        if self.last_hash.as_deref() == Some(clip.hash.as_str()) {
            return Ok(ClipApply::Echo);
        }
        let Some(text) = clip.text else {
            return Err(CoreError::ClipboardTooLarge);
        };
        if text.len() > CLIP_TEXT_MAX {
            return Err(CoreError::ClipboardTooLarge);
        }
        if hex_sha256(text.as_bytes()) != clip.hash {
            return Err(CoreError::Sha256Mismatch);
        }
        self.last_hash = Some(clip.hash);
        if clip.clip_seq > self.last_seq {
            self.last_seq = clip.clip_seq;
        }
        Ok(ClipApply::Apply(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_loop_suppressed() {
        let mut hub = ClipHub::default();
        let set = hub.prepare_local("hello").unwrap().unwrap();
        assert!(hub.prepare_local("hello").unwrap().is_none());
        let source = DeviceId::from_id_pk(&[2u8; 32]);
        assert!(matches!(
            hub.apply_remote(&source, set, 1).unwrap(),
            ClipApply::Echo
        ));
    }

    #[test]
    fn remote_then_local_echo() {
        let mut hub = ClipHub::default();
        let source = DeviceId::from_id_pk(&[3u8; 32]);
        let set = ClipSet::text(1, "from-peer").unwrap();
        match hub.apply_remote(&source, set, 10).unwrap() {
            ClipApply::Apply(t) => assert_eq!(t, "from-peer"),
            _ => panic!("expected apply"),
        }
        assert!(hub.prepare_local("from-peer").unwrap().is_none());
    }

    #[test]
    fn force_prepare_local_advances_and_broadcasts() {
        let mut hub = ClipHub::default();
        let set1 = hub.prepare_local("repeat-content").unwrap().unwrap();
        assert_eq!(set1.clip_seq, 1);
        assert!(hub.prepare_local("repeat-content").unwrap().is_none());

        let set2 = hub.force_prepare_local("repeat-content").unwrap();
        assert_eq!(set2.clip_seq, 2);
        assert_eq!(set2.text.as_deref(), Some("repeat-content"));
    }

    #[test]
    fn oversized_refused() {
        let mut hub = ClipHub::default();
        let big = "x".repeat(CLIP_TEXT_MAX + 1);
        assert_eq!(
            hub.prepare_local(&big).unwrap_err(),
            CoreError::ClipboardTooLarge
        );
    }

    #[test]
    fn history_and_bounded_ring_buffer() {
        let mut hub = ClipHub::default();
        assert_eq!(hub.history_count(), 0);
        assert_eq!(hub.stats(), (0, 0));

        let source = DeviceId::from_id_pk(&[7u8; 32]);
        // Insert 40 entries, exceeding MAX_CLIP_HISTORY (32)
        for i in 1..=40 {
            let dir = if i % 2 == 0 {
                ClipDirection::Inbound
            } else {
                ClipDirection::Outbound
            };
            let src = if i % 2 == 0 {
                Some(source.clone())
            } else {
                None
            };
            hub.record_entry(ClipMetaEntry {
                seq: i as u64,
                source: src,
                direction: dir,
                bytes: i * 10,
                hash: format!("hash-{}", i),
                timestamp_ms: 1000 + i as u64,
            });
        }

        assert_eq!(hub.history_count(), 32);
        assert_eq!(hub.stats(), (20, 20));
        assert_eq!(hub.inbound_count(), 20);
        assert_eq!(hub.outbound_count(), 20);

        // history() returns newest first
        let hist = hub.history(Some(5));
        assert_eq!(hist.len(), 5);
        assert_eq!(hist[0].seq, 40);
        assert_eq!(hist[0].hash, "hash-40");
        assert_eq!(hist[4].seq, 36);

        // history with no limit returns all 32
        let all = hub.history(None);
        assert_eq!(all.len(), 32);
        assert_eq!(all[0].seq, 40);
        assert_eq!(all[31].seq, 9); // 40 - 32 + 1 = 9

        // Clear history
        let cleared = hub.clear_history();
        assert_eq!(cleared, 32);
        assert_eq!(hub.history_count(), 0);
        assert_eq!(hub.stats(), (20, 20)); // stats counts are cumulative

        // Reset state clears last_hash and history
        hub.prepare_local("hello").unwrap();
        assert!(hub.last_hash().is_some());
        hub.record_entry(ClipMetaEntry {
            seq: 41,
            source: None,
            direction: ClipDirection::Outbound,
            bytes: 5,
            hash: "abc".into(),
            timestamp_ms: 2000,
        });
        assert_eq!(hub.history_count(), 1);
        hub.reset_state();
        assert!(hub.last_hash().is_none());
        assert_eq!(hub.history_count(), 0);
    }
}
