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
    #[error("unknown candidate")]
    UnknownCandidate,
    #[error("candidate expired")]
    CandidateExpired,
    #[error("no otp in candidate")]
    NoOtp,
    #[error("clipboard payload too large")]
    ClipboardTooLarge,
    #[error("file not accepted")]
    FileNotAccepted,
    #[error("unknown transfer")]
    UnknownTransfer,
    #[error("file already decided")]
    FileAlreadyDecided,
    #[error("bind address refused")]
    BindRefused,
    #[error("identity file corrupt")]
    IdentityCorrupt,
    #[error("sha256 mismatch")]
    Sha256Mismatch,
    #[error("insert refused: {0}")]
    InsertRefused(String),
    #[error("malformed input frame")]
    InputFrame,
    #[error("input sequence jumped")]
    InputSeqJump,
    #[error("input channel requires an already-trusted peer")]
    InputRequiresTrust,
}

impl From<serde_json::Error> for CoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value.to_string())
    }
}
