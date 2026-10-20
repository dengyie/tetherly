// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Clipboard hub. `clip_seq` + last-hash prevent echo loops. 4/s per source.
//! Text larger than 1 MiB is refused (spec: go via file).

use std::collections::VecDeque;

use crate::dedup::ClipLimiter;
use crate::device_id::DeviceId;
use crate::error::CoreError;
use crate::frame::{hex_sha256, ClipSet, CLIP_TEXT_MAX};

pub const MAX_CLIP_HISTORY: usize = 32;

/// Clipboard synchronization operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ClipSyncMode {
    /// Only synchronize on explicit manual user request (default secure state).
    #[default]
    Manual,
    /// Automatically synchronize local clipboard changes to peers and apply remote updates.
    BidirectionalAuto,
    /// Automatically synchronize local clipboard changes out, but never overwrite local clipboard from remote.
    SendOnly,
    /// Accept remote clipboard updates, but never automatically broadcast local clipboard.
    ReceiveOnly,
}

impl ClipSyncMode {
    /// Whether this mode permits automatically broadcasting local clipboard changes.
    pub fn can_send(&self) -> bool {
        matches!(self, Self::BidirectionalAuto | Self::SendOnly)
    }

    /// Whether this mode permits applying incoming remote clipboard frames.
    pub fn can_receive(&self) -> bool {
        !matches!(self, Self::SendOnly)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::BidirectionalAuto => "bidirectional_auto",
            Self::SendOnly => "send_only",
            Self::ReceiveOnly => "receive_only",
        }
    }
}

impl std::str::FromStr for ClipSyncMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "manual" | "off" | "disabled" => Ok(Self::Manual),
            "bidirectional" | "bidirectional_auto" | "auto" | "both" => Ok(Self::BidirectionalAuto),
            "send_only" | "send" | "sender" => Ok(Self::SendOnly),
            "receive_only" | "recv_only" | "receive" | "recv" | "receiver" => Ok(Self::ReceiveOnly),
            other => Err(format!(
                "invalid clip sync mode '{other}', expected: manual, bidirectional_auto, send_only, receive_only"
            )),
        }
    }
}

impl std::fmt::Display for ClipSyncMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Categorized reason why a clipboard content was blocked by the sensitive leak guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensitiveLeakReason {
    /// Asymmetric or symmetric private keys (e.g. RSA, EC, OpenSSH, PGP).
    PrivateKey,
    /// Cloud provider or platform API access tokens (e.g. AWS, GitHub, OpenAI, Slack).
    ApiToken,
    /// JSON Web Tokens (JWT) or Bearer authentication headers.
    BearerJwt,
    /// Password manager exports or structured password records.
    PasswordManagerPayload,
}

impl SensitiveLeakReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PrivateKey => "private_key",
            Self::ApiToken => "api_token",
            Self::BearerJwt => "bearer_jwt",
            Self::PasswordManagerPayload => "password_manager_payload",
        }
    }
}

impl std::fmt::Display for SensitiveLeakReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Zero-Plaintext safety guard to prevent accidental network broadcast of sensitive secrets.
#[derive(Debug, Clone, Copy, Default)]
pub struct SensitiveContentGuard;

