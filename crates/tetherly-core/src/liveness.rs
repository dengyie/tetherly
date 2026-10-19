// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Session liveness: when a peer goes quiet, probe it; when it stays silent
//! past the deadline, declare it dead.
//!
//! Pure and clock-injected, so the policy is tested with no timers and nothing
//! here reads the wall clock. A live TCP socket proves nothing on its own: a
//! middlebox can black-hole a flow while both ends still see `ESTABLISHED`, and
//! a blocking `write` into a full kernel buffer parks forever. Only inbound
//! traffic proves the peer is still there.

use std::time::Duration;

/// Quiet interval after which a live session is probed with a ping. Kept well
/// under any plausible NAT idle timeout so the mapping stays warm.
pub const SESSION_HEARTBEAT: Duration = Duration::from_secs(15);

/// A session with no inbound frame for this long is dead. Because a probe is
/// sent every heartbeat, three consecutive probes must be lost before this
/// trips, which rides out a single dropped packet without a false teardown.
pub const SESSION_LIVENESS_DEADLINE: Duration = Duration::from_secs(45);

/// Tracks the last time the peer sent *anything* and answers one question: has
/// it been silent for too long? Any inbound frame — data, ping or pong — proves
/// the peer is alive.
#[derive(Debug, Clone)]
pub struct Liveness {
    heartbeat_ms: u64,
    deadline_ms: u64,
    last_inbound_ms: u64,
}

impl Liveness {
    /// Arm at `now_ms` with the default heartbeat and deadline.
    pub fn new(now_ms: u64) -> Self {
        Self::with(SESSION_HEARTBEAT, SESSION_LIVENESS_DEADLINE, now_ms)
    }

    pub fn with(heartbeat: Duration, deadline: Duration, now_ms: u64) -> Self {
        Self {
            heartbeat_ms: heartbeat.as_millis() as u64,
            deadline_ms: deadline.as_millis() as u64,
            last_inbound_ms: now_ms,
        }
    }

    /// Record that the peer was heard from at `now_ms`.
    pub fn on_inbound(&mut self, now_ms: u64) {
        self.last_inbound_ms = now_ms;
    }

    /// How long the caller should wait between probes.
    pub fn heartbeat(&self) -> Duration {
        Duration::from_millis(self.heartbeat_ms)
    }

    /// Maximum silent duration before the session is declared dead.
    pub fn deadline(&self) -> Duration {
        Duration::from_millis(self.deadline_ms)
    }

    /// Dynamically update the heartbeat interval.
    pub fn set_heartbeat(&mut self, heartbeat: Duration) {
        self.heartbeat_ms = heartbeat.as_millis() as u64;
    }

    /// Dynamically update the liveness deadline.
    pub fn set_deadline(&mut self, deadline: Duration) {
        self.deadline_ms = deadline.as_millis() as u64;
    }

    pub fn last_inbound_ms(&self) -> u64 {
        self.last_inbound_ms
    }

    /// True once the peer has been silent for the whole deadline. The
    /// subtraction saturates so a backwards clock jump reads as "just heard
    /// from" rather than as a huge silence.
    pub fn is_expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_inbound_ms) >= self.deadline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_below_the_deadline_is_not_expired() {
        let l = Liveness::new(1_000);
        assert!(!l.is_expired(1_000));
        assert!(!l.is_expired(1_000 + 44_999));
    }

    #[test]
    fn silence_at_the_deadline_expires() {
        let l = Liveness::new(0);
        assert!(l.is_expired(45_000));
        assert!(l.is_expired(90_000));
    }

    #[test]
    fn inbound_traffic_rearms_the_deadline() {
        let mut l = Liveness::with(Duration::from_secs(15), Duration::from_secs(45), 0);
        l.on_inbound(40_000);
        assert_eq!(l.last_inbound_ms(), 40_000);
        assert_eq!(l.heartbeat(), Duration::from_secs(15));
        assert!(!l.is_expired(40_000));
        assert!(!l.is_expired(84_999));
        assert!(l.is_expired(85_000));
    }

    #[test]
    fn a_backwards_clock_never_reports_expired() {
        let mut l = Liveness::new(1_000_000);
        l.on_inbound(1_000_000);
        // A clock that jumped backwards must not be read as a long silence.
        assert!(!l.is_expired(500_000));
    }

    #[test]
    fn dynamic_heartbeat_and_deadline_tuning() {
        let mut l = Liveness::new(10_000);
        assert_eq!(l.heartbeat(), Duration::from_secs(15));
        assert_eq!(l.deadline(), Duration::from_secs(45));

        l.set_heartbeat(Duration::from_secs(5));
        l.set_deadline(Duration::from_secs(15));
        assert_eq!(l.heartbeat(), Duration::from_secs(5));
        assert_eq!(l.deadline(), Duration::from_secs(15));

        assert!(!l.is_expired(10_000 + 14_999));
        assert!(l.is_expired(10_000 + 15_000));
    }
}
