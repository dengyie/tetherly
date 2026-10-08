// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Identity, Argon2id, SPAKE2, encrypted PairBind, Noise IK. No sockets.

#![forbid(unsafe_code)]

pub mod error;
pub mod identity;
pub mod kdf;
pub mod noise;
pub mod pairbind;
pub mod spake;
pub mod vectors;

pub use error::CryptoError;
pub use identity::Identity;
pub use kdf::{argon2id_pin, derive_pair_password, file_token};
pub use noise::{prologue, NoiseHandshake, NoiseTransport, NOISE_PATTERN};
pub use pairbind::{decrypt_pairbind, encrypt_pairbind, PairBind, PairBindRole};
pub use spake::SpakeRole;
