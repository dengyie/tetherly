// SPDX-License-Identifier: Apache-2.0 OR MIT

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("identity key length")]
    KeyLength,
    #[error("argon2: {0}")]
    Argon2(String),
    #[error("spake2: {0}")]
    Spake2(String),
    #[error("noise: {0}")]
    Noise(String),
    #[error("pairbind decrypt failed")]
    PairBind,
    #[error("pairbind signature invalid")]
    PairBindSignature,
    #[error("pairbind device_id mismatch")]
    PairBindId,
    #[error("pin expired")]
    PinExpired,
    #[error("locked out")]
    LockedOut,
    #[error("wrong pin")]
    WrongPin,
    #[error("hello prologue mismatch")]
    Prologue,
    #[error("rng failure")]
    Rng,
    #[error("core: {0}")]
    Core(#[from] tetherly_core::CoreError),
}

impl From<snow::Error> for CryptoError {
    fn from(value: snow::Error) -> Self {
        Self::Noise(value.to_string())
    }
}

impl From<argon2::password_hash::Error> for CryptoError {
    fn from(value: argon2::password_hash::Error) -> Self {
        Self::Argon2(value.to_string())
    }
}

impl From<argon2::Error> for CryptoError {
    fn from(value: argon2::Error) -> Self {
        Self::Argon2(value.to_string())
    }
}
