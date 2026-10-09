// SPDX-License-Identifier: Apache-2.0 OR MIT
//! NotifyHub: truncate, dedup, rate-limit, local OTP extract, 120s candidates.
//! Never logs title/body/OTP. Callers must not put those fields in tracing.

use crate::dedup::{NotifyDeduper, NotifyKey, NotifyLimiter};
use crate::device_id::DeviceId;
use crate::error::CoreError;
use crate::frame::{hex_sha256, NotifyDismiss, NotifyPush, CANDIDATE_TTL_MS};
use crate::otp::{DefaultOtpExtractor, OtpExtractor};
use crate::ports::{ActionId, Clock, PhoneNotification};
use std::collections::HashMap;
use std::time::UNIX_EPOCH;

/// In-memory OTP candidate. Debug redacts the code and body.
pub struct Candidate {
    pub id: String,
    pub source: DeviceId,
    pub app_id: String,
    pub app_name: String,
    pub uid: String,
    pub title: String,
    pub created_ms: u64,
    pub expires_ms: u64,
    otp: Option<String>,
}

impl Candidate {
    pub fn has_otp(&self) -> bool {
        self.otp.is_some()
    }

    pub fn otp(&self) -> Option<&str> {
        self.otp.as_deref()
    }

    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_ms
    }
}

