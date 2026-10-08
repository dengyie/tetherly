// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Noise_IK_25519_ChaChaPoly_BLAKE2s.
//!
//! `snow` 0.9 `TransportState` has no `rekey()`. After 2^16 transport messages
//! or 3600s, rebuild a fresh IK handshake (same static keys). Do not invent a
//! custom rekey.

use crate::error::CryptoError;
use crate::identity::Identity;
use sha2::{Digest, Sha256};
use snow::{Builder, HandshakeState, TransportState};
use tetherly_core::hello::Hello;

pub const NOISE_PATTERN: &str = "Noise_IK_25519_ChaChaPoly_BLAKE2s";
pub const REKEY_AFTER_MSGS: u64 = 1 << 16;
pub const REKEY_AFTER_SECS: u64 = 3600;

pub fn prologue(hello_a: &Hello, hello_b: &Hello) -> Result<[u8; 32], CryptoError> {
    let (first, second) = if hello_a.device_id.as_str() <= hello_b.device_id.as_str() {
        (hello_a, hello_b)
    } else {
        (hello_b, hello_a)
    };
    let mut h = Sha256::new();
    h.update(&first.to_canonical_bytes()?);
    h.update(&second.to_canonical_bytes()?);
    Ok(h.finalize().into())
}

fn builder(prologue: &[u8]) -> Result<Builder<'_>, CryptoError> {
    let params = NOISE_PATTERN.parse()?;
    Ok(Builder::new(params).prologue(prologue))
}

pub struct NoiseHandshake {
    state: HandshakeState,
}

impl NoiseHandshake {
    pub fn initiator(
        local: &Identity,
        remote_n_pk: &[u8; 32],
        prologue: &[u8; 32],
    ) -> Result<Self, CryptoError> {
        let state = builder(prologue)?
            .local_private_key(local.n_sk())
            .remote_public_key(remote_n_pk)
            .build_initiator()?;
        Ok(Self { state })
    }

    pub fn responder(local: &Identity, prologue: &[u8; 32]) -> Result<Self, CryptoError> {
        let state = builder(prologue)?
            .local_private_key(local.n_sk())
            .build_responder()?;
        Ok(Self { state })
    }

    pub fn write_message(&mut self, payload: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut buf = vec![0u8; 65535];
        let n = self.state.write_message(payload, &mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }

    pub fn read_message(&mut self, msg: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut buf = vec![0u8; 65535];
        let n = self.state.read_message(msg, &mut buf)?;
        buf.truncate(n);
        Ok(buf)
    }

    pub fn is_handshake_finished(&self) -> bool {
        self.state.is_handshake_finished()
    }

    pub fn into_transport(self) -> Result<NoiseTransport, CryptoError> {
        Ok(NoiseTransport {
            state: self.state.into_transport_mode()?,
            msgs: 0,
            started_unix: 0,
        })
    }
}

pub struct NoiseTransport {
    state: TransportState,
    msgs: u64,
    started_unix: u64,
}

impl NoiseTransport {
    pub fn set_started(&mut self, unix: u64) {
        self.started_unix = unix;
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut buf = vec![0u8; plaintext.len() + 16 + 32];
        let n = self.state.write_message(plaintext, &mut buf)?;
        buf.truncate(n);
        self.msgs = self.msgs.saturating_add(1);
        Ok(buf)
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut buf = vec![0u8; ciphertext.len()];
        let n = self.state.read_message(ciphertext, &mut buf)?;
        buf.truncate(n);
        self.msgs = self.msgs.saturating_add(1);
        Ok(buf)
    }

    pub fn needs_rekey(&self, now_unix: u64) -> bool {
        self.msgs >= REKEY_AFTER_MSGS
            || (self.started_unix > 0
                && now_unix.saturating_sub(self.started_unix) >= REKEY_AFTER_SECS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    #[test]
    fn ik_roundtrip() {
        let a = Identity::from_secrets([11u8; 32], [12u8; 32]);
        let b = Identity::from_secrets([21u8; 32], [22u8; 32]);
        let ha = a.hello("a", "linux", &["notify"]);
        let hb = b.hello("b", "linux", &["notify"]);
        let p = prologue(&ha, &hb).unwrap();
        let mut init = NoiseHandshake::initiator(&a, b.n_pk(), &p).unwrap();
        let mut resp = NoiseHandshake::responder(&b, &p).unwrap();
        let m1 = init.write_message(b"").unwrap();
        resp.read_message(&m1).unwrap();
        let m2 = resp.write_message(b"").unwrap();
        init.read_message(&m2).unwrap();
        assert!(init.is_handshake_finished());
        assert!(resp.is_handshake_finished());
        let mut ta = init.into_transport().unwrap();
        let mut tb = resp.into_transport().unwrap();
        let ct = ta.encrypt(b"ping").unwrap();
        assert_eq!(tb.decrypt(&ct).unwrap(), b"ping");
        ta.set_started(1_000);
        assert!(!ta.needs_rekey(1_000 + 3_599));
        assert!(ta.needs_rekey(1_000 + 3_600));
    }

    #[test]
    fn prologue_covers_hello_name() {
        let a = Identity::from_secrets([11u8; 32], [12u8; 32]);
        let b = Identity::from_secrets([21u8; 32], [22u8; 32]);
        let ha = a.hello("a", "linux", &["notify"]);
        let hb = b.hello("b", "linux", &["notify"]);
        let mut hb_mitm = hb.clone();
        hb_mitm.name = "evil-alias".into();
        assert_ne!(
            prologue(&ha, &hb).unwrap(),
            prologue(&ha, &hb_mitm).unwrap()
        );
    }

    #[test]
    fn wrong_static_fails() {
        let a = Identity::from_secrets([11u8; 32], [12u8; 32]);
        let b = Identity::from_secrets([21u8; 32], [22u8; 32]);
        let c = Identity::from_secrets([31u8; 32], [32u8; 32]);
        let ha = a.hello("a", "linux", &["notify"]);
        let hb = b.hello("b", "linux", &["notify"]);
        let p = prologue(&ha, &hb).unwrap();
        let mut init = NoiseHandshake::initiator(&a, c.n_pk(), &p).unwrap();
        let mut resp = NoiseHandshake::responder(&b, &p).unwrap();
        let m1 = init.write_message(b"").unwrap();
        assert!(resp.read_message(&m1).is_err());
    }
}
