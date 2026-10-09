// SPDX-License-Identifier: Apache-2.0 OR MIT
//! On-disk DTO for the trust store. IO belongs in the node; this crate only
//! maps bytes. Identity secrets are encoded by tetherly-crypto.

use crate::device_id::DeviceId;
use crate::error::CoreError;
use crate::hello::{decode_b64_32, encode_b64_32};
use crate::ports::TrustedPeer;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedPeerDto {
    pub device_id: DeviceId,
    pub id_pk: String,
    pub n_pk: String,
    pub alias: String,
    pub paired_at: u64,
    pub revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TrustFile {
    pub peers: Vec<TrustedPeerDto>,
}

impl TrustedPeerDto {
    pub fn from_peer(p: &TrustedPeer) -> Self {
        Self {
            device_id: p.device_id.clone(),
            id_pk: encode_b64_32(&p.id_pk),
            n_pk: encode_b64_32(&p.n_pk),
            alias: p.alias.clone(),
            paired_at: p.paired_at,
            revoked: p.revoked,
        }
    }

    pub fn into_peer(self) -> Result<TrustedPeer, CoreError> {
        Ok(TrustedPeer {
            device_id: self.device_id,
            id_pk: decode_b64_32(&self.id_pk)?,
            n_pk: decode_b64_32(&self.n_pk)?,
            alias: self.alias,
            paired_at: self.paired_at,
            revoked: self.revoked,
        })
    }
}

impl TrustFile {
    pub fn from_peers<'a>(peers: impl IntoIterator<Item = &'a TrustedPeer>) -> Self {
        Self {
            peers: peers.into_iter().map(TrustedPeerDto::from_peer).collect(),
        }
    }

    pub fn into_peers(self) -> Result<Vec<TrustedPeer>, CoreError> {
        self.peers.into_iter().map(|d| d.into_peer()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let p = TrustedPeer {
            device_id: DeviceId::from_id_pk(&[7u8; 32]),
            id_pk: [7u8; 32],
            n_pk: [8u8; 32],
            alias: "phone".into(),
            paired_at: 9,
            revoked: false,
        };
        let json = serde_json::to_vec(&TrustFile::from_peers([&p])).unwrap();
        let back = serde_json::from_slice::<TrustFile>(&json)
            .unwrap()
            .into_peers()
            .unwrap();
        assert_eq!(back[0], p);
    }
}
