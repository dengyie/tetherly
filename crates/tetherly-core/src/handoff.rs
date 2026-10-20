// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Cross-device Resource & URL Handoff validation, sanitization, and lifecycle primitives.
//!
//! Provides strict scheme filtering, URI injection defense, fail-closed validation,
//! and sanitized zero-plaintext summaries for cross-device links and web resources.

use crate::allowlist::OpenAllowlist;
use crate::error::CoreError;
use crate::frame::hex_sha256;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub const MAX_HANDOFF_URL_LEN: usize = 4096;

/// Policy-driven action to take upon receiving an inbound resource or URL handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffAction {
    /// Automatically invoke the OS default browser or application to open the URL.
    AutoOpen,
    /// Store in the handoff inbox and surface an interactive prompt/notification for the user.
    PromptOnly,
    /// Store in the handoff inbox without notifying or invoking external openers.
    SaveOnly,
    /// Silently reject and discard inbound handoff requests.
    Disabled,
}

impl HandoffAction {
    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }

    pub fn allows_auto_open(&self) -> bool {
        matches!(self, Self::AutoOpen)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AutoOpen => "auto_open",
            Self::PromptOnly => "prompt_only",
            Self::SaveOnly => "save_only",
            Self::Disabled => "disabled",
        }
    }
}

impl FromStr for HandoffAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto_open" | "auto" | "open" => Ok(Self::AutoOpen),
            "prompt_only" | "prompt" | "ask" => Ok(Self::PromptOnly),
            "save_only" | "save" | "inbox_only" => Ok(Self::SaveOnly),
            "disabled" | "off" | "reject" => Ok(Self::Disabled),
            other => Err(format!(
                "invalid handoff action '{other}', expected: auto_open, prompt_only, save_only, disabled"
            )),
        }
    }
}

impl fmt::Display for HandoffAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Lifecycle status of a resource handoff entry in the node's inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    Pending,
    Opened,
    Dismissed,
    Rejected,
}

impl HandoffStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Opened => "opened",
            Self::Dismissed => "dismissed",
            Self::Rejected => "rejected",
        }
    }
}

impl FromStr for HandoffStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pending" => Ok(Self::Pending),
            "opened" | "open" => Ok(Self::Opened),
            "dismissed" | "dismiss" => Ok(Self::Dismissed),
            "rejected" | "reject" => Ok(Self::Rejected),
            other => Err(format!(
                "invalid handoff status '{other}', expected: pending, opened, dismissed, rejected"
            )),
        }
    }
}

impl fmt::Display for HandoffStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Sanitized and validated URL metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SanitizedHandoffUrl {
    pub url: String,
    pub scheme: String,
    pub host: String,
    pub sha256: String,
}

/// A stored entry in the node's handoff inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffEntry {
    pub id: String,
    pub sender_id: String,
    #[serde(default)]
    pub sender_alias: String,
    pub url: String,
    #[serde(default)]
    pub title: String,
    pub status: HandoffStatus,
    pub received_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_ms: Option<u64>,
}

/// Zero-dependency security validator and firewall for cross-device URL and resource handoffs.
pub struct ResourceHandoffValidator;

