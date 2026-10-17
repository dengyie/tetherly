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
    #[error("reply text is empty")]
    EmptyReply,
    #[error("clipboard payload too large")]
    ClipboardTooLarge,
    #[error("file not accepted")]
    FileNotAccepted,
    #[error("file exceeds the {0}-byte v1 limit")]
    FileTooLarge(u64),
    #[error("unknown transfer")]
    UnknownTransfer,
    #[error("file already decided")]
    FileAlreadyDecided,
    #[error("resume offset is not chunk-aligned or out of range")]
    ResumeOffset,
    #[error("bind address refused")]
    BindRefused,
    #[error("identity file corrupt")]
    IdentityCorrupt,
    #[error("sha256 mismatch")]
    Sha256Mismatch,
    #[error("insert refused: {0}")]
    InsertRefused(String),
    #[error("open refused: {0}")]
    OpenRefused(String),
    #[error("malformed input frame")]
    InputFrame,
    #[error("input sequence jumped")]
    InputSeqJump,
    #[error("input channel requires an already-trusted peer")]
    InputRequiresTrust,
    #[error("malformed ANCS frame")]
    AncsFrame,
    #[error("ANCS subscribe failed")]
    AncsSubscribe,
    #[error("ANCS control point write failed")]
    AncsWrite,
    #[error("malformed screen frame")]
    ScreenFrame,
    #[error("screen sequence jumped")]
    ScreenSeqJump,
    #[error("screen frame exceeds the wire budget")]
    ScreenTooLarge,
    #[error("screen refused: {0}")]
    ScreenRefused(String),
    #[error("screen fragment stream is malformed")]
    ScreenFragment,
    #[error("screen compression codec failed: {0}")]
    ScreenCodec(String),
    #[error("file error: {0}")]
    File(String),
    #[error("no connected peer")]
    NoConnectedPeer,
    #[error("peer ping timed out")]
    PingTimeout,
    #[error("invalid alias: {0}")]
    InvalidAlias(String),
    #[error("peer not found: {0}")]
    PeerNotFound(String),
}

impl From<serde_json::Error> for CoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(value: std::io::Error) -> Self {
        Self::File(value.to_string())
    }
}