impl std::fmt::Debug for Candidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Candidate")
            .field("id", &self.id)
            .field("source", &self.source)
            .field("app_id", &self.app_id)
            .field("uid", &self.uid)
            .field("has_otp", &self.has_otp())
            .field("expires_ms", &self.expires_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateView {
    pub id: String,
    pub source: DeviceId,
    pub app_id: String,
    pub app_name: String,
    pub uid: String,
    pub title: String,
    pub has_otp: bool,
    pub created_ms: u64,
    pub expires_ms: u64,
}

impl From<&Candidate> for CandidateView {
    fn from(c: &Candidate) -> Self {
        Self {
            id: c.id.clone(),
            source: c.source.clone(),
            app_id: c.app_id.clone(),
            app_name: c.app_name.clone(),
            uid: c.uid.clone(),
            title: c.title.clone(),
            has_otp: c.has_otp(),
            created_ms: c.created_ms,
            expires_ms: c.expires_ms,
        }
    }
}

#[derive(Debug, Default)]
pub struct NotifyHub {
    extractor: DefaultOtpExtractor,
    deduper: NotifyDeduper,
    limiter: NotifyLimiter,
    candidates: HashMap<String, Candidate>,
}

#[derive(Debug)]
pub enum IngestOutcome {
    Candidate(CandidateView),
    Duplicate,
    RateLimited,
}

impl NotifyHub {
    pub fn ingest_push(
        &mut self,
        source: DeviceId,
        push: NotifyPush,
        clock: &dyn Clock,
    ) -> IngestOutcome {
        let now = clock.unix_ms();
        let push = push.truncate();
        if !self.limiter.allow(&source, now) {
            tracing::debug!(source = %source, "notify rate-limited");
            return IngestOutcome::RateLimited;
        }
        let key = NotifyKey {
            source: source.clone(),
            app_id: push.app_id.clone(),
            uid: push.uid.clone(),
        };
        if self.deduper.is_duplicate(key, clock) {
            tracing::debug!(source = %source, uid = %push.uid, "notify duplicate");
            return IngestOutcome::Duplicate;
        }

        let combined = format!("{}\n{}", push.title, push.body);
        let otp = self.extractor.extract(&combined);
        let id = candidate_id(&source, &push.app_id, &push.uid, push.ts);
        let candidate = Candidate {
            id: id.clone(),
            source: source.clone(),
            app_id: push.app_id.clone(),
            app_name: push.app_name.clone(),
            uid: push.uid.clone(),
            title: push.title.clone(),
            created_ms: now,
            expires_ms: now.saturating_add(CANDIDATE_TTL_MS),
            otp,
        };
        let view = CandidateView::from(&candidate);
        tracing::info!(
            source = %source,
            app_id = %push.app_id,
            uid = %push.uid,
            has_otp = view.has_otp,
            "notify ingested"
        );
        self.candidates.insert(id, candidate);
        IngestOutcome::Candidate(view)
    }

    pub fn ingest_phone(&mut self, n: PhoneNotification, clock: &dyn Clock) -> IngestOutcome {
        let ts = n
            .received_at
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let actions = n
            .actions
            .iter()
            .map(|a| match a {
                ActionId::Copy => "copy".to_string(),
                ActionId::Dismiss => "dismiss".to_string(),
                ActionId::OpenApp => "open".to_string(),
            })
            .collect();
        self.ingest_push(
            n.source,
            NotifyPush {
                uid: n.uid,
                app_id: n.app_id,
                app_name: n.app_name,
                title: n.title,
                body: n.body,
                ts,
                actions,
            },
            clock,
        )
    }

    pub fn copy_otp(&mut self, id: &str, clock: &dyn Clock) -> Result<String, CoreError> {
        let expired = self.candidates.get(id).map(|c| c.expired(clock.unix_ms()));
        match expired {
            None => {
                self.evict_expired(clock);
                Err(CoreError::UnknownCandidate)
            }
            Some(true) => {
                self.candidates.remove(id);
                Err(CoreError::CandidateExpired)
            }
            Some(false) => self
                .candidates
                .get(id)
                .and_then(|c| c.otp.clone())
                .ok_or(CoreError::NoOtp),
        }
    }

    pub fn dismiss(&mut self, id: &str) -> bool {
        self.candidates.remove(id).is_some()
    }

    pub fn dismiss_wire(&mut self, source: &DeviceId, d: &NotifyDismiss) -> bool {
        let ids: Vec<String> = self
            .candidates
            .iter()
            .filter(|(_, c)| c.source == *source && c.uid == d.uid && c.app_id == d.app_id)
            .map(|(id, _)| id.clone())
            .collect();
        let mut any = false;
        for id in ids {
            any |= self.candidates.remove(&id).is_some();
        }
        any
    }

    pub fn evict_expired(&mut self, clock: &dyn Clock) -> Vec<String> {
        let now = clock.unix_ms();
        let expired: Vec<String> = self
            .candidates
            .iter()
            .filter(|(_, c)| c.expired(now))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            self.candidates.remove(id);
        }
        expired
    }

    pub fn views(&self, clock: &dyn Clock) -> Vec<CandidateView> {
        let now = clock.unix_ms();
        self.candidates
            .values()
            .filter(|c| !c.expired(now))
            .map(CandidateView::from)
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&Candidate> {
        self.candidates.get(id)
    }
}

pub fn candidate_id(source: &DeviceId, app_id: &str, uid: &str, ts: u64) -> String {
    let mut buf = Vec::new();
    buf.extend_from_slice(source.as_str().as_bytes());
    buf.push(0);
    buf.extend_from_slice(app_id.as_bytes());
    buf.push(0);
    buf.extend_from_slice(uid.as_bytes());
    buf.push(0);
    buf.extend_from_slice(&ts.to_be_bytes());
    hex_sha256(&buf)
}

pub fn is_desktop_platform(platform: &str) -> bool {
    matches!(platform, "windows" | "macos" | "linux" | "darwin" | "win32")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::ManualClock;

    fn push(code_in_body: bool) -> NotifyPush {
        NotifyPush {
            uid: "42".into(),
            app_id: "com.example.sms".into(),
            app_name: "信息".into(),
            title: "网易".into(),
            body: if code_in_body {
                "【网易】验证码：868740，您正在登录".into()
            } else {
                "hello".into()
            },
            ts: 1,
            actions: vec!["copy".into()],
        }
    }

    #[test]
    fn extract_locally_and_expire() {
        let mut hub = NotifyHub::default();
        let clock = ManualClock::new(1_000);
        let source = DeviceId::from_id_pk(&[9u8; 32]);
        match hub.ingest_push(source.clone(), push(true), &clock) {
            IngestOutcome::Candidate(v) => {
                assert!(v.has_otp);
                assert_eq!(hub.copy_otp(&v.id, &clock).unwrap(), "868740");
                clock.set(1_000 + CANDIDATE_TTL_MS + 1);
                assert_eq!(
                    hub.copy_otp(&v.id, &clock),
                    Err(CoreError::CandidateExpired)
                );
                assert!(hub.views(&clock).is_empty());
            }
            other => panic!("expected candidate, {other:?}"),
        }
    }

    #[test]
    fn duplicate_and_rate_limit() {
        let mut hub = NotifyHub::default();
        let clock = ManualClock::new(5_000);
        let source = DeviceId::from_id_pk(&[1u8; 32]);
        assert!(matches!(
            hub.ingest_push(source.clone(), push(true), &clock),
            IngestOutcome::Candidate(_)
        ));
        assert!(matches!(
            hub.ingest_push(source.clone(), push(true), &clock),
            IngestOutcome::Duplicate
        ));
        for i in 0..12 {
            let mut p = push(false);
            p.uid = format!("u{i}");
            let _ = hub.ingest_push(source.clone(), p, &clock);
        }
        let mut p = push(false);
        p.uid = "overflow".into();
        assert!(matches!(
            hub.ingest_push(source, p, &clock),
            IngestOutcome::RateLimited
        ));
    }
}
