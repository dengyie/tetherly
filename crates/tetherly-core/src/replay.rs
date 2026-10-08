// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::VecDeque;

const CAP: usize = 1024;

#[derive(Debug, Default)]
pub struct ReplayGuard {
    next: u64,
    recent: VecDeque<u64>,
}

impl ReplayGuard {
    pub fn check_and_record(&mut self, msg_id: u64) -> bool {
        if self.recent.contains(&msg_id) {
            return false;
        }
        if msg_id + 1024 < self.next && self.next > 1024 {
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
}
