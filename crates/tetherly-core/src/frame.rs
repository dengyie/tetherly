// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::CoreError;
use serde::{Deserialize, Serialize};

pub const INNER_PAYLOAD_MAX: usize = 64 * 1024;
pub const FILE_FRAME_MAX: usize = 256 * 1024;
pub const FLAG_MUST_UNDERSTAND: u16 = 0x0001;

pub const TYPE_PING: u16 = 0x0001;
pub const TYPE_PONG: u16 = 0x0002;
pub const TYPE_CAPS_UPDATE: u16 = 0x00F0;
pub const TYPE_NOTIFY_PUSH: u16 = 0x0101;
pub const TYPE_NOTIFY_DISMISS: u16 = 0x0102;
pub const TYPE_NOTIFY_REPLY: u16 = 0x0103;
pub const TYPE_CLIP_SET: u16 = 0x0201;
pub const TYPE_FILE_OFFER: u16 = 0x0301;
pub const TYPE_FILE_ACCEPT: u16 = 0x0302;
pub const TYPE_FILE_REJECT: u16 = 0x0303;
pub const TYPE_FILE_DONE: u16 = 0x0304;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerFrame {
    pub ty: u16,
    pub flags: u16,
    /// Monotonic per Noise session. Assigned by the sender.
    pub msg_id: u64,
    pub payload: Vec<u8>,
}

