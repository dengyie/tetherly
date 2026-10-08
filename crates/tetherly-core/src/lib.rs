// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Protocol-pure types: OTP, frames, Hello, trust, lockout, allowlist.
//! No `cfg(target_os)`, no windows/objc/jni.

#![forbid(unsafe_code)]

pub mod allowlist;
pub mod dedup;
pub mod device_id;
pub mod error;
pub mod frame;
pub mod hello;
pub mod insertion;
pub mod otp;
pub mod pin;
pub mod ports;
pub mod replay;
pub mod session;

pub use allowlist::{sanitize_file_name, OpenAllowlist, OpenRule};
pub use dedup::{TcpLimiter, TokenBucket};
pub use device_id::DeviceId;
pub use error::CoreError;
pub use frame::{InnerFrame, NotifyPush};
pub use hello::Hello;
pub use insertion::{plan as insertion_plan, InsertionPlan};
pub use otp::{extract_otp, DefaultOtpExtractor, OtpExtractor, DEFAULT_MIN_SCORE};
pub use ports::{Clock, ManualClock, PhoneNotification, Rng, SystemClock, TrustStore, TrustedPeer};
pub use replay::ReplayGuard;
pub use session::{dispose_hello, ConnState, HelloDisposition, LockoutTable};
