// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Protocol-pure types: OTP, frames, Hello, trust, lockout, allowlist.
//! No `cfg(target_os)`, no windows/objc/jni.

#![forbid(unsafe_code)]

pub mod action;
pub mod allowlist;
pub mod ancs;
pub mod audio_routing;
pub mod battery;
pub mod bench;
pub mod bind;
pub mod camera;
pub mod clip;
pub mod dedup;
pub mod device_id;
pub mod error;
pub mod filexfer;
pub mod focus;
pub mod frame;
pub mod handoff;
pub mod hello;
pub mod input;
pub mod input_tunnel;
pub mod insertion;
pub mod liveness;
pub mod locate;
pub mod media;
pub mod microphone;
pub mod notify;
pub mod otp;
pub mod overlay;
pub mod persist;
pub mod pin;
pub mod ports;
pub mod recommend;
pub mod replay;
pub mod screen;
pub mod session;
pub mod topology;
pub mod virtual_display;
pub mod wire;

pub use action::{
    ActionItem, ActionListReqFrame, ActionListRespFrame, ActionResultFrame, ActionTriggerFrame,
    ActionValidator, DEFAULT_ACTION_TIMEOUT_SECS, MAX_ACTION_DESC_LEN, MAX_ACTION_ERROR_LEN,
    MAX_ACTION_ICON_LEN, MAX_ACTION_ID_LEN, MAX_ACTION_NAME_LEN, MAX_ACTION_OUTPUT_LEN,
    MAX_ACTION_TIMEOUT_SECS, MIN_ACTION_TIMEOUT_SECS,
};
pub use allowlist::{sanitize_file_name, OpenAllowlist, OpenRule};
pub use ancs::{
    ancs_actions, ancs_backoff_delay, ancs_source_id, parse_ble_address, AncsCharacteristic,
    AncsEvent, AncsIngress, AncsNotification, AncsState, AncsTransport, BleAddress, Category,
    ConnectOutcome, DataAssembler, DataItem, EventFlags, EventId, MemoryAncsTransport,
    NotificationEvent, APP_NAME_WAIT_MS, ATTR_MESSAGE, ATTR_TITLE, NOTIF_ATTR_REQUEST,
    UUID_ANCS_SERVICE, UUID_CONTROL_POINT, UUID_DATA_SOURCE, UUID_NOTIFICATION_SOURCE,
};
pub use audio_routing::{
    AudioChannelLayout, AudioRoutingAnnounceFrame, AudioRoutingAttachFrame,
    AudioRoutingCapabilities, AudioRoutingChunkFrame, AudioRoutingDetachFrame, AudioRoutingRole,
    AudioRoutingValidator, AudioSampleFormat, AudioSpatialPosition, AudioStreamCodec,
};
pub use battery::{BatteryStateFrame, BatteryValidator, PowerSource};
pub use bench::{
    BenchAcceptFrame, BenchDataFrame, BenchDirection, BenchReportFrame, BenchRequestFrame,
    BenchRole, BenchValidator,
};
pub use bind::{
    classify, classify_v4, is_unspecified_bind, should_listen, should_listen_v4, BindClass,
};
pub use camera::{
    CameraAnnounceFrame, CameraAttachFrame, CameraCapabilities, CameraCodec, CameraDetachFrame,
    CameraFacing, CameraFrameChunk, CameraLens, CameraResolution, CameraRole, CameraValidator,
};
pub use clip::{
    ClipApply, ClipDirection, ClipHub, ClipMetaEntry, ClipSyncMode, SensitiveContentGuard,
    SensitiveLeakReason, MAX_CLIP_HISTORY,
};
pub use dedup::{ClipLimiter, NotifyLimiter, TcpLimiter, TokenBucket};
pub use device_id::DeviceId;
pub use error::CoreError;
pub use filexfer::{FileHub, IncomingTransfer, ResumeState, ResumeTable, TransferStatus, FILE_MAX};
pub use focus::{
    FocusMode, FocusStateFrame, FocusSyncReqFrame, FocusValidator, DEFAULT_FOCUS_DURATION_MINS,
    MAX_FOCUS_DURATION_MINS, MAX_FOCUS_REASON_LEN, MIN_FOCUS_DURATION_MINS,
};
pub use frame::{
    hex_lower, hex_sha256, CapsUpdate, ClipSet, FileDecision, FileDone, FileMeta, FileOffer,
    IncrementalSha256, InnerFrame, NotifyDismiss, NotifyPush, NotifyReply, ResourceHandoffFrame,
    TextMessage, CANDIDATE_TTL_MS, CLIPBOARD_OTP_CLEAR_MS, CLIP_TEXT_MAX, FILE_CHUNK,
    FILE_TOKEN_TTL_MS, HANDOFF_TITLE_MAX, HANDOFF_URL_MAX, NOTIFY_TEXT_MAX, TEXT_MESSAGE_MAX,
    TYPE_ACTION_LIST_REQ, TYPE_ACTION_LIST_RESP, TYPE_ACTION_RESULT, TYPE_ACTION_TRIGGER,
    TYPE_AUDIO_ROUTING_ANNOUNCE, TYPE_AUDIO_ROUTING_ATTACH, TYPE_AUDIO_ROUTING_CHUNK,
    TYPE_AUDIO_ROUTING_DETACH, TYPE_BATTERY_STATE, TYPE_BENCH_ACCEPT, TYPE_BENCH_DATA,
    TYPE_BENCH_REPORT, TYPE_BENCH_REQUEST, TYPE_CAMERA_ANNOUNCE, TYPE_CAMERA_ATTACH,
    TYPE_CAMERA_DETACH, TYPE_CAMERA_FRAME_CHUNK, TYPE_FOCUS_STATE, TYPE_FOCUS_SYNC_REQ,
    TYPE_LOCATE_REQUEST, TYPE_LOCATE_STOP, TYPE_MEDIA_COMMAND, TYPE_MEDIA_STATE, TYPE_MIC_ANNOUNCE,
    TYPE_MIC_ATTACH, TYPE_MIC_AUDIO_CHUNK, TYPE_MIC_DETACH, TYPE_PING, TYPE_PONG,
    TYPE_RESOURCE_HANDOFF, TYPE_TEXT_MESSAGE, TYPE_TOPOLOGY_STATE, TYPE_TOPOLOGY_SYNC_REQ,
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
pub use input_tunnel::{
    InputTunnelHeartbeatFrame, InputTunnelReleaseFrame, InputTunnelRequestFrame,
    InputTunnelRespFrame, InputTunnelRole, InputTunnelValidator,
};
pub use insertion::{plan as insertion_plan, InsertionDecision, InsertionPlan};
pub use liveness::{Liveness, SESSION_HEARTBEAT, SESSION_LIVENESS_DEADLINE};
pub use locate::{
    LocatePattern, LocateRequestFrame, LocateStopFrame, LocateValidator,
    DEFAULT_LOCATE_DURATION_SECS, MAX_LOCATE_DURATION_SECS, MIN_LOCATE_DURATION_SECS,
};
pub use media::{
    MediaCommand, MediaCommandFrame, MediaControlMode, MediaStateFrame, MediaValidator,
    PlaybackState, MAX_MEDIA_ALBUM_LEN, MAX_MEDIA_ARTIST_LEN, MAX_MEDIA_PLAYER_ID_LEN,
    MAX_MEDIA_TITLE_LEN,
};
pub use microphone::{
    MicAnnounceFrame, MicAttachFrame, MicAudioChunkFrame, MicCapabilities, MicCodec,
    MicDetachFrame, MicDevice, MicMode, MicRole, MicValidator,
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
pub use recommend::{
    recommend_streaming_params, AudioTier, CameraTier, DisplayTier, MicTier,
    StreamingRecommendation,
};
pub use replay::ReplayGuard;
pub use screen::{
    is_screen_type, ControlMsg, MemoryScreenSink, MemoryScreenSource, PixelFormat, ScreenAssembler,
    ScreenFpsPacer, ScreenFrame, ScreenRateController, ScreenReceiver, ScreenRect, ScreenSender,
    ScreenState, ScreenStats, SCREEN_BITRATE_COOLDOWN_US, SCREEN_BITRATE_DEFAULT_BPS,
    SCREEN_BITRATE_MAX_BPS, SCREEN_BITRATE_MIN_BPS, SCREEN_BITRATE_STEP_INCREASE_BPS,
    SCREEN_COMPRESS_H264, SCREEN_COMPRESS_NONE, SCREEN_COMPRESS_ZSTD, SCREEN_FPS_DEFAULT,
    SCREEN_FPS_MAX, SCREEN_FPS_MIN, SCREEN_MAGIC, SCREEN_PROTO, SCREEN_SEQ_WINDOW,
};
pub use session::{
    backoff_delay, commit_trust, dispose_hello, ConnState, HelloDisposition, LockoutTable, PIN_TTL,
};
pub use topology::{
    map_edge_exit_to_entry, DisplayEdge, DisplayRect, EdgeCrossingConfig, EdgeNeighbor,
    TopologyStateFrame, TopologySyncReqFrame, TopologyValidator,
};
pub use virtual_display::{
    VirtualDisplayAnnounceFrame, VirtualDisplayAttachFrame, VirtualDisplayCapabilities,
    VirtualDisplayCodec, VirtualDisplayDetachFrame, VirtualDisplayFrameAck, VirtualDisplayMode,
    VirtualDisplayPixelFormat, VirtualDisplayRole, VirtualDisplayValidator,
};
