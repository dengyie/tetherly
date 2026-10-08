// SPDX-License-Identifier: Apache-2.0 OR MIT

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CoreError {
    #[error("invalid device id")]
    InvalidDeviceId,
    #[error("hello exceeds 4 KiB")]
    HelloTooLarge,
    #[error("hello proto is not 1")]
    UnsupportedProto,
    #[error("device_id does not match id_pk")]
    DeviceIdMismatch,
    #[error("invalid public key encoding")]
    InvalidPublicKey,
    #[error("inner payload exceeds 64 KiB")]
    PayloadTooLarge,
    #[error("truncated inner frame")]
    TruncatedFrame,
    #[error("unknown inner type 0x{0:04x} with MUST_UNDERSTAND")]
    MustUnderstand(u16),
    #[error("unsafe file name")]
    UnsafeFileName,
    #[error("invalid pin")]
    InvalidPin,
    #[error("rng failure")]
    Rng,
    #[error("peer n_pk changed; re-pair required")]
    StaticKeyChanged,
    #[error("peer is revoked")]
    Revoked,
    #[error("json: {0}")]
    Json(String),
}

impl From<serde_json::Error> for CoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value.to_string())
    }
}
