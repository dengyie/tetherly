// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Protocol-pure types: OTP, frames, Hello, trust, lockout, allowlist.
//! No `cfg(target_os)`, no windows/objc/jni.

#![forbid(unsafe_code)]

pub mod allowlist;
pub mod ancs;
pub mod bind;
pub mod clip;
pub mod dedup;
pub mod device_id;
pub mod error;
pub mod filexfer;
pub mod frame;
pub mod handoff;
pub mod hello;
pub mod input;
pub mod insertion;
pub mod liveness;
pub mod media;
pub mod notify;
pub mod otp;
pub mod overlay;
pub mod persist;
pub mod pin;
pub mod ports;
pub mod replay;
pub mod screen;
pub mod session;

pub use allowlist::{sanitize_file_name, OpenAllowlist, OpenRule};
pub use ancs::{
    ancs_actions, ancs_backoff_delay, ancs_source_id, parse_ble_address, AncsCharacteristic,
    AncsEvent, AncsIngress, AncsNotification, AncsState, AncsTransport, BleAddress, Category,
    ConnectOutcome, DataAssembler, DataItem, EventFlags, EventId, MemoryAncsTransport,
    NotificationEvent, APP_NAME_WAIT_MS, ATTR_MESSAGE, ATTR_TITLE, NOTIF_ATTR_REQUEST,
    UUID_ANCS_SERVICE, UUID_CONTROL_POINT, UUID_DATA_SOURCE, UUID_NOTIFICATION_SOURCE,
};
pub use bind::{
    classify, classify_v4, is_unspecified_bind, should_listen, should_listen_v4, BindClass,
};
pub use clip::{
    ClipApply, ClipDirection, ClipHub, ClipMetaEntry, ClipSyncMode, SensitiveContentGuard,
    SensitiveLeakReason, MAX_CLIP_HISTORY,
};
pub use dedup::{ClipLimiter, NotifyLimiter, TcpLimiter, TokenBucket};
pub use device_id::DeviceId;
pub use error::CoreError;
pub use filexfer::{FileHub, IncomingTransfer, ResumeState, ResumeTable, TransferStatus, FILE_MAX};
pub use frame::{
    hex_lower, hex_sha256, CapsUpdate, ClipSet, FileDecision, FileDone, FileMeta, FileOffer,
    IncrementalSha256, InnerFrame, NotifyDismiss, NotifyPush, NotifyReply, ResourceHandoffFrame,
    TextMessage, CANDIDATE_TTL_MS, CLIPBOARD_OTP_CLEAR_MS, CLIP_TEXT_MAX, FILE_CHUNK,
    FILE_TOKEN_TTL_MS, HANDOFF_TITLE_MAX, HANDOFF_URL_MAX, NOTIFY_TEXT_MAX, TEXT_MESSAGE_MAX,
    TYPE_MEDIA_COMMAND, TYPE_MEDIA_STATE, TYPE_PING, TYPE_PONG, TYPE_RESOURCE_HANDOFF,
    TYPE_TEXT_MESSAGE,
};
pub use handoff::{
    HandoffAction, HandoffEntry, HandoffStatus, ResourceHandoffValidator, SanitizedHandoffUrl,
    MAX_HANDOFF_URL_LEN,
};
pub use hello::Hello;
pub use input::{
    is_input_type, CursorSeat, InputClient, InputEvent, InputKind, InputServer, InputSink,
    MemorySink, ScreenEdge, BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, INPUT_MAGIC, SCREEN_UNITS,
    TYPE_INPUT_BUTTON, TYPE_INPUT_CLIP_HINT, TYPE_INPUT_ENTER, TYPE_INPUT_KEY, TYPE_INPUT_LEAVE,
    TYPE_INPUT_MOVE, TYPE_INPUT_WHEEL,
};
pub use insertion::{plan as insertion_plan, InsertionDecision, InsertionPlan};
pub use liveness::{Liveness, SESSION_HEARTBEAT, SESSION_LIVENESS_DEADLINE};
pub use media::{
    MediaCommand, MediaCommandFrame, MediaControlMode, MediaStateFrame, MediaValidator,
    PlaybackState, MAX_MEDIA_ALBUM_LEN, MAX_MEDIA_ARTIST_LEN, MAX_MEDIA_PLAYER_ID_LEN,
    MAX_MEDIA_TITLE_LEN,
};
pub use notify::{is_desktop_platform, CandidateView, IngestOutcome, NotifyHub, ReplyView};
pub use otp::{extract_otp, DefaultOtpExtractor, OtpExtractor, DEFAULT_MIN_SCORE};
pub use overlay::{
    default_overlay_cidrs, iface_looks_easytier, in_overlay, overlay_ips_from_json, parse_cidr,
    path_kind, prefer_existing, PathKind, DEFAULT_RPC_PORT, OVERLAY_PROBE_MS,
};
pub use persist::{TrustFile, TrustedPeerDto};
pub use ports::{
    Clip, Clock, Insertor, ManualClock, MemoryTrustStore, NotificationPayload, NotificationUrgency,
    Notifier, Opener, PhoneNotification, Rng, ScreenCodec, ScreenSink, ScreenSource, SystemClock,
    TrustStore, TrustedPeer,
};
pub use replay::ReplayGuard;
pub use screen::{
    is_screen_type, ControlMsg, MemoryScreenSink, MemoryScreenSource, PixelFormat, ScreenAssembler,
    ScreenFrame, ScreenReceiver, ScreenRect, ScreenSender, ScreenState, ScreenStats, SCREEN_MAGIC,
    SCREEN_PROTO, SCREEN_SEQ_WINDOW,
};
pub use session::{
    backoff_delay, commit_trust, dispose_hello, ConnState, HelloDisposition, LockoutTable, PIN_TTL,
};
