// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Frozen Phase 0 test vectors. PIN `25170394`, deterministic identity seeds.
//!
//! Argon2id and Noise prologue digests are deterministic. SPAKE2 messages are
//! not: the crate draws ephemeral scalars, so M0.6 asserts equal session keys
//! plus the frozen KDF/prologue hex, then a Noise IK round-trip.

use crate::identity::Identity;

pub const VECTOR_PIN: &[u8; 8] = b"25170394";
pub const VECTOR_ID_SK_A: [u8; 32] = [0xA1; 32];
pub const VECTOR_N_SK_A: [u8; 32] = [0xA2; 32];
pub const VECTOR_ID_SK_B: [u8; 32] = [0xB1; 32];
pub const VECTOR_N_SK_B: [u8; 32] = [0xB2; 32];

/// Argon2id(PIN, salt(id_pk_A, id_pk_B), m=64MiB, t=3, p=1) for the seeds above.
pub const VECTOR_ARGON2ID_HEX: &str =
    "37dd853bcc520d123726d5a6cae0e852c8b9ee8100c28e5949b0a67d13901065";

/// sha256(canonical_hello_lo || canonical_hello_hi) with names alice/bob.
pub const VECTOR_PROLOGUE_HEX: &str =
    "b90692b4a9b6ee24f2a636edaebdb1d0e299e36a668a9053f4f374c5bb657b82";

pub fn identities() -> (Identity, Identity) {
    (
        Identity::from_secrets(VECTOR_ID_SK_A, VECTOR_N_SK_A),
        Identity::from_secrets(VECTOR_ID_SK_B, VECTOR_N_SK_B),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::pairing_salt;
    use crate::kdf::{argon2id_pin, derive_pair_password};
    use crate::noise::{prologue, NoiseHandshake, NOISE_PATTERN};
    use crate::spake::SpakeRole;

    fn hex_lower(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 0x0f) as usize] as char);
        }
        s
    }

    #[test]
    fn frozen_argon2_spake2_prologue() {
        assert_eq!(NOISE_PATTERN, "Noise_IK_25519_ChaChaPoly_BLAKE2s");
        let (a, b) = identities();

        let salt = pairing_salt(a.id_pk(), b.id_pk());
        let pwd = argon2id_pin(VECTOR_PIN, &salt).unwrap();
        let pwd2 = derive_pair_password(VECTOR_PIN, b.id_pk(), a.id_pk()).unwrap();
        assert_eq!(pwd, pwd2);
        let pwd_hex = hex_lower(&pwd);
        assert_eq!(
            pwd_hex, VECTOR_ARGON2ID_HEX,
            "Argon2id digest changed — protocol break. got={pwd_hex}"
        );

        let ha = a.hello("alice", "linux", &["notify", "clip", "file"]);
        let hb = b.hello("bob", "linux", &["notify", "clip", "file"]);
        let p = prologue(&ha, &hb).unwrap();
        let p_hex = hex_lower(&p);
        assert_eq!(
            p_hex, VECTOR_PROLOGUE_HEX,
            "Noise prologue changed — protocol break. got={p_hex}"
        );

        let id_a = a.device_id().as_str().as_bytes();
        let id_b = b.device_id().as_str().as_bytes();
        let (id_lo, id_hi, a_is_alice) = if a.device_id().as_str() <= b.device_id().as_str() {
            (id_a, id_b, true)
        } else {
            (id_b, id_a, false)
        };
        let sa =
            SpakeRole::start(a_is_alice, VECTOR_PIN, a.id_pk(), b.id_pk(), id_lo, id_hi).unwrap();
        let sb =
            SpakeRole::start(!a_is_alice, VECTOR_PIN, b.id_pk(), a.id_pk(), id_lo, id_hi).unwrap();
        let a_msg = sa.outbound_message().to_vec();
        let b_msg = sb.outbound_message().to_vec();
        let ka = sa.finish(&b_msg).unwrap();
        let kb = sb.finish(&a_msg).unwrap();
        assert_eq!(ka, kb);

        let mut init = NoiseHandshake::initiator(&a, b.n_pk(), &p).unwrap();
        let mut resp = NoiseHandshake::responder(&b, &p).unwrap();
        let m1 = init.write_message(b"").unwrap();
        resp.read_message(&m1).unwrap();
        let m2 = resp.write_message(b"").unwrap();
        init.read_message(&m2).unwrap();
        assert!(init.is_handshake_finished());
        let mut ta = init.into_transport().unwrap();
        let mut tb = resp.into_transport().unwrap();
        let ct = ta.encrypt(b"vector").unwrap();
        assert_eq!(tb.decrypt(&ct).unwrap(), b"vector");
    }
}
