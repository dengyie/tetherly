// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::CryptoError;
use crate::identity::Identity;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use sha2::{Digest, Sha256};
use tetherly_core::DeviceId;

/// Direction-specific PairBind nonces. RFC 8439 forbids reusing (key, nonce)
/// for two distinct plaintexts. Both peers share `pair_key` for one pairing,
/// so initiator and responder MUST use distinct 12-byte nonces.
pub const BIND_NONCE_INITIATOR: [u8; 12] = *b"tetherlybndI";
pub const BIND_NONCE_RESPONDER: [u8; 12] = *b"tetherlybndR";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairBindRole {
    Initiator,
    Responder,
}

impl PairBindRole {
    pub fn nonce(self) -> [u8; 12] {
        match self {
            PairBindRole::Initiator => BIND_NONCE_INITIATOR,
            PairBindRole::Responder => BIND_NONCE_RESPONDER,
        }
    }

    pub fn peer(self) -> Self {
        match self {
            PairBindRole::Initiator => PairBindRole::Responder,
            PairBindRole::Responder => PairBindRole::Initiator,
        }
    }
}

#[derive(Clone)]
pub struct PairBind {
    pub n_pk: [u8; 32],
    pub id_pk: [u8; 32],
    pub device_id: DeviceId,
    pub sig: [u8; 64],
}

impl PairBind {
    pub fn sign(identity: &Identity) -> Self {
        let mut msg = Vec::with_capacity(32 + 32 + identity.device_id().as_str().len());
        msg.extend_from_slice(identity.id_pk());
        msg.extend_from_slice(identity.n_pk());
        msg.extend_from_slice(identity.device_id().as_str().as_bytes());
        let sig = identity.sign(&msg);
        Self {
            n_pk: *identity.n_pk(),
            id_pk: *identity.id_pk(),
            device_id: identity.device_id().clone(),
            sig,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(32 + 32 + 64 + 1 + self.device_id.as_str().len());
        out.extend_from_slice(&self.id_pk);
        out.extend_from_slice(&self.n_pk);
        out.extend_from_slice(&self.sig);
        let id = self.device_id.as_str().as_bytes();
        out.push(id.len() as u8);
        out.extend_from_slice(id);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() < 32 + 32 + 64 + 1 {
            return Err(CryptoError::PairBind);
        }
        let id_pk: [u8; 32] = bytes[0..32].try_into().map_err(|_| CryptoError::PairBind)?;
        let n_pk: [u8; 32] = bytes[32..64]
            .try_into()
            .map_err(|_| CryptoError::PairBind)?;
        let sig: [u8; 64] = bytes[64..128]
            .try_into()
            .map_err(|_| CryptoError::PairBind)?;
        let id_len = bytes[128] as usize;
        if bytes.len() != 129 + id_len {
            return Err(CryptoError::PairBind);
        }
        let device_id = DeviceId::parse(
            std::str::from_utf8(&bytes[129..]).map_err(|_| CryptoError::PairBind)?,
        )?;
        Ok(Self {
            n_pk,
            id_pk,
            device_id,
            sig,
        })
    }

    pub fn verify_self(&self) -> Result<(), CryptoError> {
        if DeviceId::from_id_pk(&self.id_pk) != self.device_id {
            return Err(CryptoError::PairBindId);
        }
        let mut msg = Vec::new();
        msg.extend_from_slice(&self.id_pk);
        msg.extend_from_slice(&self.n_pk);
        msg.extend_from_slice(self.device_id.as_str().as_bytes());
        Identity::verify(&self.id_pk, &msg, &self.sig)
    }
}

fn seal_key(pair_key: &[u8]) -> Key {
    let mut h = Sha256::new();
    h.update(b"tetherly-pairbind-v1");
    h.update(pair_key);
    let digest = h.finalize();
    Key::clone_from_slice(&digest[..32])
}

pub fn encrypt_pairbind(
    pair_key: &[u8],
    bind: &PairBind,
    role: PairBindRole,
) -> Result<Vec<u8>, CryptoError> {
    let cipher = ChaCha20Poly1305::new(&seal_key(pair_key));
    cipher
        .encrypt(Nonce::from_slice(&role.nonce()), bind.encode().as_ref())
        .map_err(|_| CryptoError::PairBind)
}

pub fn decrypt_pairbind(
    pair_key: &[u8],
    ciphertext: &[u8],
    peer_role: PairBindRole,
) -> Result<PairBind, CryptoError> {
    let cipher = ChaCha20Poly1305::new(&seal_key(pair_key));
    let plain = cipher
        .decrypt(Nonce::from_slice(&peer_role.nonce()), ciphertext)
        .map_err(|_| CryptoError::PairBind)?;
    let bind = PairBind::decode(&plain)?;
    bind.verify_self()?;
    Ok(bind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    #[test]
    fn roundtrip_direction_nonces() {
        let a = Identity::from_secrets([3u8; 32], [4u8; 32]);
        let b = Identity::from_secrets([5u8; 32], [6u8; 32]);
        let bind_a = PairBind::sign(&a);
        let bind_b = PairBind::sign(&b);
        let key = [7u8; 32];
        let ct_a = encrypt_pairbind(&key, &bind_a, PairBindRole::Initiator).unwrap();
        let ct_b = encrypt_pairbind(&key, &bind_b, PairBindRole::Responder).unwrap();
        assert_ne!(ct_a, ct_b);
        let back_a = decrypt_pairbind(&key, &ct_a, PairBindRole::Initiator).unwrap();
        let back_b = decrypt_pairbind(&key, &ct_b, PairBindRole::Responder).unwrap();
        assert_eq!(back_a.n_pk, *a.n_pk());
        assert_eq!(back_b.n_pk, *b.n_pk());
        assert!(decrypt_pairbind(&key, &ct_a, PairBindRole::Responder).is_err());
        assert!(decrypt_pairbind(&[8u8; 32], &ct_a, PairBindRole::Initiator).is_err());
        assert_ne!(BIND_NONCE_INITIATOR, BIND_NONCE_RESPONDER);
    }
}
