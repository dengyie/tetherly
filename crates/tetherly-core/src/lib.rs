// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Protocol-pure types: OTP, frames, Hello, trust, lockout, allowlist.
//! No `cfg(target_os)`, no windows/objc/jni.

#![forbid(unsafe_code)]

pub mod allowlist;
pub mod bind;
pub mod clip;
pub mod dedup;
pub mod device_id;
pub mod error;
pub mod filexfer;
pub mod frame;
pub mod hello;
pub mod insertion;
pub mod notify;
pub mod otp;
pub mod persist;
pub mod pin;
pub mod ports;
pub mod replay;
pub mod session;

pub use allowlist::{sanitize_file_name, OpenAllowlist, OpenRule};
pub use bind::{is_unspecified_bind, should_listen, should_listen_v4};
pub use clip::{ClipApply, ClipHub};
pub use dedup::{ClipLimiter, NotifyLimiter, TcpLimiter, TokenBucket};
pub use device_id::DeviceId;
pub use error::CoreError;
pub use filexfer::{FileHub, IncomingTransfer, TransferStatus};
pub use frame::{
    hex_lower, hex_sha256, CapsUpdate, ClipSet, FileDecision, FileDone, FileMeta, FileOffer,
    InnerFrame, NotifyDismiss, NotifyPush, CANDIDATE_TTL_MS, CLIPBOARD_OTP_CLEAR_MS, CLIP_TEXT_MAX,
    FILE_TOKEN_TTL_MS, TYPE_PING, TYPE_PONG,
};
pub use hello::Hello;
pub use insertion::{plan as insertion_plan, InsertionDecision, InsertionPlan};
pub use notify::{is_desktop_platform, CandidateView, IngestOutcome, NotifyHub};
pub use otp::{extract_otp, DefaultOtpExtractor, OtpExtractor, DEFAULT_MIN_SCORE};
pub use persist::{TrustFile, TrustedPeerDto};
pub use ports::{
    Clip, Clock, Insertor, ManualClock, MemoryTrustStore, PhoneNotification, Rng, SystemClock,
    TrustStore, TrustedPeer,
};
pub use replay::ReplayGuard;
pub use session::{
    backoff_delay, commit_trust, dispose_hello, ConnState, HelloDisposition, LockoutTable, PIN_TTL,
};