impl InnerFrame {
    pub fn new(ty: u16, payload: Vec<u8>) -> Result<Self, CoreError> {
        if payload.len() > INNER_PAYLOAD_MAX {
            return Err(CoreError::PayloadTooLarge);
        }
        Ok(Self {
            ty,
            flags: 0,
            msg_id: 0,
            payload,
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>, CoreError> {
        if self.payload.len() > INNER_PAYLOAD_MAX {
            return Err(CoreError::PayloadTooLarge);
        }
        let mut out = Vec::with_capacity(12 + self.payload.len());
        out.extend_from_slice(&self.ty.to_be_bytes());
        out.extend_from_slice(&self.flags.to_be_bytes());
        out.extend_from_slice(&self.msg_id.to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() < 12 {
            return Err(CoreError::TruncatedFrame);
        }
        let ty = u16::from_be_bytes([bytes[0], bytes[1]]);
        let flags = u16::from_be_bytes([bytes[2], bytes[3]]);
        let msg_id = u64::from_be_bytes(
            bytes[4..12]
                .try_into()
                .map_err(|_| CoreError::TruncatedFrame)?,
        );
        let payload = bytes[12..].to_vec();
        if payload.len() > INNER_PAYLOAD_MAX {
            return Err(CoreError::PayloadTooLarge);
        }
        if flags & FLAG_MUST_UNDERSTAND != 0
            && !is_known_type(ty)
            && !crate::input::is_input_type(ty)
            && !crate::screen::is_screen_type(ty)
        {
            return Err(CoreError::MustUnderstand(ty));
        }
        Ok(Self {
            ty,
            flags,
            msg_id,
            payload,
        })
    }
}

pub fn is_known_type(ty: u16) -> bool {
    matches!(
        ty,
        TYPE_PING
            | TYPE_PONG
            | TYPE_CAPS_UPDATE
            | TYPE_NOTIFY_PUSH
            | TYPE_NOTIFY_DISMISS
            | TYPE_NOTIFY_REPLY
            | TYPE_CLIP_SET
            | TYPE_FILE_OFFER
            | TYPE_FILE_ACCEPT
            | TYPE_FILE_REJECT
            | TYPE_FILE_DONE
    )
}

pub fn ping(unix_ms: u64) -> InnerFrame {
    InnerFrame {
        ty: TYPE_PING,
        flags: 0,
        msg_id: 0,
        payload: unix_ms.to_be_bytes().to_vec(),
    }
}

pub fn pong(unix_ms: u64) -> InnerFrame {
    InnerFrame {
        ty: TYPE_PONG,
        flags: 0,
        msg_id: 0,
        payload: unix_ms.to_be_bytes().to_vec(),
    }
}

pub fn decode_unix_ms(payload: &[u8]) -> Option<u64> {
    if payload.len() != 8 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(payload);
    Some(u64::from_be_bytes(buf))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotifyPush {
    pub uid: String,
    pub app_id: String,
    pub app_name: String,
    pub title: String,
    pub body: String,
    pub ts: u64,
    #[serde(default)]
    pub actions: Vec<String>,
}

impl NotifyPush {
    pub fn truncate(mut self) -> Self {
        const LIMIT: usize = 4 * 1024;
        truncate_utf8(&mut self.title, LIMIT);
        truncate_utf8(&mut self.body, LIMIT);
        self.actions.retain(|a| a != "url" && !a.contains("://"));
        self
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_NOTIFY_PUSH, serde_json::to_vec(self)?)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        let parsed: Self = serde_json::from_slice(payload)?;
        Ok(parsed.truncate())
    }
}

pub const CLIP_TEXT_MAX: usize = 1024 * 1024;
pub const CANDIDATE_TTL_MS: u64 = 120_000;
pub const CLIPBOARD_OTP_CLEAR_MS: u64 = 60_000;
pub const FILE_TOKEN_TTL_MS: u64 = 60_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NotifyDismiss {
    pub uid: String,
    pub app_id: String,
}

impl NotifyDismiss {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_NOTIFY_DISMISS, serde_json::to_vec(self)?)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        Ok(serde_json::from_slice(payload)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClipSet {
    pub mime: String,
    pub text: Option<String>,
    pub blob_ref: Option<String>,
    pub clip_seq: u64,
    pub hash: String,
}

impl ClipSet {
    pub fn text(clip_seq: u64, text: impl Into<String>) -> Result<Self, CoreError> {
        let text = text.into();
        if text.len() > CLIP_TEXT_MAX || text.len() > INNER_PAYLOAD_MAX.saturating_sub(256) {
            return Err(CoreError::ClipboardTooLarge);
        }
        let hash = hex_sha256(text.as_bytes());
        Ok(Self {
            mime: "text/plain".into(),
            text: Some(text),
            blob_ref: None,
            clip_seq,
            hash,
        })
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_CLIP_SET, serde_json::to_vec(self)?)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        Ok(serde_json::from_slice(payload)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileMeta {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileOffer {
    pub transfer_id: String,
    pub files: Vec<FileMeta>,
}

impl FileOffer {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_FILE_OFFER, serde_json::to_vec(self)?)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        Ok(serde_json::from_slice(payload)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDecision {
    pub transfer_id: String,
}

impl FileDecision {
    pub fn accept_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_FILE_ACCEPT, serde_json::to_vec(self)?)
    }

    pub fn reject_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_FILE_REJECT, serde_json::to_vec(self)?)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        Ok(serde_json::from_slice(payload)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDone {
    pub transfer_id: String,
    pub sha256: String,
}

impl FileDone {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_FILE_DONE, serde_json::to_vec(self)?)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        Ok(serde_json::from_slice(payload)?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapsUpdate {
    pub caps: Vec<String>,
}

impl CapsUpdate {
    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_CAPS_UPDATE, serde_json::to_vec(self)?)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        Ok(serde_json::from_slice(payload)?)
    }
}

pub fn hex_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    hex_lower(&digest)
}

pub fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

fn truncate_utf8(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_roundtrip() {
        let f = ping(1_760_054_400_000);
        let enc = f.encode().unwrap();
        let dec = InnerFrame::decode(&enc).unwrap();
        assert_eq!(dec.ty, TYPE_PING);
        assert_eq!(decode_unix_ms(&dec.payload), Some(1_760_054_400_000));
    }

    #[test]
    fn unknown_must_understand_disconnects() {
        let f = InnerFrame {
            ty: 0xFFFF,
            flags: FLAG_MUST_UNDERSTAND,
            msg_id: 1,
            payload: vec![],
        };
        let enc = f.encode().unwrap();
        assert!(matches!(
            InnerFrame::decode(&enc),
            Err(CoreError::MustUnderstand(0xFFFF))
        ));
    }

    #[test]
    fn unknown_without_flag_is_kept() {
        let f = InnerFrame {
            ty: 0xFFFF,
            flags: 0,
            msg_id: 1,
            payload: b"x".to_vec(),
        };
        let dec = InnerFrame::decode(&f.encode().unwrap()).unwrap();
        assert_eq!(dec.ty, 0xFFFF);
    }

    #[test]
    fn notify_truncates_and_strips_urls() {
        let n = NotifyPush {
            uid: "1".into(),
            app_id: "com.apple.MobileSMS".into(),
            app_name: "信息".into(),
            title: "x".repeat(5000),
            body: "y".repeat(5000),
            ts: 1,
            actions: vec!["copy".into(), "https://evil".into()],
        }
        .truncate();
        assert_eq!(n.title.len(), 4096);
        assert_eq!(n.body.len(), 4096);
        assert_eq!(n.actions, vec!["copy".to_string()]);
    }

    #[test]
    fn truncate_does_not_split_multibyte_char() {
        // 4094 ASCII + one CJK char (3 bytes) would land mid-character at 4096.
        let n = NotifyPush {
            uid: "1".into(),
            app_id: "sms".into(),
            app_name: "信息".into(),
            title: format!("{}验", "x".repeat(4094)),
            body: format!("{}码", "y".repeat(4095)),
            ts: 1,
            actions: vec![],
        }
        .truncate();
        assert!(n.title.is_char_boundary(n.title.len()));
        assert!(n.body.is_char_boundary(n.body.len()));
        assert!(n.title.len() <= 4096);
        assert!(n.body.len() <= 4096);
        assert!(!n.title.contains('验'));
        assert!(!n.body.contains('码'));
    }
}
