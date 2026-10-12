// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::VecDeque;

const CAP: usize = 1024;

#[derive(Debug, Default)]
pub struct ReplayGuard {
    next: u64,
    recent: VecDeque<u64>,
}

impl ReplayGuard {
    /// Accept and record a fresh `msg_id`; reject a replay or an id more than
    /// `CAP` behind the high-water mark. `msg_id` comes straight off the wire,
    /// so every step here must be wrap-safe: `msg_id + 1024` would otherwise
    /// overflow on an attacker-chosen `u64` (panic in debug, silently accept an
    /// ancient id in release).
    pub fn check_and_record(&mut self, msg_id: u64) -> bool {
        if self.recent.contains(&msg_id) {
            return false;
        }
        if self.next > 1024 && msg_id.saturating_add(1024) < self.next {
            return false;
        }
        self.recent.push_back(msg_id);
        if self.recent.len() > CAP {
            self.recent.pop_front();
        }
        if msg_id >= self.next {
            self.next = msg_id.saturating_add(1);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_replay() {
        let mut g = ReplayGuard::default();
        assert!(g.check_and_record(1));
        assert!(!g.check_and_record(1));
        assert!(g.check_and_record(2));
    }

    #[test]
    fn rejects_id_far_behind_the_high_water_mark() {
        let mut g = ReplayGuard::default();
        assert!(g.check_and_record(4_000));
        // Much older than the 1024 window and never seen: stale, reject.
        assert!(!g.check_and_record(1_000));
        // Within the window: still fresh.
        assert!(g.check_and_record(3_500));
    }

    #[test]
    fn extreme_peer_ids_do_not_overflow() {
        let mut g = ReplayGuard::default();
        // u64::MAX would make `msg_id + 1024` wrap; must not panic and must
        // still be treated as the high-water mark.
        assert!(g.check_and_record(u64::MAX));
        assert!(!g.check_and_record(u64::MAX)); // replay of the same id
        assert!(!g.check_and_record(0)); // far behind now
        let mut g2 = ReplayGuard::default();
        // A near-`MAX` id is fine and becomes the new mark.
        assert!(g2.check_and_record(u64::MAX - 5));
        assert!(g2.check_and_record(u64::MAX));
    }
}
