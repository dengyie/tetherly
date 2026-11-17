// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::device_id::DeviceId;
use crate::error::CoreError;
use crate::hello::Hello;
use crate::ports::{TrustStore, TrustedPeer};
use std::net::IpAddr;
use std::time::Duration;

pub const PIN_TTL: Duration = Duration::from_secs(3 * 60);
pub const PIN_MAX_FAILS: u32 = 5;
pub const LOCKOUT: Duration = Duration::from_secs(15 * 60);
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Idle,
    Discovering,
    TcpConnected,
    Hello,
    Pairing,
    PairBind,
    NoiseIk,
    Active,
    Backoff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloDisposition {
    Pair,
    ResumeNoise,
}

#[derive(Debug, Clone, Default)]
pub struct Lockout {
    fails: u32,
    locked_until_ms: u64,
}

impl Lockout {
    pub fn is_locked(&self, now_ms: u64) -> bool {
        now_ms < self.locked_until_ms
    }

    pub fn record_failure(&mut self, now_ms: u64) {
        self.fails = self.fails.saturating_add(1);
        if self.fails >= PIN_MAX_FAILS {
            self.locked_until_ms = now_ms.saturating_add(LOCKOUT.as_millis() as u64);
        }
    }

    /// An expired lock is spent: the entry becomes eligible for eviction and
    /// a fresh attacker gets a fresh fail budget. (Keeps the table bounded
    /// without a time-stamped eviction queue.)
    pub fn expired(&self, now_ms: u64) -> bool {
        !self.is_locked(now_ms)
    }

    pub fn reset(&mut self) {
        self.fails = 0;
        self.locked_until_ms = 0;
    }

    pub fn fails(&self) -> u32 {
        self.fails
    }
}

#[derive(Debug, Default)]
pub struct LockoutTable {
    by_peer: std::collections::HashMap<DeviceId, Lockout>,
    by_ip: std::collections::HashMap<IpAddr, Lockout>,
}

/// Entries retained after a sweep. Lockout entries only matter while a lock
/// is live or a fail counter is accumulating toward one; anything past both
/// is dead weight, and pre-auth keys are attacker-controlled, so an
/// unbounded table would let a peer (or LAN scanner) grow it forever.
const LOCKOUT_SWEEP_FLOOR: usize = 64;

impl LockoutTable {
    /// Drop entries whose lock expired with no new fails since. Called from
    /// `fail` (the only writer) so attacker-forged keys cannot grow the
    /// tables without bound; keeps a small floor so a live lock is never
    /// evicted by a concurrent sweep of stale entries.
    fn sweep(&mut self, now_ms: u64) {
        if self.by_peer.len() > LOCKOUT_SWEEP_FLOOR {
            self.by_peer
                .retain(|_, l| l.fails() > 0 && !l.expired(now_ms));
        }
        if self.by_ip.len() > LOCKOUT_SWEEP_FLOOR {
            self.by_ip
                .retain(|_, l| l.fails() > 0 && !l.expired(now_ms));
        }
    }

    pub fn check(&self, peer: Option<&DeviceId>, ip: Option<IpAddr>, now_ms: u64) -> bool {
        if let Some(id) = peer {
            if self.by_peer.get(id).is_some_and(|l| l.is_locked(now_ms)) {
                return true;
            }
        }
        if let Some(ip) = ip {
            if self.by_ip.get(&ip).is_some_and(|l| l.is_locked(now_ms)) {
                return true;
            }
        }
        false
    }

    pub fn fail(&mut self, peer: Option<&DeviceId>, ip: Option<IpAddr>, now_ms: u64) {
        if let Some(id) = peer {
            self.by_peer
                .entry(id.clone())
                .or_default()
                .record_failure(now_ms);
        }
        if let Some(ip) = ip {
            self.by_ip.entry(ip).or_default().record_failure(now_ms);
        }
        self.sweep(now_ms);
    }

    pub fn success(&mut self, peer: Option<&DeviceId>, ip: Option<IpAddr>) {
        if let Some(id) = peer {
            if let Some(l) = self.by_peer.get_mut(id) {
                l.reset();
            }
        }
        if let Some(ip) = ip {
            if let Some(l) = self.by_ip.get_mut(&ip) {
                l.reset();
            }
        }
    }

    pub fn fails_for(&self, peer: &DeviceId) -> u32 {
        self.by_peer.get(peer).map(|l| l.fails()).unwrap_or(0)
    }
}

pub fn backoff_delay(attempt: u32) -> Duration {
    let secs = 1u64.checked_shl(attempt.min(6)).unwrap_or(u64::MAX);
    Duration::from_secs(secs.min(BACKOFF_MAX.as_secs()))
}

pub fn dispose_hello(hello: &Hello, store: &dyn TrustStore) -> Result<HelloDisposition, CoreError> {
    let keys = hello.validate()?;
    match store.get(&hello.device_id) {
        None => Ok(HelloDisposition::Pair),
        Some(peer) if peer.revoked => Err(CoreError::Revoked),
        Some(peer) if peer.n_pk != keys.n_pk || peer.id_pk != keys.id_pk => {
            Err(CoreError::StaticKeyChanged)
        }
        Some(_) => Ok(HelloDisposition::ResumeNoise),
    }
}

