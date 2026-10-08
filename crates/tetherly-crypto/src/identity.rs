// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::CryptoError;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use tetherly_core::hello::encode_b64_32;
use tetherly_core::{DeviceId, Hello};
use x25519_dalek::{PublicKey as XPublic, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Ed25519 identity + independent X25519 static key. Never reuse the Ed25519
/// scalar as X25519.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Identity {
    id_sk: [u8; 32],
    n_sk: [u8; 32],
    #[zeroize(skip)]
    id_pk: [u8; 32],
    #[zeroize(skip)]
    n_pk: [u8; 32],
    #[zeroize(skip)]
    device_id: DeviceId,
}

impl Identity {
    pub fn generate() -> Result<Self, CryptoError> {
        let mut id_sk = [0u8; 32];
        let mut n_sk = [0u8; 32];
        getrandom::getrandom(&mut id_sk).map_err(|_| CryptoError::Rng)?;
        getrandom::getrandom(&mut n_sk).map_err(|_| CryptoError::Rng)?;
        Ok(Self::from_secrets(id_sk, n_sk))
    }

    pub fn from_secrets(id_sk: [u8; 32], n_sk: [u8; 32]) -> Self {
        let signing = SigningKey::from_bytes(&id_sk);
        let id_pk = signing.verifying_key().to_bytes();
        let static_secret = StaticSecret::from(n_sk);
        let n_pk = XPublic::from(&static_secret).to_bytes();
        let device_id = DeviceId::from_id_pk(&id_pk);
        Self {
            id_sk,
            n_sk,
            id_pk,
            n_pk,
            device_id,
        }
    }

    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    pub fn id_pk(&self) -> &[u8; 32] {
        &self.id_pk
    }

    pub fn n_pk(&self) -> &[u8; 32] {
        &self.n_pk
    }

    pub fn n_sk(&self) -> &[u8; 32] {
        &self.n_sk
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        SigningKey::from_bytes(&self.id_sk).sign(msg).to_bytes()
    }

    pub fn verify(id_pk: &[u8; 32], msg: &[u8], sig: &[u8; 64]) -> Result<(), CryptoError> {
        let vk = VerifyingKey::from_bytes(id_pk).map_err(|_| CryptoError::KeyLength)?;
        let sig = ed25519_dalek::Signature::from_bytes(sig);
        vk.verify_strict(msg, &sig)
            .map_err(|_| CryptoError::PairBindSignature)
    }

    pub fn hello(&self, name: &str, platform: &str, caps: &[&str]) -> Hello {
        Hello {
            proto: tetherly_core::hello::PROTO_V1,
            device_id: self.device_id.clone(),
            id_pk: encode_b64_32(&self.id_pk),
            n_pk: encode_b64_32(&self.n_pk),
            caps: caps.iter().map(|s| (*s).to_string()).collect(),
            name: name.to_string(),
            platform: platform.to_string(),
        }
    }
}

pub fn pairing_salt(id_pk_a: &[u8; 32], id_pk_b: &[u8; 32]) -> [u8; 32] {
    let (lo, hi) = if id_pk_a <= id_pk_b {
        (id_pk_a, id_pk_b)
    } else {
        (id_pk_b, id_pk_a)
    };
    let mut h = Sha256::new();
    h.update(b"tetherly-pair-v1");
    h.update(lo);
    h.update(hi);
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_from_id_pk_not_n_pk() {
        let id = Identity::from_secrets([1u8; 32], [2u8; 32]);
        assert_eq!(id.device_id(), &DeviceId::from_id_pk(id.id_pk()));
        assert_ne!(id.device_id(), &DeviceId::from_id_pk(id.n_pk()));
        let hello = id.hello("t", "linux", &["notify"]);
        hello.validate().unwrap();
    }
}
