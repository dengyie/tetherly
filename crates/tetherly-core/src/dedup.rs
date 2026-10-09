// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::device_id::DeviceId;
use crate::ports::Clock;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

const WINDOW: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct NotifyKey {
    pub source: DeviceId,
    pub app_id: String,
    pub uid: String,
}

#[derive(Debug, Default)]
pub struct NotifyDeduper {
    seen: HashMap<NotifyKey, u64>,
}

impl NotifyDeduper {
    pub fn is_duplicate(&mut self, key: NotifyKey, clock: &dyn Clock) -> bool {
        let now = clock.unix_ms();
        self.evict(now);
        if let Some(&ts) = self.seen.get(&key) {
            if now.saturating_sub(ts) <= WINDOW.as_millis() as u64 {
                return true;
            }
        }
        self.seen.insert(key, now);
        false
    }

    fn evict(&mut self, now: u64) {
        let window_ms = WINDOW.as_millis() as u64;
        self.seen
            .retain(|_, ts| now.saturating_sub(*ts) <= window_ms);
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[derive(Debug)]
pub struct TokenBucket {
    tokens: f64,
    last_ms: u64,
    rate_per_sec: f64,
    burst: f64,
}

impl TokenBucket {
    pub fn new(rate_per_sec: f64, burst: f64, now_ms: u64) -> Self {
        Self {
            tokens: burst,
            last_ms: now_ms,
            rate_per_sec,
            burst,
        }
    }

    pub fn try_take(&mut self, now_ms: u64) -> bool {
        let elapsed = now_ms.saturating_sub(self.last_ms) as f64 / 1000.0;
        self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.burst);
        self.last_ms = now_ms;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Default)]
pub struct NotifyLimiter {
    per_source: HashMap<DeviceId, TokenBucket>,
}

impl NotifyLimiter {
    pub fn allow(&mut self, source: &DeviceId, now_ms: u64) -> bool {
        let bucket = self
            .per_source
            .entry(source.clone())
            .or_insert_with(|| TokenBucket::new(2.0, 10.0, now_ms));
        bucket.try_take(now_ms)
    }
}

/// Spec §7.3: new TCP connections, 30 per IP per minute.
pub const TCP_PER_IP_PER_MIN: f64 = 30.0 / 60.0;
pub const TCP_PER_IP_BURST: f64 = 30.0;

#[derive(Debug, Default)]
pub struct TcpLimiter {
    per_ip: HashMap<IpAddr, TokenBucket>,
}

impl TcpLimiter {
    pub fn allow(&mut self, ip: IpAddr, now_ms: u64) -> bool {
        let bucket = self
            .per_ip
            .entry(ip)
            .or_insert_with(|| TokenBucket::new(TCP_PER_IP_PER_MIN, TCP_PER_IP_BURST, now_ms));
        bucket.try_take(now_ms)
    }
}

/// Spec §7.3: clip.set 4/s.
#[derive(Debug, Default)]
pub struct ClipLimiter {
    per_source: HashMap<DeviceId, TokenBucket>,
}

impl ClipLimiter {
    pub fn allow(&mut self, source: &DeviceId, now_ms: u64) -> bool {
        let bucket = self
            .per_source
            .entry(source.clone())
            .or_insert_with(|| TokenBucket::new(4.0, 4.0, now_ms));
        bucket.try_take(now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::ManualClock;

    #[test]
    fn duplicate_within_window() {
        let clock = ManualClock::new(1_000_000);
        let mut d = NotifyDeduper::default();
        let key = NotifyKey {
            source: DeviceId::from_id_pk(&[1u8; 32]),
            app_id: "sms".into(),
            uid: "42".into(),
        };
        assert!(!d.is_duplicate(key.clone(), &clock));
        assert!(d.is_duplicate(key.clone(), &clock));
        clock.set(1_000_000 + 10 * 60 * 1000 + 1);
        assert!(!d.is_duplicate(key, &clock));
    }

    #[test]
    fn burst_then_rate() {
        let mut b = TokenBucket::new(2.0, 10.0, 0);
        for _ in 0..10 {
            assert!(b.try_take(0));
        }
        assert!(!b.try_take(0));
        assert!(b.try_take(500));
    }

    #[test]
    fn tcp_limiter_thirty_per_minute() {
        let ip: IpAddr = "127.0.0.1".parse().unwrap();
        let mut lim = TcpLimiter::default();
        for _ in 0..30 {
            assert!(lim.allow(ip, 0));
        }
        assert!(!lim.allow(ip, 0));
        assert!(lim.allow(ip, 2_000));
    }
}