pub fn commit_trust(
    store: &mut dyn TrustStore,
    hello: &Hello,
    now_ms: u64,
    alias: String,
) -> Result<TrustedPeer, CoreError> {
    let keys = hello.validate()?;
    let peer = TrustedPeer {
        device_id: hello.device_id.clone(),
        id_pk: keys.id_pk,
        n_pk: keys.n_pk,
        alias,
        paired_at: now_ms,
        revoked: false,
    };
    store.put(peer.clone())?;
    Ok(peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hello::encode_b64_32;
    use crate::ports::MemoryTrustStore;

    fn hello_for(id_pk: [u8; 32], n_pk: [u8; 32]) -> Hello {
        Hello {
            proto: 1,
            device_id: DeviceId::from_id_pk(&id_pk),
            id_pk: encode_b64_32(&id_pk),
            n_pk: encode_b64_32(&n_pk),
            caps: vec!["notify".into()],
            name: "A".into(),
            platform: "linux".into(),
        }
    }

    #[test]
    fn unknown_goes_to_pair() {
        let store = MemoryTrustStore::default();
        let h = hello_for([1u8; 32], [2u8; 32]);
        assert_eq!(dispose_hello(&h, &store).unwrap(), HelloDisposition::Pair);
    }

    #[test]
    fn trusted_matching_n_pk_resumes() {
        let mut store = MemoryTrustStore::default();
        let h = hello_for([1u8; 32], [2u8; 32]);
        commit_trust(&mut store, &h, 1, "x".into()).unwrap();
        assert_eq!(
            dispose_hello(&h, &store).unwrap(),
            HelloDisposition::ResumeNoise
        );
    }

    #[test]
    fn n_pk_change_disconnects() {
        let mut store = MemoryTrustStore::default();
        let h = hello_for([1u8; 32], [2u8; 32]);
        commit_trust(&mut store, &h, 1, "x".into()).unwrap();
        let h2 = hello_for([1u8; 32], [9u8; 32]);
        assert_eq!(
            dispose_hello(&h2, &store).unwrap_err(),
            CoreError::StaticKeyChanged
        );
    }

    #[test]
    fn five_fails_lock() {
        let mut t = LockoutTable::default();
        let id = DeviceId::from_id_pk(&[8u8; 32]);
        for i in 0..5 {
            t.fail(Some(&id), None, 1000 + i as u64);
        }
        assert!(t.check(Some(&id), None, 1000));
        let last_fail_ms = 1000 + 4;
        assert!(!t.check(Some(&id), None, last_fail_ms + LOCKOUT.as_millis() as u64));
    }

    /// §158: a flood of attacker-forged pre-auth device ids cannot grow the
    /// table without bound — expired entries are swept once past the floor,
    /// while a live lock is never evicted.
    #[test]
    fn lockout_table_sweeps_expired_entries_and_keeps_live_locks() {
        let mut t = LockoutTable::default();
        let live = DeviceId::from_id_pk(&[9u8; 32]);
        // The live lock first so the sweep floor covers it.
        for i in 0..PIN_MAX_FAILS {
            t.fail(Some(&live), None, 1000 + i as u64);
        }
        // A flood of one-shot forged ids: each fails once, never locks, never
        // returns — exactly the unbounded-growth vector.
        let flood = LOCKOUT_SWEEP_FLOOR + 40;
        for n in 0..flood {
            let mut pk = [0u8; 32];
            pk[0] = 0xA0 + (n % 0x10) as u8;
            pk[1] = (n / 0x10) as u8 + 1;
            let id = DeviceId::from_id_pk(&pk);
            t.fail(Some(&id), None, 1000);
        }
        // Every flood entry is a single fail (no lock): they survive a sweep
        // only while unexpired-and-counting, so the table stays at floor+live.
        assert!(
            t.by_peer.len() <= LOCKOUT_SWEEP_FLOOR + 1,
            "sweep caps the table (got {})",
            t.by_peer.len()
        );
        // The genuine locked peer is untouched.
        assert!(
            t.check(Some(&live), None, 1000),
            "live lock survives the sweep"
        );
    }

    /// An expired lock is spent: after LOCKOUT the entry is swept away and a
    /// fresh attacker gets a fresh fail budget (no sticky re-lock).
    #[test]
    fn expired_lock_entry_is_swept_so_budgets_reset() {
        let mut t = LockoutTable::default();
        let id = DeviceId::from_id_pk(&[7u8; 32]);
        for i in 0..PIN_MAX_FAILS {
            t.fail(Some(&id), None, 1000 + i as u64);
        }
        assert!(
            t.check(Some(&id), None, 1000),
            "locked while the lock lives"
        );
        // Past expiry + past the floor, one more fail (a new attempt) sweeps
        // the stale entry: the map must not retain the expired record. The
        // lock runs from the LAST fail (1004), so expiry is 1004+LOCKOUT.
        let after = 1004 + LOCKOUT.as_millis() as u64 + 1;
        // Fill past the floor so the sweep triggers.
        for n in 0..=LOCKOUT_SWEEP_FLOOR {
            let mut pk = [0u8; 32];
            pk[0] = (n as u8) + 1;
            pk[1] = 0xEE;
            t.fail(Some(&DeviceId::from_id_pk(&pk)), None, after);
        }
        assert!(
            !t.by_peer.contains_key(&DeviceId::from_id_pk(&[7u8; 32])),
            "expired entry swept"
        );
        assert!(!t.check(Some(&id), None, after), "budget is fresh again");
    }

    #[test]
    fn backoff_caps_at_sixty() {
        assert_eq!(backoff_delay(0), Duration::from_secs(1));
        assert_eq!(backoff_delay(1), Duration::from_secs(2));
        assert_eq!(backoff_delay(10), Duration::from_secs(60));
    }
}
