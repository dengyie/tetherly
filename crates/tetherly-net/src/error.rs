// SPDX-License-Identifier: Apache-2.0 OR MIT

use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Core(#[from] tetherly_core::CoreError),
    #[error(transparent)]
    Crypto(#[from] tetherly_crypto::CryptoError),
    #[error("outer frame too large")]
    TooLarge,
    #[error("handshake incomplete")]
    Handshake,
    #[error("business frame before Noise")]
    Premature,
    #[error("device_id mismatch")]
    DeviceIdMismatch,
    #[error("locked out")]
    LockedOut,
    #[error("wrong pin")]
    WrongPin,
    #[error("mitm / pairing failed")]
    PairingFailed,
    #[error("peer n_pk changed")]
    StaticKeyChanged,
    #[error("discovery: {0}")]
    Discovery(String),
    #[error("handshake timed out")]
    Timeout,
    #[error("tcp rate limited")]
    RateLimited,
    #[error("replayed inner frame")]
    Replay,
    #[error("noise rekey required")]
    RekeyRequired,
}
