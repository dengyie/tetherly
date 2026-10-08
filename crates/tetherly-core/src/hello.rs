// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::device_id::DeviceId;
use crate::error::CoreError;
use serde::{Deserialize, Serialize};

pub const HELLO_MAX_BYTES: usize = 4096;
pub const PROTO_V1: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hello {
    pub proto: u32,
    pub device_id: DeviceId,
    pub id_pk: String,
    pub n_pk: String,
    pub caps: Vec<String>,
    pub name: String,
    pub platform: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedKeys {
    pub id_pk: [u8; 32],
    pub n_pk: [u8; 32],
}

impl Hello {
    pub fn decode_keys(&self) -> Result<DecodedKeys, CoreError> {
        Ok(DecodedKeys {
            id_pk: decode_b64_32(&self.id_pk)?,
            n_pk: decode_b64_32(&self.n_pk)?,
        })
    }

    pub fn validate(&self) -> Result<DecodedKeys, CoreError> {
        if self.proto != PROTO_V1 {
            return Err(CoreError::UnsupportedProto);
        }
        let keys = self.decode_keys()?;
        let expected = DeviceId::from_id_pk(&keys.id_pk);
        if expected != self.device_id {
            return Err(CoreError::DeviceIdMismatch);
        }
        Ok(keys)
    }

    pub fn to_canonical_bytes(&self) -> Result<Vec<u8>, CoreError> {
        #[derive(Serialize)]
        struct Canonical<'a> {
            proto: u32,
            device_id: &'a str,
            id_pk: &'a str,
            n_pk: &'a str,
            caps: &'a [String],
            name: &'a str,
            platform: &'a str,
        }
        let json = serde_json::to_vec(&Canonical {
            proto: self.proto,
            device_id: self.device_id.as_str(),
            id_pk: &self.id_pk,
            n_pk: &self.n_pk,
            caps: &self.caps,
            name: &self.name,
            platform: &self.platform,
        })?;
        if json.len() > HELLO_MAX_BYTES {
            return Err(CoreError::HelloTooLarge);
        }
        Ok(json)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        if bytes.len() > HELLO_MAX_BYTES {
            return Err(CoreError::HelloTooLarge);
        }
        let hello: Hello = serde_json::from_slice(bytes)?;
        hello.validate()?;
        Ok(hello)
    }
}

pub fn decode_b64_32(s: &str) -> Result<[u8; 32], CoreError> {
    use base64::Engine;
    let raw = base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|_| CoreError::InvalidPublicKey)?;
    raw.try_into().map_err(|_| CoreError::InvalidPublicKey)
}

pub fn encode_b64_32(bytes: &[u8; 32]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Hello {
        let id_pk = [7u8; 32];
        Hello {
            proto: 1,
            device_id: DeviceId::from_id_pk(&id_pk),
            id_pk: encode_b64_32(&id_pk),
            n_pk: encode_b64_32(&[9u8; 32]),
            caps: vec!["notify".into(), "clip".into(), "file".into()],
            name: "Mango-PC".into(),
            platform: "windows".into(),
        }
    }

    #[test]
    fn roundtrip_and_validate() {
        let h = sample();
        let bytes = h.to_canonical_bytes().unwrap();
        let parsed = Hello::from_bytes(&bytes).unwrap();
        assert_eq!(parsed, h);
    }

    #[test]
    fn device_id_mismatch_rejected() {
        let mut h = sample();
        h.device_id = DeviceId::from_id_pk(&[1u8; 32]);
        let bytes = serde_json::to_vec(&h).unwrap();
        assert_eq!(
            Hello::from_bytes(&bytes).unwrap_err(),
            CoreError::DeviceIdMismatch
        );
    }

    #[test]
    fn too_large_rejected() {
        let mut h = sample();
        h.name = "x".repeat(5000);
        assert_eq!(
            h.to_canonical_bytes().unwrap_err(),
            CoreError::HelloTooLarge
        );
    }
}
