// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Clipboard hub. `clip_seq` + last-hash prevent echo loops. 4/s per source.
//! Text larger than 1 MiB is refused (spec: go via file).

use crate::dedup::ClipLimiter;
use crate::device_id::DeviceId;
use crate::error::CoreError;
use crate::frame::{hex_sha256, ClipSet, CLIP_TEXT_MAX};

#[derive(Debug, Default)]
pub struct ClipHub {
    last_hash: Option<String>,
    last_seq: u64,
    limiter: ClipLimiter,
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
    fn oversized_refused() {
        let mut hub = ClipHub::default();
        let big = "x".repeat(CLIP_TEXT_MAX + 1);
        assert_eq!(
            hub.prepare_local(&big).unwrap_err(),
            CoreError::ClipboardTooLarge
        );
    }
}
