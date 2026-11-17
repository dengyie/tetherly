// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Sequence numbering and the bounded replay ring behind `Last-Event-ID`.

use std::collections::VecDeque;
use std::sync::Arc;
use tetherly_api::{Event, EventEnvelope, Topic};

/// One published event, serialized once and shared by every subscriber.
#[derive(Debug)]
pub struct Frame {
    pub seq: u64,
    /// `None` only for snapshots, which bypass topic filters.
    pub topic: Option<Topic>,
    pub name: &'static str,
    /// The [`EventEnvelope`] JSON.
    pub data: String,
}

impl Frame {
    pub fn new(seq: u64, event: &Event) -> Self {
        let envelope = EventEnvelope {
            seq,
            event: event.clone(),
        };
        // Fail-soft on purpose: every `Event` variant is plain data, so an
        // encode failure is effectively unreachable — and tearing down the
        // shared ring (or emitting a malformed SSE frame) over one would
        // hurt every subscriber. `{}` keeps the frame parseable; the id
        // still advances so replay offsets stay consistent.
        Self {
            seq,
            topic: event.topic(),
            name: event.name(),
            data: serde_json::to_string(&envelope).unwrap_or_else(|_| "{}".to_string()),
        }
    }

    pub fn sse(&self) -> String {
        format!(
            "id: {}\nevent: {}\ndata: {}\n\n",
            self.seq, self.name, self.data
        )
    }
}

#[derive(Debug)]
pub struct Journal {
    /// Seq of the last published frame (`base` before the first).
    seq: u64,
    ring: VecDeque<Arc<Frame>>,
    cap: usize,
}

impl Journal {
    /// `base` should exceed any seq an earlier daemon run could have issued.
    pub fn new(base: u64, cap: usize) -> Self {
        Self {
            seq: base,
            ring: VecDeque::with_capacity(cap),
            cap: cap.max(1),
        }
    }

    pub fn current(&self) -> u64 {
        self.seq
    }

    pub fn push(&mut self, event: &Event) -> Arc<Frame> {
        self.seq += 1;
        let frame = Arc::new(Frame::new(self.seq, event));
        if self.ring.len() == self.cap {
            self.ring.pop_front();
        }
        self.ring.push_back(frame.clone());
        frame
    }

    /// Frames after `last`, or `None` when the ring cannot prove it holds all
    /// of them (evicted, earlier run, or an id from the future).
    pub fn since(&self, last: u64) -> Option<Vec<Arc<Frame>>> {
        if last > self.seq {
            return None;
        }
        let oldest = self.ring.front().map_or(self.seq + 1, |f| f.seq);
        if last + 1 < oldest {
            return None;
        }
        Some(self.ring.iter().filter(|f| f.seq > last).cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inv(topic: Topic) -> Event {
        Event::Invalidated { topic }
    }

    fn seqs(frames: &[std::sync::Arc<Frame>]) -> Vec<u64> {
        frames.iter().map(|f| f.seq).collect()
    }

    #[test]
    fn sequence_is_contiguous_above_the_base() {
        let mut j = Journal::new(1_000, 8);
        assert_eq!(j.current(), 1_000);
        assert_eq!(j.push(&inv(Topic::Inbox)).seq, 1_001);
        assert_eq!(j.push(&inv(Topic::Inbox)).seq, 1_002);
        assert_eq!(j.current(), 1_002);
    }

    #[test]
    fn frame_renders_sse_with_the_envelope_as_data() {
        let mut j = Journal::new(41, 8);
        let f = j.push(&inv(Topic::Layout));
        assert_eq!(f.topic, Some(Topic::Layout));
        let text = f.sse();
        assert!(
            text.starts_with("id: 42\nevent: invalidated\ndata: "),
            "{text}"
        );
        assert!(text.ends_with("\n\n"));
        let data = text.lines().nth(2).unwrap().trim_start_matches("data: ");
        let env: EventEnvelope = serde_json::from_str(data).unwrap();
        assert_eq!(env.seq, 42);
        assert_eq!(env.event, inv(Topic::Layout));
    }

    #[test]
    fn replays_everything_after_a_covered_id() {
        let mut j = Journal::new(100, 8);
        for _ in 0..3 {
            j.push(&inv(Topic::Inbox));
        }
        assert_eq!(seqs(&j.since(101).unwrap()), vec![102, 103]);
        assert_eq!(seqs(&j.since(100).unwrap()), vec![101, 102, 103]);
        assert!(j.since(103).unwrap().is_empty());
    }

    #[test]
    fn uncovered_ids_need_a_snapshot() {
        let mut j = Journal::new(100, 2);
        for _ in 0..3 {
            j.push(&inv(Topic::Inbox));
        }
        // ring holds 102,103: 101 was evicted, so "after 100" is incomplete.
        assert!(j.since(100).is_none());
        assert_eq!(seqs(&j.since(101).unwrap()), vec![102, 103]);
        // earlier daemon run / future id
        assert!(j.since(5).is_none());
        assert!(j.since(104).is_none());
    }

    #[test]
    fn fresh_journal_replays_nothing_for_its_own_base() {
        let j = Journal::new(100, 2);
        assert!(j.since(100).unwrap().is_empty());
        assert!(j.since(99).is_none());
    }
}
