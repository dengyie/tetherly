// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::CoreError;
use data_encoding::BASE32_NOPAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// `tdev_` + RFC 4648 base32 (lowercase, no pad) of `sha256(id_pk)[0..10]`.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceId(String);

const PREFIX: &str = "tdev_";
const BASE32_LEN: usize = 16;

impl DeviceId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn parse(raw: &str) -> Result<Self, CoreError> {
        let rest = raw.strip_prefix(PREFIX).ok_or(CoreError::InvalidDeviceId)?;
        if rest.len() != BASE32_LEN {
            return Err(CoreError::InvalidDeviceId);
        }
        if !rest.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7')) {
            return Err(CoreError::InvalidDeviceId);
        }
        Ok(Self(raw.to_string()))
    }

    pub fn from_id_pk(id_pk: &[u8; 32]) -> Self {
        let digest = Sha256::digest(id_pk);
        Self::from_id_pk_hash10(digest[..10].try_into().expect("10 bytes"))
    }

    pub fn from_id_pk_hash10(hash10: [u8; 10]) -> Self {
        let mut out = String::with_capacity(PREFIX.len() + BASE32_LEN);
        out.push_str(PREFIX);
        out.push_str(&BASE32_NOPAD.encode(&hash10).to_ascii_lowercase());
        Self(out)
    }

    pub fn fingerprint12(id_pk: &[u8; 32]) -> String {
        let digest = Sha256::digest(id_pk);
        hex_lower(&digest[..6])
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0f) as usize] as char);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_bytes_encode_to_sixteen_chars() {
        let id = DeviceId::from_id_pk(&[0u8; 32]);
        assert!(id.as_str().starts_with("tdev_"));
        assert_eq!(id.as_str().len(), PREFIX.len() + BASE32_LEN);
        assert_eq!(DeviceId::parse(id.as_str()).unwrap(), id);
    }

    #[test]
    fn mismatch_is_detectable() {
        let a = DeviceId::from_id_pk(&[1u8; 32]);
        let b = DeviceId::from_id_pk(&[2u8; 32]);
        assert_ne!(a, b);
        assert!(DeviceId::parse("not-an-id").is_err());
        assert!(DeviceId::parse("tdev_AAAAAAAAAAAAAAAA").is_err());
    }

    #[test]
    fn fingerprint_is_twelve_hex() {
        let fp = DeviceId::fingerprint12(&[9u8; 32]);
        assert_eq!(fp.len(), 12);
        assert!(fp.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