impl SensitiveContentGuard {
    /// Inspects text content for high-risk secrets and private credentials.
    /// Returns `Some(reason)` if the text matches a sensitive pattern that must not be broadcast.
    pub fn inspect(text: &str) -> Option<SensitiveLeakReason> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }

        // 1. Private Keys: PEM / OpenSSH / PGP / PKCS
        if trimmed.contains("-----BEGIN") {
            let upper = trimmed.to_ascii_uppercase();
            if upper.contains("PRIVATE KEY")
                || upper.contains("RSA PRIVATE KEY")
                || upper.contains("EC PRIVATE KEY")
                || upper.contains("DSA PRIVATE KEY")
                || upper.contains("OPENSSH PRIVATE KEY")
                || upper.contains("PGP PRIVATE KEY BLOCK")
            {
                return Some(SensitiveLeakReason::PrivateKey);
            }
        }

        // 2. Cloud and Platform API Tokens
        // AWS Access Key ID: AKIA or ASIA followed by 16 alphanumeric characters
        if let Some(pos) = trimmed.find("AKIA") {
            if Self::is_aws_key_at(trimmed, pos) {
                return Some(SensitiveLeakReason::ApiToken);
            }
        }
        if let Some(pos) = trimmed.find("ASIA") {
            if Self::is_aws_key_at(trimmed, pos) {
                return Some(SensitiveLeakReason::ApiToken);
            }
        }

        // GitHub tokens: ghp_ (personal), gho_ (oauth), ghu_ (user-to-server), ghs_ (server-to-server), github_pat_
        if trimmed.contains("ghp_")
            || trimmed.contains("gho_")
            || trimmed.contains("ghu_")
            || trimmed.contains("ghs_")
            || trimmed.contains("github_pat_")
        {
            return Some(SensitiveLeakReason::ApiToken);
        }

        // OpenAI API keys / Stripe live keys: sk- or sk-proj- or sk_live_
        if (trimmed.contains("sk-") || trimmed.contains("sk_live_") || trimmed.contains("sk_test_"))
            && Self::has_openai_or_stripe_key(trimmed)
        {
            return Some(SensitiveLeakReason::ApiToken);
        }

        // Slack tokens: xoxb-, xoxp-, xoxa-, xoxr-
        if trimmed.contains("xoxb-")
            || trimmed.contains("xoxp-")
            || trimmed.contains("xoxa-")
            || trimmed.contains("xoxr-")
        {
            return Some(SensitiveLeakReason::ApiToken);
        }

        // 3. Bearer JWT Tokens: standard 3-part base64url separated by dots (e.g. eyJhbGci... . eyJ... . ...)
        if Self::is_jwt_bearer_token(trimmed) {
            return Some(SensitiveLeakReason::BearerJwt);
        }

        // 4. Password Manager Payload / Structured Export
        if Self::is_password_manager_payload(trimmed) {
            return Some(SensitiveLeakReason::PasswordManagerPayload);
        }

        None
    }

    fn is_aws_key_at(text: &str, pos: usize) -> bool {
        let remainder = &text[pos..];
        if remainder.len() >= 20 {
            let key_candidate = &remainder[..20];
            let rest = &key_candidate[4..20];
            if rest
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            {
                let before_ok = pos == 0
                    || !text[..pos]
                        .chars()
                        .last()
                        .map(|c| c.is_ascii_alphanumeric())
                        .unwrap_or(false);
                let after_ok = remainder.len() == 20
                    || !remainder[20..]
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_alphanumeric())
                        .unwrap_or(false);
                return before_ok && after_ok;
            }
        }
        false
    }

    fn has_openai_or_stripe_key(text: &str) -> bool {
        for segment in text.split(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_') {
            if (segment.starts_with("sk-")
                || segment.starts_with("sk_live_")
                || segment.starts_with("sk_test_"))
                && segment.len() >= 24
            {
                return true;
            }
        }
        false
    }

    fn is_jwt_bearer_token(text: &str) -> bool {
        let clean = text.strip_prefix("Bearer ").unwrap_or(text).trim();
        if clean.starts_with("eyJ") {
            let parts: Vec<&str> = clean.split('.').collect();
            if parts.len() == 3 {
                let is_b64url = |p: &str| {
                    !p.is_empty()
                        && p.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                };
                if is_b64url(parts[0]) && is_b64url(parts[1]) && is_b64url(parts[2]) {
                    return true;
                }
            }
        }
        false
    }

    fn is_password_manager_payload(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        if lower.contains("bitwarden")
            && (lower.contains("password")
                || lower.contains("\"encrypted\"")
                || lower.contains("folderid"))
        {
            return true;
        }
        if lower.contains("op://")
            || (lower.contains("1password")
                && (lower.contains("password") || lower.contains("vault")))
        {
            return true;
        }
        if lower.contains("keepass")
            && (lower.contains("<key>")
                || lower.contains("<keepassfile>")
                || lower.contains("<pwentry>"))
        {
            return true;
        }
        false
    }
}

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

    #[test]
    fn test_clip_sync_mode_parsing_and_directionality() {
        use std::str::FromStr;

        let m1 = ClipSyncMode::from_str("manual").unwrap();
        assert_eq!(m1, ClipSyncMode::Manual);
        assert!(!m1.can_send());
        assert!(m1.can_receive());

        let m2 = ClipSyncMode::from_str("bidirectional_auto").unwrap();
        assert_eq!(m2, ClipSyncMode::BidirectionalAuto);
        assert!(m2.can_send());
        assert!(m2.can_receive());

        let m3 = ClipSyncMode::from_str("send_only").unwrap();
        assert_eq!(m3, ClipSyncMode::SendOnly);
        assert!(m3.can_send());
        assert!(!m3.can_receive());

        let m4 = ClipSyncMode::from_str("receive_only").unwrap();
        assert_eq!(m4, ClipSyncMode::ReceiveOnly);
        assert!(!m4.can_send());
        assert!(m4.can_receive());

        assert!(ClipSyncMode::from_str("invalid_mode").is_err());
    }

    #[test]
    fn test_sensitive_content_guard_detection() {
        // Safe normal text
        assert_eq!(
            SensitiveContentGuard::inspect("Hello world from Tetherly!"),
            None
        );
        assert_eq!(
            SensitiveContentGuard::inspect("https://github.com/tetherly/tetherly"),
            None
        );
        assert_eq!(SensitiveContentGuard::inspect("let mut x = 42;"), None);

        // 1. Private keys
        let rsa_key =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEAz5...\n-----END RSA PRIVATE KEY-----";
        assert_eq!(
            SensitiveContentGuard::inspect(rsa_key),
            Some(SensitiveLeakReason::PrivateKey)
        );

        let ssh_key = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjE...\n-----END OPENSSH PRIVATE KEY-----";
        assert_eq!(
            SensitiveContentGuard::inspect(ssh_key),
            Some(SensitiveLeakReason::PrivateKey)
        );

        let ec_key = "-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEI...\n-----END EC PRIVATE KEY-----";
        assert_eq!(
            SensitiveContentGuard::inspect(ec_key),
            Some(SensitiveLeakReason::PrivateKey)
        );

        // 2. Cloud and Platform API Tokens
        let aws_key = "AccessKeyId: AKIAIOSFODNN7EXAMPLE in config";
        assert_eq!(
            SensitiveContentGuard::inspect(aws_key),
            Some(SensitiveLeakReason::ApiToken)
        );

        let gh_token = "export GITHUB_TOKEN=ghp_1234567890abcdefghijklmnopqrstuvwxyz";
        assert_eq!(
            SensitiveContentGuard::inspect(gh_token),
            Some(SensitiveLeakReason::ApiToken)
        );

        let openai_key = "OPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF";
        assert_eq!(
            SensitiveContentGuard::inspect(openai_key),
            Some(SensitiveLeakReason::ApiToken)
        );

        let slack_token = "Slack bot token: xoxb-123456789012-abcdefghijklmnop";
        assert_eq!(
            SensitiveContentGuard::inspect(slack_token),
            Some(SensitiveLeakReason::ApiToken)
        );

        // 3. JWT Bearer Tokens
        let jwt = "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIiwibmFtZSI6IkpvaG4gRG9lIiwiaWF0IjoxNTE2MjM5MDIyfQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        assert_eq!(
            SensitiveContentGuard::inspect(jwt),
            Some(SensitiveLeakReason::BearerJwt)
        );

        // 4. Password Manager Payloads
        let bitwarden_export =
            "{\"bitwarden\": true, \"password\": \"secret123\", \"encrypted\": true}";
        assert_eq!(
            SensitiveContentGuard::inspect(bitwarden_export),
            Some(SensitiveLeakReason::PasswordManagerPayload)
        );

        let op_ref = "op://vault-id/item-id/password";
        assert_eq!(
            SensitiveContentGuard::inspect(op_ref),
            Some(SensitiveLeakReason::PasswordManagerPayload)
        );
    }
}
