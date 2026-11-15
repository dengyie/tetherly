// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Pure per-key coalescer. Marks accumulate between flush ticks; each key is
//! emitted at most once per tick. Noisy-only keys are throttled to one emit
//! per [`NOISY_INTERVAL_MS`]; delivery is trailing, so the final state is
//! never dropped, only delayed.

use crate::apiv1::project::Key;
use std::collections::{BTreeMap, HashMap};

/// Minimum spacing between emits of a key whose marks were all noisy.
pub const NOISY_INTERVAL_MS: u64 = 1_000;

#[derive(Debug, Default)]
pub struct Coalescer {
    /// Pending key -> "some non-noisy mark arrived". Ordered for stable output.
    pending: BTreeMap<Key, bool>,
    /// Last emit time per key, kept only while it can still throttle.
    last_sent: HashMap<Key, u64>,
}

impl Coalescer {
    pub fn mark(&mut self, key: Key, noisy: bool) {
        let urgent = self.pending.entry(key).or_insert(false);
        *urgent |= !noisy;
    }

    /// Keys to resolve and publish now, in key order.
    pub fn due(&mut self, now_ms: u64) -> Vec<Key> {
        self.last_sent
            .retain(|_, sent| now_ms.saturating_sub(*sent) < NOISY_INTERVAL_MS);
        let mut out = Vec::new();
        let last_sent = &mut self.last_sent;
        self.pending.retain(|key, urgent| {
            let ready = *urgent || !last_sent.contains_key(key);
            if ready {
                last_sent.insert(key.clone(), now_ms);
                out.push(key.clone());
            }
            !ready
        });
        out
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.last_sent.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str) -> Key {
        Key::Device(id.into())
    }

    #[test]
    fn nothing_pending_means_nothing_due() {
        let mut c = Coalescer::default();
        assert!(c.due(0).is_empty());
    }

    #[test]
    fn repeated_marks_collapse_into_one_emit() {
        let mut c = Coalescer::default();
        c.mark(dev("a"), false);
        c.mark(dev("a"), false);
        c.mark(Key::System, false);
        assert_eq!(c.due(100), vec![dev("a"), Key::System]);
        assert!(c.due(200).is_empty());
    }

    #[test]
    fn first_noisy_mark_is_not_delayed() {
        let mut c = Coalescer::default();
        c.mark(dev("a"), true);
        assert_eq!(c.due(100), vec![dev("a")]);
    }

    #[test]
    fn noisy_marks_are_throttled_but_trailing() {
        let mut c = Coalescer::default();
        c.mark(dev("a"), true);
        assert_eq!(c.due(100), vec![dev("a")]);
        c.mark(dev("a"), true);
        assert!(c.due(300).is_empty());
        assert!(c.due(1000).is_empty());
        assert_eq!(c.due(1100), vec![dev("a")]);
    }

    #[test]
    fn a_real_change_overrides_the_noisy_throttle() {
        let mut c = Coalescer::default();
        c.mark(dev("a"), true);
        assert_eq!(c.due(100), vec![dev("a")]);
        c.mark(dev("a"), true);
        c.mark(dev("a"), false);
        assert_eq!(c.due(200), vec![dev("a")]);
    }

    #[test]
    fn throttle_history_is_pruned() {
        let mut c = Coalescer::default();
        for i in 0..100 {
            c.mark(dev(&format!("d{i}")), false);
        }
        assert_eq!(c.due(0).len(), 100);
        c.due(NOISY_INTERVAL_MS * 2);
        assert_eq!(c.tracked(), 0);
    }
}
