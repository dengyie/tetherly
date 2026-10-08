// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::CryptoError;
use crate::identity::pairing_salt;
use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use sha2::Sha256;

pub const ARGON2_M_KIB: u32 = 64 * 1024;
pub const ARGON2_T: u32 = 3;
pub const ARGON2_P: u32 = 1;
pub const ARGON2_OUT: usize = 32;

pub fn argon2id_pin(pin_ascii: &[u8], salt: &[u8; 32]) -> Result<[u8; 32], CryptoError> {
    let params = Params::new(ARGON2_M_KIB, ARGON2_T, ARGON2_P, Some(ARGON2_OUT))
        .map_err(|e| CryptoError::Argon2(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon.hash_password_into(pin_ascii, salt, &mut out)?;
    Ok(out)
}

pub fn derive_pair_password(
    pin_ascii: &[u8],
    id_pk_a: &[u8; 32],
    id_pk_b: &[u8; 32],
) -> Result<[u8; 32], CryptoError> {
    let salt = pairing_salt(id_pk_a, id_pk_b);
    argon2id_pin(pin_ascii, &salt)
}

pub fn file_token(ck: &[u8], transfer_id: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, ck);
    let mut out = [0u8; 32];
    let mut info = Vec::from(b"tetherly-file-v1".as_slice());
    info.extend_from_slice(transfer_id);
    hk.expand(&info, &mut out).expect("hkdf 32");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salt_is_order_invariant() {
        let a = [1u8; 32];
        let b = [9u8; 32];
        assert_eq!(pairing_salt(&a, &b), pairing_salt(&b, &a));
        assert_ne!(pairing_salt(&a, &b), pairing_salt(&a, &a));
    }
}