impl ResourceHandoffValidator {
    /// Validates and sanitizes a URL intended for cross-device handoff.
    ///
    /// # Security Invariants
    /// 1. Length must not exceed [`MAX_HANDOFF_URL_LEN`] (4096 bytes).
    /// 2. Banned characters: control chars (`0x00..=0x1F`, `0x7F`), whitespace, `\0`, `\r`, `\n`, `\`, `..`.
    /// 3. Scheme must be `http` or `https`, or an allowlisted local scheme (e.g. `weixin://`, `tetherly://`).
    /// 4. Disallows dangerous schemes: `file://`, `javascript:`, `vbscript:`, `data:`, `cmd:`, `powershell:`, `ssh:`, etc.
    /// 5. Banned userinfo credentials: `http://user:pass@host` to prevent credential harvesting or misleading URLs.
    /// 6. Banned cloud metadata endpoints: `169.254.169.254`.
    pub fn validate(
        raw_url: &str,
        allowlist: Option<&OpenAllowlist>,
    ) -> Result<SanitizedHandoffUrl, CoreError> {
        let trimmed = raw_url.trim();
        if trimmed.is_empty() {
            return Err(CoreError::Json("handoff url cannot be empty".into()));
        }
        if trimmed.len() > MAX_HANDOFF_URL_LEN {
            return Err(CoreError::PayloadTooLarge);
        }

        // Check for forbidden characters
        for b in trimmed.bytes() {
            if b <= 0x1F || b == 0x7F || b == b' ' || b == b'\\' || b == 0 {
                return Err(CoreError::Json(
                    "handoff url contains invalid control or whitespace characters".into(),
                ));
            }
        }
        if trimmed.contains("..") {
            return Err(CoreError::UnsafeFileName);
        }

        let colon_pos = trimmed
            .find(':')
            .ok_or_else(|| CoreError::Json("handoff url missing scheme separator".into()))?;
        let scheme = trimmed[..colon_pos].to_ascii_lowercase();

        // Check banned dangerous schemes
        const BANNED_SCHEMES: &[&str] = &[
            "file",
            "javascript",
            "data",
            "blob",
            "vbscript",
            "cmd",
            "powershell",
            "ssh",
            "telnet",
            "ftp",
            "tftp",
            "ms-msdt",
            "ws",
            "wss",
        ];
        if BANNED_SCHEMES.contains(&scheme.as_str()) {
            return Err(CoreError::Json(format!(
                "forbidden dangerous url scheme: '{scheme}'"
            )));
        }

        let is_http_family = scheme == "http" || scheme == "https";

        let host = if is_http_family {
            let prefix = if scheme == "http" {
                "http://"
            } else {
                "https://"
            };
            if !trimmed.to_ascii_lowercase().starts_with(prefix) {
                return Err(CoreError::Json("invalid http/https url syntax".into()));
            }
            let authority_and_path = &trimmed[prefix.len()..];
            if authority_and_path.is_empty() {
                return Err(CoreError::Json("missing host in http/https url".into()));
            }

            // Extract authority portion (before first '/', '?', '#')
            let authority_end = authority_and_path
                .find(['/', '?', '#'])
                .unwrap_or(authority_and_path.len());
            let authority = &authority_and_path[..authority_end];

            // Forbid userinfo credentials in authority (e.g., user:pass@evil.com)
            if authority.contains('@') {
                return Err(CoreError::Json(
                    "userinfo credentials not permitted in handoff url".into(),
                ));
            }

            // Extract host (strip port if present)
            let host_part = if let Some(stripped) = authority.strip_prefix('[') {
                // IPv6 bracketed address: [::1]:8080
                let closing = stripped
                    .find(']')
                    .ok_or_else(|| CoreError::Json("unclosed ipv6 bracket".into()))?;
                &stripped[..closing]
            } else if let Some(colon) = authority.find(':') {
                &authority[..colon]
            } else {
                authority
            };

            let host_clean = host_part.trim().to_ascii_lowercase();
            if host_clean.is_empty() {
                return Err(CoreError::Json("empty host in handoff url".into()));
            }

            // Reject cloud instance metadata IP
            if host_clean == "169.254.169.254" {
                return Err(CoreError::Json(
                    "forbidden access to cloud metadata address".into(),
                ));
            }

            host_clean
        } else {
            // Local custom app scheme: verify against OpenAllowlist if provided
            let scheme_allowed = if let Some(al) = allowlist {
                al.rules().iter().any(|r| {
                    let r_scheme = r.url.split(':').next().unwrap_or("");
                    r_scheme.eq_ignore_ascii_case(&scheme)
                })
            } else {
                matches!(
                    scheme.as_str(),
                    "weixin" | "wechat" | "alipay" | "taobao" | "tetherly"
                )
            };

            if !scheme_allowed {
                return Err(CoreError::Json(format!(
                    "url scheme '{scheme}' is not allowed for handoff"
                )));
            }

            scheme.clone()
        };

        let sha256 = hex_sha256(trimmed.as_bytes());

        Ok(SanitizedHandoffUrl {
            url: trimmed.to_string(),
            scheme,
            host,
            sha256,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handoff_action_and_status_parsing() {
        assert_eq!(
            HandoffAction::from_str("auto_open").unwrap(),
            HandoffAction::AutoOpen
        );
        assert_eq!(
            HandoffAction::from_str("prompt_only").unwrap(),
            HandoffAction::PromptOnly
        );
        assert_eq!(
            HandoffAction::from_str("save_only").unwrap(),
            HandoffAction::SaveOnly
        );
        assert_eq!(
            HandoffAction::from_str("disabled").unwrap(),
            HandoffAction::Disabled
        );

        assert!(HandoffAction::AutoOpen.allows_auto_open());
        assert!(!HandoffAction::PromptOnly.allows_auto_open());
        assert!(HandoffAction::PromptOnly.is_enabled());
        assert!(!HandoffAction::Disabled.is_enabled());

        assert_eq!(
            HandoffStatus::from_str("pending").unwrap(),
            HandoffStatus::Pending
        );
        assert_eq!(
            HandoffStatus::from_str("opened").unwrap(),
            HandoffStatus::Opened
        );
        assert_eq!(
            HandoffStatus::from_str("dismissed").unwrap(),
            HandoffStatus::Dismissed
        );
        assert_eq!(
            HandoffStatus::from_str("rejected").unwrap(),
            HandoffStatus::Rejected
        );
    }

    #[test]
    fn test_valid_urls_pass_validation() {
        let u1 = ResourceHandoffValidator::validate("https://github.com/rust-lang/rust", None)
            .expect("valid https url");
        assert_eq!(u1.scheme, "https");
        assert_eq!(u1.host, "github.com");
        assert_eq!(u1.url, "https://github.com/rust-lang/rust");
        assert_eq!(u1.sha256.len(), 64);

        let u2 = ResourceHandoffValidator::validate("http://192.168.1.100:8080/dashboard", None)
            .expect("valid http url with port");
        assert_eq!(u2.scheme, "http");
        assert_eq!(u2.host, "192.168.1.100");

        let u3 = ResourceHandoffValidator::validate("tetherly://open?id=123", None)
            .expect("valid tetherly custom scheme");
        assert_eq!(u3.scheme, "tetherly");
    }

    #[test]
    fn test_malicious_and_banned_urls_rejected() {
        // Dangerous schemes
        assert!(
            ResourceHandoffValidator::validate("file:///etc/passwd", None).is_err(),
            "file:// must be rejected"
        );
        assert!(
            ResourceHandoffValidator::validate("javascript:alert(1)", None).is_err(),
            "javascript: must be rejected"
        );
        assert!(
            ResourceHandoffValidator::validate("data:text/html,<html></html>", None).is_err(),
            "data: must be rejected"
        );
        assert!(
            ResourceHandoffValidator::validate("cmd://calc.exe", None).is_err(),
            "cmd:// must be rejected"
        );
        assert!(
            ResourceHandoffValidator::validate("powershell://invoke", None).is_err(),
            "powershell:// must be rejected"
        );
        assert!(
            ResourceHandoffValidator::validate("ssh://root@evil.com", None).is_err(),
            "ssh:// must be rejected"
        );

        // Userinfo credential hijacking
        assert!(
            ResourceHandoffValidator::validate("http://admin:pass@example.com", None).is_err(),
            "userinfo credentials must be rejected"
        );

        // Cloud metadata IP
        assert!(
            ResourceHandoffValidator::validate("http://169.254.169.254/latest/meta-data/", None)
                .is_err(),
            "metadata IP must be rejected"
        );

        // Control characters & whitespace
        assert!(
            ResourceHandoffValidator::validate("https://example.com/\r\nSet-Cookie: evil", None)
                .is_err(),
            "CRLF must be rejected"
        );
        assert!(
            ResourceHandoffValidator::validate("https://example.com/foo bar", None).is_err(),
            "whitespace must be rejected"
        );
        assert!(
            ResourceHandoffValidator::validate("https://example.com/..", None).is_err(),
            "traversal must be rejected"
        );
    }
}
