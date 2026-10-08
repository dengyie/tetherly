// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::CryptoError;
use crate::kdf::derive_pair_password;
use spake2::{Ed25519Group, Identity as SpakeIdentity, Password, Spake2};
use zeroize::Zeroize;

pub struct SpakeRole {
    state: Option<Spake2<Ed25519Group>>,
    outbound: Vec<u8>,
    password: [u8; 32],
}

impl Drop for SpakeRole {
    fn drop(&mut self) {
        self.password.zeroize();
        self.outbound.zeroize();
    }
}

impl SpakeRole {
    /// Both peers must pass the same `(id_a, id_b)` byte strings. `is_alice`
    /// selects `start_a` vs `start_b`; it is independent of who dialed TCP.
    pub fn start(
        is_alice: bool,
        pin_ascii: &[u8],
        id_pk_left: &[u8; 32],
        id_pk_right: &[u8; 32],
        id_a: &[u8],
        id_b: &[u8],
    ) -> Result<Self, CryptoError> {
        let password = derive_pair_password(pin_ascii, id_pk_left, id_pk_right)?;
        let (state, outbound) = if is_alice {
            Spake2::<Ed25519Group>::start_a(
                &Password::new(password),
                &SpakeIdentity::new(id_a),
                &SpakeIdentity::new(id_b),
            )
        } else {
            Spake2::<Ed25519Group>::start_b(
                &Password::new(password),
                &SpakeIdentity::new(id_a),
                &SpakeIdentity::new(id_b),
            )
        };
        Ok(Self {
            state: Some(state),
            outbound,
            password,
        })
    }

    pub fn outbound_message(&self) -> &[u8] {
        &self.outbound
    }

    pub fn finish(mut self, inbound: &[u8]) -> Result<[u8; 32], CryptoError> {
        let state = self
            .state
            .take()
            .ok_or_else(|| CryptoError::Spake2("used".into()))?;
        let key = state
            .finish(inbound)
            .map_err(|e| CryptoError::Spake2(e.to_string()))?;
        if key.len() < 32 {
            return Err(CryptoError::Spake2("short key".into()));
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&key[..32]);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_pin_same_key() {
        let pin = b"25170394";
        let a_pk = [1u8; 32];
        let b_pk = [2u8; 32];
        let a = SpakeRole::start(true, pin, &a_pk, &b_pk, b"A", b"B").unwrap();
        let b = SpakeRole::start(false, pin, &b_pk, &a_pk, b"A", b"B").unwrap();
        let a_msg = a.outbound_message().to_vec();
        let b_msg = b.outbound_message().to_vec();
        let ka = a.finish(&b_msg).unwrap();
        let kb = b.finish(&a_msg).unwrap();
        assert_eq!(ka, kb);
    }

    #[test]
    fn wrong_pin_fails_or_diverges() {
        let a_pk = [1u8; 32];
        let b_pk = [2u8; 32];
        let a = SpakeRole::start(true, b"25170394", &a_pk, &b_pk, b"A", b"B").unwrap();
        let b = SpakeRole::start(false, b"00000000", &b_pk, &a_pk, b"A", b"B").unwrap();
        let a_msg = a.outbound_message().to_vec();
        let b_msg = b.outbound_message().to_vec();
        if let (Ok(x), Ok(y)) = (a.finish(&b_msg), b.finish(&a_msg)) {
            assert_ne!(x, y);
        }
    }
}
