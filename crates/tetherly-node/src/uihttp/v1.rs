// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `/api/v1` facade (ADR-139): typed DTOs from `tetherly-api`, one error
//! envelope, version headers on every response. Auth tiers are enforced in
//! `handle()` before this runs.

use super::{write_http_response, UiResponse};
use crate::apiv1::hub::EventHub;
use crate::apiv1::journal::Frame;
use crate::apiv1::model::system;
use crate::Node;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::sync::Arc;
use tetherly_api::{
    ActionPolicyPatch,
    ActionTriggerRequest,
    AncsAllowRequest,
    AncsRemoveRequest,
    AudioAttachRequest,
    AudioDetachRequest,
    AudioPolicyPatch,
    BatteryPolicyPatch,
    BatteryStatePatch,
    BenchPolicyPatch,
    BenchStartRequest,
    BlockRequest,
    CameraAttachRequest,
    CameraDetachRequest,
    CameraPolicyPatch,
    CapsUpdate,
    ConnectRequest,
    DevicePatch,
    Diagnostics,
    DialReport,
    DialRequest,
    DisplayAttachRequest,
    DisplayDetachRequest,
    DisplayPolicyPatch,
    ErrorBody,
    ErrorCode,
    Event,
    FocusPolicyPatch,
    FocusSetRequest,
    HandoffInboxActionRequest,
    HandoffPolicyPatch,
    HandoffSendRequest,
    Health,
    HeartbeatPatch,
    InputAttachRequest,
    InputPolicyPatch,
    InputReleaseRequest,
    InputStartRequest,
    LinkProbeRequest,
    LocalActionRequest,
    LocatePolicyPatch,
    LocateRingRequest,
    LocateStopRequest,
    LogLevel,
    LogLevelRequest,
    LogTail,
    MediaCommandRequest,
    MediaPolicyPatch,
    MediaStatePatch,
    MicAttachRequest,
    MicDetachRequest,
    MicPolicyPatch,
    Patch,
    PeerProfileDto,
    PeerProfilePatch,
    PeerProfileUpdated,
    PinGrant,
    RemoteActionRefreshRequest,
    ScreenAnnotationAdded,
    ScreenAnnotationCreateRequest,
    ScreenAnnotations,
    ScreenAuthorRequest,
    ScreenBitratePatch,
    ScreenColorPatch,
    ScreenCursorPatch,
    ScreenDeltaPatch,
    ScreenFecPatch,
    ScreenFocusPatch,
    ScreenFpsPatch,
    ScreenHarqPatch,
    ScreenLaserPatch,
    ScreenLaserReport,
    ScreenLatencyPatch,
    ScreenLtrPatch,
    ScreenMatrixModePatch,
    ScreenMotionPatch,
    ScreenPacerPatch,
    ScreenPirPatch,
    ScreenPlaybackStartRequest,
    ScreenPlayoutPatch,
    ScreenPrivacyPatch,
    ScreenProbePatch,
    ScreenProfilePatch,
    ScreenQoePatch,
    ScreenQualityMetricPatch,
    ScreenRecordingSnapshotRequest,
    ScreenRecordingStartRequest,
    ScreenRoiPatch,
    ScreenScalePatch,
    ScreenSnapshotRequest,
    ScreenSquelchPatch,
    // §167: screen domain contract types.
    ScreenStartRequest,
    ScreenSvcDropT1Request,
    ScreenSvcPatch,
    ScreenSwitchRequest,
    ScreenSyncPatch,
    ScreenTileBatchPatch,
    ScreenTileCachePatch,
    ScreenTileLodPatch,
    ScreenTileMotionPatch,
    ScreenTileNackPatch,
    ScreenTilePalettePatch,
    ScreenTilePatch,
    ScreenTilePredictPatch,
    ScreenTilePresentPatch,
    ScreenTileSaliencyPatch,
    ScreenTrendlinePatch,
    ScreenTuningPatch,
    ScreenTwccPatch,
    ScreenWatermarkEnableRequest,
    ScreenWatermarkPatch,
    ScreenWhiteboardExportReport,
    ScreenWhiteboardExportRequest,
    ScreenWhiteboardImportReport,
    ScreenWhiteboardImportRequest,
    ScreenWhiteboardPolicyPatch,
    ScreenWhiteboardPolicyView,
    ScreenWhiteboardRedoReport,
    ScreenWhiteboardUndoReport,
    SettingsPatch,
    SysHost,
    SysProcess,
    SysTelemetry,
    Topic,
    API_VERSION,
};
use tetherly_core::CoreError;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::broadcast::error::RecvError;

/// Stamp the contract and daemon version on a v1 response.
fn versioned(resp: UiResponse) -> UiResponse {
    resp.with_header(tetherly_api::API_HEADER, API_VERSION.to_string())
        .with_header(tetherly_api::VERSION_HEADER, env!("CARGO_PKG_VERSION"))
}

fn json<T: Serialize>(code: u16, v: &T) -> UiResponse {
    match serde_json::to_vec(v) {
        Ok(b) => versioned(UiResponse::new(code, "application/json; charset=utf-8", b)),
        Err(e) => error(ErrorCode::Internal, format!("encode: {e}")),
    }
}

/// A v1 error response: `{"error":{"code","message"}}` with the code's status.
pub(super) fn error(code: ErrorCode, message: impl Into<String>) -> UiResponse {
    let body = serde_json::to_vec(&ErrorBody::new(code, message)).unwrap_or_default();
    versioned(UiResponse::new(
        code.http_status(),
        "application/json; charset=utf-8",
        body,
    ))
}

/// `/api/v1/devices/{id}` -> `id` (one non-empty segment, percent-decoded).
fn device_item(path: &str) -> Option<String> {
    Some(super::util::percent_decode(
        path.strip_prefix("/api/v1/devices/")?,
    ))
    .filter(|id| !id.is_empty() && !id.contains('/'))
}

/// `/api/v1/profiles/{id}` -> `id` (one non-empty segment, percent-decoded).
fn profile_item(path: &str) -> Option<String> {
    Some(super::util::percent_decode(
        path.strip_prefix("/api/v1/profiles/")?,
    ))
    .filter(|id| !id.is_empty() && !id.contains('/'))
}

/// `/api/v1/devices/{id}/{action}` -> (`id`, `action`); id percent-decoded.
fn device_action(path: &str) -> Option<(String, &str)> {
    let rest = path.strip_prefix("/api/v1/devices/")?;
    let (id, action) = rest.split_once('/')?;
    (!id.is_empty() && !action.contains('/')).then_some((super::util::percent_decode(id), action))
}

/// `/api/v1/transfers/{id}/{action}` -> (`id`, `action`).
fn transfer_action(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/v1/transfers/")?;
    let (id, action) = rest.split_once('/')?;
    (!id.is_empty() && !action.contains('/')).then_some((id, action))
}

/// `/api/v1/inbox/{name}` -> name (single segment, may contain dots;
/// percent-decoded).
fn inbox_item(path: &str) -> Option<String> {
    Some(super::util::percent_decode(
        path.strip_prefix("/api/v1/inbox/")?,
    ))
    .filter(|n| !n.is_empty() && !n.contains('/'))
}

/// `/api/v1/inbox/{name}/open` -> name (percent-decoded).
fn inbox_open_action(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/v1/inbox/")?;
    let (name, action) = rest.split_once('/')?;
    (action == "open" && !name.is_empty()).then(|| super::util::percent_decode(name))
}

/// §162b: `/api/v1/outbox/{id}/resend` -> id (percent-decoded, single segment).
fn outbox_resend_action(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/v1/outbox/")?;
    let (id, action) = rest.split_once('/')?;
    (action == "resend" && !id.is_empty()).then(|| super::util::percent_decode(id))
}

/// §164: `/api/v1/action/local/{id}` -> id (percent-decoded) — the
/// remove-local-action route (legacy `DELETE /api/action/local?id=`).
fn action_local_item(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/v1/action/local/")?;
    (!rest.is_empty() && !rest.contains('/')).then(|| super::util::percent_decode(rest))
}

/// §162b: `/api/v1/inbox/{name}/content` -> name (percent-decoded) — the raw
/// file-bytes download route (legacy `/api/file/inbox/download`).
fn inbox_content_item(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/api/v1/inbox/")?;
    let (name, action) = rest.split_once('/')?;
    (action == "content" && !name.is_empty()).then(|| super::util::percent_decode(name))
}

/// Methods each known v1 path answers; drives 404 vs 405.
fn allowed(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/v1/health"
        | "/api/v1/system"
        | "/api/v1/events"
        | "/api/v1/devices"
        | "/api/v1/pairing"
        | "/api/v1/layout"
        | "/api/v1/activity"
        | "/api/v1/diagnostics"
        | "/api/v1/diagnostics/sys"
        | "/api/v1/diagnostics/logs" => Some(&["GET"]),
        "/api/v1/link" => Some(&["GET"]),
        "/api/v1/link/heartbeat" => Some(&["GET", "PATCH"]),
        "/api/v1/transfers" => Some(&["GET", "POST", "DELETE"]),
        "/api/v1/inbox" => Some(&["GET", "DELETE", "POST"]),
        p if inbox_content_item(p).is_some() => Some(&["GET"]),
        "/api/v1/settings" => Some(&["GET", "PATCH"]),
        "/api/v1/pairing/pin" => Some(&["GET", "DELETE"]),
        "/api/v1/pairing/pin/rotate" | "/api/v1/input/release" => Some(&["POST"]),
        "/api/v1/pairing/dial" => Some(&["POST"]),
        "/api/v1/diagnostics/log-level" => Some(&["PATCH"]),
        "/api/v1/diagnostics/metrics" => Some(&["GET", "PATCH"]),
        "/api/v1/profiles" => Some(&["GET"]),
        "/api/v1/caps" => Some(&["GET", "PATCH"]),
        "/api/v1/caps/peers" => Some(&["GET"]),
        "/api/v1/topology" => Some(&["GET"]),
        "/api/v1/topology/peers" => Some(&["GET"]),
        "/api/v1/topology/policy" => Some(&["GET", "PATCH"]),
        "/api/v1/topology/links" => Some(&["POST"]),
        "/api/v1/topology/unlinks" => Some(&["POST"]),
        "/api/v1/topology/sync" => Some(&["POST"]),
        "/api/v1/topology/config" => Some(&["PATCH"]),
        "/api/v1/routes" => Some(&["GET", "POST", "DELETE"]),
        "/api/v1/net" => Some(&["GET"]),
        "/api/v1/net/refresh" => Some(&["POST"]),
        // §160: discovery + daemon lifecycle (restart/shutdown are the
        // pre-classified Admin routes the tier table reserved).
        "/api/v1/discovery" => Some(&["GET", "POST"]),
        "/api/v1/system/restart" | "/api/v1/system/shutdown" => Some(&["POST"]),
        // §161: notify / reply / OTP + peer message domains.
        "/api/v1/notify" => Some(&["POST"]),
        "/api/v1/notify/candidates" | "/api/v1/notify/replies" => Some(&["GET"]),
        "/api/v1/notify/config" => Some(&["GET", "PATCH"]),
        "/api/v1/notify/dismissals" => Some(&["POST"]),
        "/api/v1/notify/otp" | "/api/v1/notify/open" | "/api/v1/notify/test" => Some(&["POST"]),
        "/api/v1/notify/copies" | "/api/v1/notify/insertions" => Some(&["POST"]),
        "/api/v1/replies" | "/api/v1/replies/take" => Some(&["POST"]),
        "/api/v1/messages" => Some(&["GET", "POST", "DELETE"]),
        // §162b: file domain — offers/decisions, outbox, storage, resume,
        // auto-accept policy (transfers/inbox gate above).
        "/api/v1/offers" => Some(&["GET"]),
        "/api/v1/offers/accept" | "/api/v1/offers/reject" => Some(&["POST"]),
        "/api/v1/transfers/multi" | "/api/v1/transfers/batch" => Some(&["POST"]),
        "/api/v1/outbox" => Some(&["GET", "DELETE"]),
        p if outbox_resend_action(p).is_some() => Some(&["POST"]),
        "/api/v1/storage" => Some(&["GET"]),
        "/api/v1/storage/clean" => Some(&["POST"]),
        "/api/v1/resume" => Some(&["GET"]),
        "/api/v1/policy/auto-accept" => Some(&["GET", "PATCH"]),
        // §163: clipboard sync + audit trail domains.
        "/api/v1/clip" => Some(&["GET", "DELETE"]),
        "/api/v1/clip/status" | "/api/v1/clip/history" => Some(&["GET"]),
        "/api/v1/clip/sync" | "/api/v1/clip/poll" => Some(&["POST"]),
        "/api/v1/clip/policy" => Some(&["GET", "PATCH"]),
        "/api/v1/audit" => Some(&["GET", "DELETE"]),
        // §164: remote action + link benchmark domains.
        "/api/v1/action/local" => Some(&["GET", "POST"]),
        p if action_local_item(p).is_some() => Some(&["DELETE"]),
        "/api/v1/action/remote" => Some(&["GET"]),
        "/api/v1/action/remote/refresh" | "/api/v1/action/trigger" => Some(&["POST"]),
        "/api/v1/action/history" => Some(&["GET"]),
        "/api/v1/action/policy" => Some(&["GET", "PATCH"]),
        "/api/v1/bench" => Some(&["GET"]),
        "/api/v1/bench/start" | "/api/v1/bench/stop" => Some(&["POST"]),
        "/api/v1/bench/policy" => Some(&["GET", "PATCH"]),
        "/api/v1/recommendation" => Some(&["GET"]),
        // §165: battery/power domain.
        "/api/v1/battery/state" => Some(&["GET", "PATCH"]),
        "/api/v1/battery/policy" => Some(&["GET", "PATCH"]),
        // §165: media control domain.
        "/api/v1/media/command" | "/api/v1/media/state/sync" => Some(&["POST", "PATCH"]),
        "/api/v1/media/state" => Some(&["GET", "PATCH"]),
        "/api/v1/media/policy" => Some(&["GET", "PATCH"]),
        // §165: ANCS (iPhone notification bridge) domain.
        "/api/v1/ancs" => Some(&["GET"]),
        "/api/v1/ancs/connect" | "/api/v1/ancs/disconnect" => Some(&["POST"]),
        "/api/v1/ancs/allow" => Some(&["POST", "DELETE"]),
        "/api/v1/ancs/rules/clear" => Some(&["POST"]),
        // §165: input tunnel domain.
        "/api/v1/input" => Some(&["GET"]),
        "/api/v1/input/trace" => Some(&["GET"]),
        "/api/v1/input/start" | "/api/v1/input/stop" => Some(&["POST"]),
        "/api/v1/input/tunnel" => Some(&["GET"]),
        "/api/v1/input/tunnel/attach" | "/api/v1/input/tunnel/release" => Some(&["POST"]),
        "/api/v1/input/policy" => Some(&["GET", "PATCH"]),
        // §165: handoff (URL/resource inbox) domain.
        "/api/v1/handoff" | "/api/v1/handoff/inbox" => Some(&["GET", "DELETE"]),
        "/api/v1/handoff/send" | "/api/v1/handoff/send/multi" => Some(&["POST"]),
        "/api/v1/handoff/inbox/open" | "/api/v1/handoff/inbox/dismiss" => Some(&["POST"]),
        "/api/v1/handoff/policy" => Some(&["GET", "PATCH"]),
        // §165: focus (DND / quiet hours) domain.
        "/api/v1/focus/state" => Some(&["GET", "POST"]),
        "/api/v1/focus/peers" => Some(&["GET"]),
        "/api/v1/focus/sync" => Some(&["POST"]),
        "/api/v1/focus/policy" => Some(&["GET", "PATCH"]),
        // §165: camera (virtual camera streaming) domain.
        "/api/v1/camera" => Some(&["GET"]),
        "/api/v1/camera/attach" | "/api/v1/camera/detach" => Some(&["POST"]),
        "/api/v1/camera/policy" => Some(&["GET", "PATCH"]),
        // §165: mic (virtual microphone / intercom) domain.
        "/api/v1/mic" => Some(&["GET"]),
        "/api/v1/mic/attach" | "/api/v1/mic/detach" => Some(&["POST"]),
        "/api/v1/mic/policy" => Some(&["GET", "PATCH"]),
        // §165: audio routing domain.
        "/api/v1/audio" => Some(&["GET"]),
        "/api/v1/audio/attach" | "/api/v1/audio/detach" => Some(&["POST"]),
        "/api/v1/audio/policy" => Some(&["GET", "PATCH"]),
        // §165: locate (find-my ringing) domain.
        "/api/v1/locate/status" => Some(&["GET"]),
        "/api/v1/locate/ring" | "/api/v1/locate/stop" => Some(&["POST"]),
        "/api/v1/locate/policy" => Some(&["GET", "PATCH"]),
        // §165: virtual display domain.
        "/api/v1/display/virtual" => Some(&["GET"]),
        "/api/v1/display/virtual/attach" | "/api/v1/display/virtual/detach" => Some(&["POST"]),
        "/api/v1/display/virtual/policy" => Some(&["GET", "PATCH"]),
        // §167: screen streaming domain.
        "/api/v1/screen" => Some(&["GET"]),
        "/api/v1/screen/allow"
        | "/api/v1/screen/revoke"
        | "/api/v1/screen/start"
        | "/api/v1/screen/stop"
        | "/api/v1/screen/keyframe" => Some(&["POST"]),
        "/api/v1/screen/displays" => Some(&["GET"]),
        "/api/v1/screen/switch" => Some(&["POST"]),
        "/api/v1/screen/privacy" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/recording" => Some(&["GET"]),
        "/api/v1/screen/recording/start" | "/api/v1/screen/recording/stop" => Some(&["POST"]),
        "/api/v1/screen/recording/replay" => Some(&["GET"]),
        "/api/v1/screen/recording/snapshot" => Some(&["GET", "POST"]),
        "/api/v1/screen/playback" => Some(&["GET"]),
        "/api/v1/screen/playback/start"
        | "/api/v1/screen/playback/pause"
        | "/api/v1/screen/playback/resume"
        | "/api/v1/screen/playback/stop" => Some(&["POST"]),
        "/api/v1/screen/snapshot" => Some(&["GET", "POST"]),
        "/api/v1/screen/laser" => Some(&["GET", "POST"]),
        "/api/v1/screen/annotation" => Some(&["GET", "POST", "DELETE"]),
        "/api/v1/screen/whiteboard/undo"
        | "/api/v1/screen/whiteboard/redo"
        | "/api/v1/screen/whiteboard/clear"
        | "/api/v1/screen/whiteboard/import" => Some(&["POST"]),
        "/api/v1/screen/whiteboard/export" => Some(&["GET", "POST"]),
        "/api/v1/screen/whiteboard/policy" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/watermark" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/watermark/enable" | "/api/v1/screen/watermark/disable" => Some(&["POST"]),
        "/api/v1/screen/probe/trigger"
        | "/api/v1/screen/sync/trigger"
        | "/api/v1/screen/svc/drop-t1"
        | "/api/v1/screen/ltr/anchor"
        | "/api/v1/screen/ltr/recover" => Some(&["POST"]),
        "/api/v1/screen/cursor" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/trendline" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/squelch" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/fec" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/pacer" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/harq" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/twcc" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/sync" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/latency" => Some(&["GET", "PATCH", "DELETE"]),
        "/api/v1/screen/tuning" => Some(&["GET", "PATCH", "DELETE"]),
        "/api/v1/screen/playout" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/matrix" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/bitrate"
        | "/api/v1/screen/fps"
        | "/api/v1/screen/scale"
        | "/api/v1/screen/profile" => Some(&["GET", "PATCH"]),
        "/api/v1/screen/color"
        | "/api/v1/screen/delta"
        | "/api/v1/screen/metric"
        | "/api/v1/screen/qoe"
        | "/api/v1/screen/pir"
        | "/api/v1/screen/motion"
        | "/api/v1/screen/roi" => Some(&["GET", "PATCH", "DELETE"]),
        "/api/v1/screen/focus" => Some(&["GET", "PATCH", "DELETE"]),
        "/api/v1/screen/tile"
        | "/api/v1/screen/tile/cache"
        | "/api/v1/screen/tile/palette"
        | "/api/v1/screen/tile/predict"
        | "/api/v1/screen/tile/present"
        | "/api/v1/screen/tile/nack"
        | "/api/v1/screen/tile/saliency"
        | "/api/v1/screen/tile/lod"
        | "/api/v1/screen/tile/batch"
        | "/api/v1/screen/tile/motion" => Some(&["GET", "PATCH", "DELETE"]),
        "/api/v1/screen/ltr" => Some(&["GET", "PATCH", "DELETE"]),
        "/api/v1/screen/svc" => Some(&["GET", "PATCH", "DELETE"]),
        "/api/v1/screen/probe" => Some(&["GET", "PATCH"]),

        p if profile_item(p).is_some() => Some(&["GET", "PATCH"]),
        p if matches!(device_action(p), Some((_, "caps"))) => Some(&["GET"]),
        p if device_item(p).is_some() => Some(&["GET", "PATCH", "DELETE"]),
        p => match device_action(p) {
            Some((_, "connect"))
            | Some((_, "disconnect"))
            | Some((_, "probe"))
            | Some((_, "ping")) => Some(&["POST"]),
            Some((_, "block")) => Some(&["POST", "DELETE"]),
            _ => match transfer_action(p) {
                Some((_, "cancel")) => Some(&["POST"]),
                _ => match inbox_open_action(p) {
                    Some(_) => Some(&["POST"]),
                    None => match inbox_item(p) {
                        Some(_) => Some(&["DELETE"]),
                        None => None,
                    },
                },
            },
        },
    }
}

fn no_content() -> UiResponse {
    versioned(UiResponse::new(
        204,
        "application/json; charset=utf-8",
        Vec::new(),
    ))
}

/// Strict JSON body for non-Default DTOs (all fields required).
fn parse_body_strict<T: DeserializeOwned>(body: &[u8]) -> Result<T, UiResponse> {
    serde_json::from_slice(body)
        .map_err(|e| error(ErrorCode::BadRequest, format!("invalid body: {e}")))
}

/// Strict JSON body; an empty body reads as `{}`.
fn parse_body<T: DeserializeOwned + Default>(body: &[u8]) -> Result<T, UiResponse> {
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(T::default());
    }
    serde_json::from_slice(body)
        .map_err(|e| error(ErrorCode::BadRequest, format!("invalid body: {e}")))
}

/// Map a node error onto the v1 envelope. Every variant the daemon can
/// surface to a v1 route gets its legacy-mirrored class; the catch-all
/// stays `Internal` so a genuinely unexpected fault is not masked as a
/// client problem.
fn core_error(e: CoreError) -> UiResponse {
    use CoreError as E;
    let code = match &e {
        E::PeerNotFound(_) | E::UnknownCandidate | E::UnknownTransfer => ErrorCode::NotFound,
        // Inbox lookups surface missing files as File("file not found…");
        // legacy /api/file/inbox maps the same shape to 404.
        E::File(ref msg) if msg.contains("not found") => ErrorCode::NotFound,
        E::InvalidAlias(_)
        | E::InvalidProfile(_)
        | E::InvalidRoute(_)
        | E::InvalidPayload(_)
        | E::InvalidConfig(_)
        | E::InvalidDeviceId
        | E::InvalidPin
        | E::InvalidPublicKey
        | E::UnsafeFileName
        | E::EmptyReply
        | E::ResumeOffset
        | E::InsertRefused(_)
        | E::OpenRefused(_)
        | E::FileNotAccepted
        | E::FileAlreadyDecided
        | E::InputFrame
        | E::InputSeqJump
        | E::InputRequiresTrust
        | E::AncsFrame
        | E::ScreenFrame
        | E::ScreenSeqJump
        | E::ScreenTooLarge
        | E::ScreenFragment => ErrorCode::BadRequest,
        // Caller-side state: the request is valid but cannot run now.
        E::NoConnectedPeer
        | E::PingTimeout
        | E::PeerBlocked(_)
        | E::StaticKeyChanged
        | E::Revoked => ErrorCode::Conflict,
        E::FileTooLarge(_) | E::PayloadTooLarge | E::HelloTooLarge | E::ClipboardTooLarge => {
            ErrorCode::PayloadTooLarge
        }
        E::NoTargetsMatched(_) | E::NoAncsTransport | E::NoOtp => ErrorCode::Unavailable,
        _ => ErrorCode::Internal,
    };
    error(code, e.to_string())
}

/// Resolve a path id (id, alias or unique prefix) to the canonical id.
fn resolve(node: &Node, id: &str) -> Result<String, UiResponse> {
    node.resolve_peer(id)
        .map(|d| d.to_string())
        .map_err(core_error)
}

fn device_json(node: &Node, id: &str) -> UiResponse {
    match crate::apiv1::model::device(node, id) {
        Some(d) => json(200, &d),
        None => error(ErrorCode::NotFound, format!("no such device: {id}")),
    }
}

pub(super) async fn route(
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
    node: &Node,
    hub: &Arc<EventHub>,
    granted: Option<crate::apitoken::Tier>,
) -> UiResponse {
    let result = match (method, path) {
        ("GET", "/api/v1/health") => Ok(json(200, &Health::new(env!("CARGO_PKG_VERSION")))),
        ("GET", "/api/v1/system") => Ok(json(200, &system(node))),
        ("GET", "/api/v1/devices") => Ok(json(200, &crate::apiv1::model::devices(node))),
        ("GET", "/api/v1/pairing") => Ok(json(200, &crate::apiv1::model::pairing(node))),
        ("GET", "/api/v1/pairing/pin") => Ok(get_pin(node)),
        ("POST", "/api/v1/pairing/pin/rotate") => rotate_pin(node),
        ("POST", "/api/v1/pairing/dial") => dial_peer(node, body).await,
        ("DELETE", "/api/v1/pairing/pin") => {
            node.clear_pin();
            Ok(no_content())
        }
        ("GET", "/api/v1/transfers") => Ok(json(200, &crate::apiv1::model::transfers(node))),
        ("GET", "/api/v1/inbox") => {
            // §162a: optional `search`/`limit` mirror the legacy inbox list
            // query (substring match, capped listing).
            let search = profile_filter(query, "search");
            let limit = match profile_filter(query, "limit") {
                Some(raw) => match raw.parse::<usize>() {
                    Ok(n) => Some(n),
                    Err(_) => {
                        return error(ErrorCode::BadRequest, format!("invalid limit value: {raw}"));
                    }
                },
                None => None,
            };
            Ok(json(
                200,
                &crate::apiv1::model::inbox(node, search.as_deref(), limit),
            ))
        }
        ("GET", "/api/v1/layout") => Ok(json(200, &crate::apiv1::model::layout(node))),
        ("GET", "/api/v1/settings") => Ok(json(200, &crate::apiv1::model::settings(node))),
        ("GET", "/api/v1/link") => Ok(json(200, &crate::apiv1::model::link_quality(node))),
        ("GET", "/api/v1/link/heartbeat") => Ok(json(200, &crate::apiv1::model::heartbeat(node))),
        ("PATCH", "/api/v1/link/heartbeat") => patch_heartbeat(node, body),
        ("GET", "/api/v1/activity") => Ok(json(200, &crate::apiv1::model::activity(node))),
        ("POST", "/api/v1/input/release") => match node.input_stop_reported().await {
            Ok(_) => Ok(json(200, &crate::apiv1::model::input(node).await)),
            Err(e) => Err(core_error(e)),
        },
        ("GET", "/api/v1/diagnostics") => Ok(json(
            200,
            &Diagnostics {
                event_seq: hub.current(),
                legacy_routes: hub.legacy.snapshot(),
            },
        )),
        ("GET", "/api/v1/diagnostics/sys") => match node.system_report() {
            Ok(rep) => Ok(json(
                200,
                &SysTelemetry {
                    ok: rep.ok,
                    node_name: rep.node_name,
                    device_id: rep.device_id,
                    process: SysProcess {
                        pid: rep.process.pid,
                        uptime_secs: rep.process.uptime_secs,
                        rss_bytes: rep.process.rss_bytes,
                        virt_bytes: rep.process.virt_bytes,
                        thread_count: rep.process.thread_count,
                        open_fds: rep.process.open_fds,
                        cpu_time_ms: rep.process.cpu_time_ms,
                        memory_rss_formatted: rep.process.memory_rss_formatted,
                        memory_virt_formatted: rep.process.memory_virt_formatted,
                    },
                    host: SysHost {
                        os: rep.host.os,
                        arch: rep.host.arch,
                        num_cpus: rep.host.num_cpus,
                        total_memory_bytes: rep.host.total_memory_bytes,
                        total_memory_formatted: rep.host.total_memory_formatted,
                    },
                    timestamp_ms: rep.timestamp_ms,
                },
            )),
            Err(e) => Err(core_error(e)),
        },
        ("GET", "/api/v1/diagnostics/logs") => log_tail(query).await,
        ("PATCH", "/api/v1/diagnostics/log-level") => log_level(body),
        ("GET", "/api/v1/diagnostics/metrics") => {
            Ok(json(200, &crate::apiv1::model::traffic(node)))
        }
        ("PATCH", "/api/v1/diagnostics/metrics") => {
            node.reset_traffic_metrics();
            Ok(json(200, &crate::apiv1::model::traffic(node)))
        }
        ("PATCH", "/api/v1/settings") => patch_settings(node, body),
        ("DELETE", "/api/v1/transfers") => {
            node.clear_terminal_transfers();
            Ok(json(200, &crate::apiv1::model::transfers(node)))
        }
        ("DELETE", "/api/v1/inbox") => {
            // §162a: optional `before=<unix secs>` mirrors the legacy
            // inbox/clear cutoff — files modified at/after the cutoff stay.
            let before = match profile_filter(query, "before") {
                Some(raw) => match raw.parse::<u64>() {
                    Ok(secs) => Some(secs),
                    Err(_) => {
                        return error(
                            ErrorCode::BadRequest,
                            format!("invalid before value: {raw}"),
                        );
                    }
                },
                None => None,
            };
            match node.clear_inbox(before) {
                Ok(_) => Ok(json(200, &crate::apiv1::model::inbox(node, None, None))),
                Err(e) => Err(core_error(e)),
            }
        }
        ("POST", "/api/v1/inbox") => {
            // §162a: name-less open — reveal the inbox directory itself (the
            // named-file variant lives at POST /inbox/{name}/open).
            open_inbox_dir(node, granted).await
        }
        ("GET", "/api/v1/offers") => {
            // §162b: pending offers resource with the legacy id/peer filters.
            let id = profile_filter(query, "id");
            let peer = profile_filter(query, "peer");
            match crate::apiv1::model::file_offers(node, id.as_deref(), peer.as_deref()) {
                Ok(rep) => Ok(json(200, &rep)),
                Err(e) => Err(core_error(e)),
            }
        }
        ("POST", "/api/v1/offers/accept") => post_file_decision(node, query, body, true).await,
        ("POST", "/api/v1/offers/reject") => post_file_decision(node, query, body, false).await,
        ("POST", "/api/v1/transfers/multi") => post_file_send_multi(node, body).await,
        ("POST", "/api/v1/transfers/batch") => post_file_send_batch(node, body).await,
        ("POST", "/api/v1/transfers") => post_file_send(node, body).await,
        ("GET", "/api/v1/outbox") => {
            // §162b: outbox resource with the legacy peer/status/limit query.
            let peer = profile_filter(query, "peer");
            let status = match profile_filter(query, "status") {
                Some(raw) => match raw.parse::<crate::runtime::OutboxStatus>() {
                    Ok(st) => Some(st),
                    Err(e) => {
                        return error(ErrorCode::BadRequest, e);
                    }
                },
                None => None,
            };
            let limit = match profile_filter(query, "limit") {
                Some(raw) => match raw.parse::<usize>() {
                    Ok(n) => Some(n),
                    Err(_) => {
                        return error(ErrorCode::BadRequest, format!("invalid limit value: {raw}"));
                    }
                },
                None => None,
            };
            match crate::apiv1::model::outbox(node, peer.as_deref(), status, limit) {
                Ok(rep) => Ok(json(200, &rep)),
                Err(e) => Err(core_error(e)),
            }
        }
        ("DELETE", "/api/v1/outbox") => {
            // §162b: purge outbox records, optionally scoped to peer/status.
            let peer = profile_filter(query, "peer");
            let status = match profile_filter(query, "status") {
                Some(raw) => match raw.parse::<crate::runtime::OutboxStatus>() {
                    Ok(st) => Some(st),
                    Err(e) => {
                        return error(ErrorCode::BadRequest, e);
                    }
                },
                None => None,
            };
            match node.clear_outbox(peer.as_deref(), status) {
                Ok(rep) => Ok(json(200, &crate::apiv1::model::outbox_clear(rep))),
                Err(e) => Err(core_error(e)),
            }
        }
        ("POST", p) if outbox_resend_action(p).is_some() => {
            post_outbox_resend(node, &outbox_resend_action(p).unwrap_or_default()).await
        }
        ("GET", "/api/v1/storage") => {
            let mut rep = crate::apiv1::model::storage(node);
            if granted < Some(crate::apitoken::Tier::Admin) {
                // P3: absolute filesystem layout is Admin-only; lower tiers
                // get stable placeholders instead of the daemon's data dir.
                rep.data_dir = "<data-dir>".to_string();
                rep.inbox_dir = "<data-dir>/inbox".to_string();
            }
            Ok(json(200, &rep))
        }
        ("POST", "/api/v1/storage/clean") => {
            // §162b: `orphaned_only` defaults to true (legacy parity); an
            // explicit false also sweeps in-flight partials.
            let orphaned_only = if body.iter().all(u8::is_ascii_whitespace) {
                true
            } else {
                match serde_json::from_slice::<tetherly_api::StorageCleanBody>(body) {
                    Ok(d) => d.orphaned_only.unwrap_or(true),
                    Err(e) => {
                        return error(ErrorCode::BadRequest, format!("invalid body: {e}"));
                    }
                }
            };
            Ok(json(
                200,
                &crate::apiv1::model::storage_clean(node.clean_storage(orphaned_only)),
            ))
        }
        ("GET", "/api/v1/resume") => Ok(json(200, &crate::apiv1::model::resume_markers(node))),
        ("GET", "/api/v1/policy/auto-accept") => {
            Ok(json(200, &crate::apiv1::model::auto_accept_policy(node)))
        }
        ("PATCH", "/api/v1/policy/auto-accept") => patch_auto_accept_policy(node, body),
        // §163: clipboard sync domain. `GET /clip` is the one plaintext
        // readback; the rest are metadata-only (zero-plaintext rule).
        ("GET", "/api/v1/clip") => Ok(json(200, &crate::apiv1::model::clip_content(node))),
        ("DELETE", "/api/v1/clip") => match crate::apiv1::model::clip_clear(node) {
            Ok(rep) => Ok(json(200, &rep)),
            Err(e) => Err(core_error(e)),
        },
        ("GET", "/api/v1/clip/status") => match crate::apiv1::model::clip_status(node) {
            Ok(rep) => Ok(json(200, &rep)),
            Err(e) => Err(core_error(e)),
        },
        ("GET", "/api/v1/clip/history") => {
            let limit = query.and_then(|q| {
                profile_filter(Some(q), "limit").and_then(|s| s.parse::<usize>().ok())
            });
            Ok(json(200, &crate::apiv1::model::clip_history(node, limit)))
        }
        ("POST", "/api/v1/clip/sync") => post_clip_sync(node, body).await,
        ("POST", "/api/v1/clip/poll") => match node.poll_clipboard_auto_sync().await {
            Ok(count) => Ok(json(
                200,
                &crate::apiv1::model::clip_poll(node, count as u64),
            )),
            Err(e) => Err(core_error(e)),
        },
        ("GET", "/api/v1/clip/policy") => Ok(json(200, &crate::apiv1::model::clip_policy(node))),
        ("PATCH", "/api/v1/clip/policy") => patch_clip_policy(node, query, body).await,
        // §163: audit trail domain (clear reuses the DELETE verb).
        ("GET", "/api/v1/audit") => get_audit_v1(node, query, body),
        ("DELETE", "/api/v1/audit") => Ok(json(200, &crate::apiv1::model::audit_clear(node))),
        // §164: remote action domain.
        ("GET", "/api/v1/action/local") => Ok(json(200, &crate::apiv1::model::local_actions(node))),
        ("POST", "/api/v1/action/local") => post_action_local(node, body).await,
        ("DELETE", p) if action_local_item(p).is_some() => {
            let id = action_local_item(p).unwrap_or_default();
            match crate::apiv1::model::local_action_remove(node, &id) {
                Ok(rep) => Ok(json(200, &rep)),
                Err(e) => Err(core_error(e)),
            }
        }
        ("GET", "/api/v1/action/remote") => {
            match query.and_then(|q| super::parse_query_map(q).get("peer").cloned()) {
                Some(peer) => match crate::apiv1::model::remote_actions(node, &peer) {
                    Ok(rep) => Ok(json(200, &rep)),
                    Err(e) => Err(core_error(e)),
                },
                None => Err(error(ErrorCode::BadRequest, "missing peer")),
            }
        }
        ("POST", "/api/v1/action/remote/refresh") => {
            let req = parse_body::<RemoteActionRefreshRequest>(body);
            let peer = match req
                .ok()
                .and_then(|r| r.peer)
                .or_else(|| query.and_then(|q| super::parse_query_map(q).get("peer").cloned()))
            {
                Some(p) if !p.trim().is_empty() => p,
                _ => return error(ErrorCode::BadRequest, "missing peer"),
            };
            match node.query_remote_actions(&peer).await {
                Ok(rep) => Ok(json(200, &crate::apiv1::model::remote_actions_report(rep))),
                Err(e) => Err(core_error(e)),
            }
        }
        ("POST", "/api/v1/action/trigger") => {
            let req: ActionTriggerRequest = if body.iter().all(u8::is_ascii_whitespace) {
                let q = query.map(super::parse_query_map).unwrap_or_default();
                let action_id = q.get("action_id").cloned().or_else(|| q.get("id").cloned());
                match (q.get("peer").cloned(), action_id) {
                    (Some(peer), Some(action_id)) => ActionTriggerRequest { peer, action_id },
                    _ => return error(ErrorCode::BadRequest, "missing peer/action_id"),
                }
            } else {
                match parse_body_strict::<ActionTriggerRequest>(body) {
                    Ok(r) => r,
                    Err(resp) => return resp,
                }
            };
            match crate::apiv1::model::action_trigger(node, &req.peer, &req.action_id).await {
                Ok(rep) => Ok(json(200, &rep)),
                Err(e) => Err(core_error(e)),
            }
        }
        ("GET", "/api/v1/action/history") => {
            Ok(json(200, &crate::apiv1::model::action_history(node)))
        }
        ("GET", "/api/v1/action/policy") => {
            Ok(json(200, &crate::apiv1::model::action_policy(node)))
        }
        ("PATCH", "/api/v1/action/policy") => patch_action_policy(node, query, body).await,
        // §164: link benchmark + recommendation domain.
        ("GET", "/api/v1/bench") => Ok(json(200, &crate::apiv1::model::bench_status(node))),
        ("POST", "/api/v1/bench/start") => post_bench_start(node, query, body).await,
        ("POST", "/api/v1/bench/stop") => match node.bench_stop().await {
            Ok(rep) => Ok(json(200, &crate::apiv1::model::bench_decision(rep))),
            Err(e) => Err(core_error(e)),
        },
        ("GET", "/api/v1/bench/policy") => Ok(json(200, &crate::apiv1::model::bench_policy(node))),
        ("PATCH", "/api/v1/bench/policy") => patch_bench_policy(node, query, body).await,
        ("GET", "/api/v1/recommendation") => {
            let peer = query.and_then(|q| super::parse_query_map(q).get("peer").cloned());
            match peer {
                Some(p) => match crate::apiv1::model::recommendation(node, &p) {
                    Ok(rep) => Ok(json(200, &rep)),
                    Err(e) => Err(core_error(e)),
                },
                None => Ok(json(200, &crate::apiv1::model::recommendation_status(node))),
            }
        }
        // §165: battery/power domain.
        ("GET", "/api/v1/battery/state") => get_battery_state(node, query).await,
        ("PATCH", "/api/v1/battery/state") => patch_battery_state(node, query, body).await,
        ("GET", "/api/v1/battery/policy") => {
            Ok(json(200, &crate::apiv1::model::battery_policy(node)))
        }
        ("PATCH", "/api/v1/battery/policy") => patch_battery_policy(node, query, body).await,
        // §165: media control domain.
        ("POST", "/api/v1/media/command") => post_media_command(node, query, body).await,
        ("GET", "/api/v1/media/state") => get_media_state(node, query),
        ("PATCH", "/api/v1/media/state") => patch_media_state(node, body),
        ("GET", "/api/v1/media/policy") => Ok(json(200, &crate::apiv1::model::media_policy(node))),
        ("PATCH", "/api/v1/media/policy") => patch_media_policy(node, query, body).await,
        // §165: ANCS (iPhone notification bridge) domain.
        ("GET", "/api/v1/ancs") => Ok(json(200, &crate::apiv1::model::ancs_status(node))),
        ("POST", "/api/v1/ancs/connect") => post_ancs_connect(node).await,
        ("POST", "/api/v1/ancs/disconnect") => {
            Ok(json(200, &crate::apiv1::model::ancs_disconnect(node)))
        }
        ("POST", "/api/v1/ancs/allow") => post_ancs_allow(node, body).await,
        ("DELETE", "/api/v1/ancs/allow") => delete_ancs_allow(node, query, body).await,
        ("POST", "/api/v1/ancs/rules/clear") => {
            Ok(json(200, &crate::apiv1::model::ancs_clear_rules(node)))
        }
        // §165: input tunnel domain.
        ("GET", "/api/v1/input") => get_input_status(node).await,
        ("GET", "/api/v1/input/trace") => get_input_trace(node).await,
        ("POST", "/api/v1/input/start") => post_input_start(node, body).await,
        ("POST", "/api/v1/input/stop") => post_input_stop(node).await,
        ("GET", "/api/v1/input/tunnel") => get_input_tunnel(node).await,
        ("POST", "/api/v1/input/tunnel/attach") => post_input_attach(node, query, body).await,
        ("POST", "/api/v1/input/tunnel/release") => post_input_release(node, query, body).await,
        ("GET", "/api/v1/input/policy") => Ok(json(200, &crate::apiv1::model::input_policy(node))),
        ("PATCH", "/api/v1/input/policy") => patch_input_policy(node, query, body).await,
        // §165: handoff (URL/resource inbox) domain.
        ("POST", "/api/v1/handoff/send") => post_handoff_send(node, query, body).await,
        ("POST", "/api/v1/handoff/send/multi") => post_handoff_send_multi(node, query, body).await,
        ("GET", "/api/v1/handoff/inbox") => get_handoff_inbox(node, query),
        ("POST", "/api/v1/handoff/inbox/open") => post_handoff_inbox_open(node, query, body).await,
        ("POST", "/api/v1/handoff/inbox/dismiss") => {
            post_handoff_inbox_dismiss(node, query, body).await
        }
        ("DELETE", "/api/v1/handoff/inbox") => delete_handoff_inbox(node).await,
        ("GET", "/api/v1/handoff/policy") => {
            Ok(json(200, &crate::apiv1::model::handoff_policy(node)))
        }
        ("PATCH", "/api/v1/handoff/policy") => patch_handoff_policy(node, query, body).await,
        // §165: focus (DND / quiet hours) domain.
        ("GET", "/api/v1/focus/state") => Ok(json(200, &crate::apiv1::model::focus_state(node))),
        ("POST", "/api/v1/focus/state") => post_focus_state(node, query, body).await,
        ("GET", "/api/v1/focus/peers") => Ok(json(200, &crate::apiv1::model::focus_peers(node))),
        ("POST", "/api/v1/focus/sync") => post_focus_sync(node).await,
        ("GET", "/api/v1/focus/policy") => Ok(json(200, &crate::apiv1::model::focus_policy(node))),
        ("PATCH", "/api/v1/focus/policy") => patch_focus_policy(node, query, body).await,
        // §165: camera (virtual camera streaming) domain.
        ("GET", "/api/v1/camera") => Ok(json(200, &crate::apiv1::model::camera_status(node))),
        ("POST", "/api/v1/camera/attach") => post_camera_attach(node, query, body).await,
        ("POST", "/api/v1/camera/detach") => post_camera_detach(node, query, body).await,
        ("GET", "/api/v1/camera/policy") => {
            Ok(json(200, &crate::apiv1::model::camera_policy(node)))
        }
        ("PATCH", "/api/v1/camera/policy") => patch_camera_policy(node, query, body).await,
        // §165: mic (virtual microphone / intercom) domain.
        ("GET", "/api/v1/mic") => Ok(json(200, &crate::apiv1::model::mic_status(node))),
        ("POST", "/api/v1/mic/attach") => post_mic_attach(node, query, body).await,
        ("POST", "/api/v1/mic/detach") => post_mic_detach(node, query, body).await,
        ("GET", "/api/v1/mic/policy") => Ok(json(200, &crate::apiv1::model::mic_policy(node))),
        ("PATCH", "/api/v1/mic/policy") => patch_mic_policy(node, query, body).await,
        // §165: audio routing domain.
        ("GET", "/api/v1/audio") => Ok(json(200, &crate::apiv1::model::audio_status(node))),
        ("POST", "/api/v1/audio/attach") => post_audio_attach(node, query, body).await,
        ("POST", "/api/v1/audio/detach") => post_audio_detach(node, query, body).await,
        ("GET", "/api/v1/audio/policy") => Ok(json(200, &crate::apiv1::model::audio_policy(node))),
        ("PATCH", "/api/v1/audio/policy") => patch_audio_policy(node, query, body).await,
        // §165: locate (find-my ringing) domain.
        ("GET", "/api/v1/locate/status") => {
            Ok(json(200, &crate::apiv1::model::locate_status(node)))
        }
        ("POST", "/api/v1/locate/ring") => post_locate_ring(node, query, body).await,
        ("POST", "/api/v1/locate/stop") => post_locate_stop(node, query, body).await,
        ("GET", "/api/v1/locate/policy") => {
            Ok(json(200, &crate::apiv1::model::locate_policy(node)))
        }
        ("PATCH", "/api/v1/locate/policy") => patch_locate_policy(node, query, body).await,
        // §165: virtual display domain.
        ("GET", "/api/v1/display/virtual") => {
            Ok(json(200, &crate::apiv1::model::display_status(node)))
        }
        ("POST", "/api/v1/display/virtual/attach") => post_display_attach(node, query, body).await,
        ("POST", "/api/v1/display/virtual/detach") => post_display_detach(node, query, body).await,
        ("GET", "/api/v1/display/virtual/policy") => {
            Ok(json(200, &crate::apiv1::model::display_policy(node)))
        }
        ("PATCH", "/api/v1/display/virtual/policy") => {
            patch_display_policy(node, query, body).await
        }
        // §167: screen streaming domain.
        ("GET", "/api/v1/screen") => get_screen_status(node).await,
        ("POST", "/api/v1/screen/allow") => Ok(json(
            200,
            &crate::apiv1::model::screen_control_of(node.screen_allow_reported()),
        )),
        ("POST", "/api/v1/screen/revoke") => Ok(json(
            200,
            &crate::apiv1::model::screen_control_of(node.screen_revoke_reported()),
        )),
        ("POST", "/api/v1/screen/start") => post_screen_start(node, body).await,
        ("POST", "/api/v1/screen/stop") => match node.screen_stop_reported().await {
            Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_control_of(rep))),
            Err(e) => Err(core_error(e)),
        },
        ("POST", "/api/v1/screen/keyframe") => {
            match node.screen_keyframe_request_reported().await {
                Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_control_of(rep))),
                Err(e) => Err(core_error(e)),
            }
        }
        ("GET", "/api/v1/screen/displays") => Ok(json(
            200,
            &crate::apiv1::model::screen_displays_of(node.screen_display_report()),
        )),
        ("POST", "/api/v1/screen/switch") => post_screen_switch(node, body).await,
        ("GET", "/api/v1/screen/privacy") => Ok(json(
            200,
            &crate::apiv1::model::screen_privacy_of(node.screen_privacy_report()),
        )),
        ("PATCH", "/api/v1/screen/privacy") => patch_screen_privacy(node, body).await,
        ("GET", "/api/v1/screen/recording") => Ok(json(
            200,
            &crate::apiv1::model::screen_recording_of(node.screen_recording_report()),
        )),
        ("POST", "/api/v1/screen/recording/start") => post_screen_recording_start(node, body).await,
        ("POST", "/api/v1/screen/recording/stop") => match node.screen_stop_recording().await {
            Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_recording_of(rep))),
            Err(e) => Err(core_error(e)),
        },
        ("GET", "/api/v1/screen/recording/replay") => {
            get_screen_recording_replay(node, query).await
        }
        ("GET", "/api/v1/screen/recording/snapshot") => {
            screen_recording_snapshot(node, query, body, false).await
        }
        ("POST", "/api/v1/screen/recording/snapshot") => {
            screen_recording_snapshot(node, query, body, true).await
        }
        ("GET", "/api/v1/screen/playback") => Ok(json(
            200,
            &crate::apiv1::model::screen_playback_of(node.screen_playback_report()),
        )),
        ("POST", "/api/v1/screen/playback/start") => post_screen_playback_start(node, body).await,
        ("POST", "/api/v1/screen/playback/pause") => match node.screen_playback_pause() {
            Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_playback_of(rep))),
            Err(e) => Err(core_error(e)),
        },
        ("POST", "/api/v1/screen/playback/resume") => match node.screen_playback_resume() {
            Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_playback_of(rep))),
            Err(e) => Err(core_error(e)),
        },
        ("POST", "/api/v1/screen/playback/stop") => match node.screen_playback_stop() {
            Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_playback_of(rep))),
            Err(e) => Err(core_error(e)),
        },
        ("GET", "/api/v1/screen/snapshot") => get_screen_snapshot(node, query).await,
        ("POST", "/api/v1/screen/snapshot") => post_screen_snapshot(node, body).await,
        ("GET", "/api/v1/screen/laser") => Ok(json(200, &screen_laser_report_of(node))),
        ("POST", "/api/v1/screen/laser") => post_screen_laser(node, body).await,
        ("GET", "/api/v1/screen/annotation") => Ok(json(
            200,
            &crate::apiv1::model::screen_annotations_of(node.screen_annotation_report()),
        )),
        ("POST", "/api/v1/screen/annotation") => post_screen_annotation(node, body).await,
        ("DELETE", "/api/v1/screen/annotation") => match node.screen_clear_annotations().await {
            Ok(()) => Ok(json(
                200,
                &crate::apiv1::model::screen_annotations_of(node.screen_annotation_report()),
            )),
            Err(e) => Err(core_error(e)),
        },
        ("POST", "/api/v1/screen/whiteboard/undo") => post_screen_whiteboard_undo(node, body).await,
        ("POST", "/api/v1/screen/whiteboard/redo") => post_screen_whiteboard_redo(node, body).await,
        ("POST", "/api/v1/screen/whiteboard/clear") => {
            match node.screen_clear_annotations().await {
                Ok(()) => Ok(json(
                    200,
                    &crate::apiv1::model::screen_annotations_of(node.screen_annotation_report()),
                )),
                Err(e) => Err(core_error(e)),
            }
        }
        ("POST", "/api/v1/screen/whiteboard/import") => {
            post_screen_whiteboard_import(node, body).await
        }
        ("GET", "/api/v1/screen/whiteboard/export") => {
            get_screen_whiteboard_export(node, query).await
        }
        ("POST", "/api/v1/screen/whiteboard/export") => {
            post_screen_whiteboard_export(node, body).await
        }
        ("GET", "/api/v1/screen/whiteboard/policy") => Ok(json(
            200,
            &ScreenWhiteboardPolicyView {
                policy: crate::apiv1::model::whiteboard_policy_dto_of(
                    node.screen_whiteboard_get_policy(),
                ),
            },
        )),
        ("PATCH", "/api/v1/screen/whiteboard/policy") => {
            patch_screen_whiteboard_policy(node, body).await
        }
        ("GET", "/api/v1/screen/watermark") => Ok(json(
            200,
            &crate::apiv1::model::screen_watermark_of(node.screen_watermark_report()),
        )),
        ("PATCH", "/api/v1/screen/watermark") => patch_screen_watermark(node, body).await,
        ("POST", "/api/v1/screen/watermark/enable") => {
            post_screen_watermark_enable(node, body).await
        }
        ("POST", "/api/v1/screen/watermark/disable") => match node.screen_disable_watermark().await
        {
            Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_watermark_of(rep))),
            Err(e) => Err(core_error(e)),
        },
        ("POST", "/api/v1/screen/probe/trigger") => post_screen_probe_trigger(node, body).await,
        ("POST", "/api/v1/screen/sync/trigger") => post_screen_sync_trigger(node).await,
        ("POST", "/api/v1/screen/svc/drop-t1") => post_screen_svc_drop_t1(node, body).await,
        ("POST", "/api/v1/screen/ltr/anchor") => match node.screen_anchor_ltr().await {
            Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_ltr_of(rep))),
            Err(e) => Err(core_error(e)),
        },
        ("POST", "/api/v1/screen/ltr/recover") => match node.screen_request_ltr_recovery().await {
            Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_ltr_of(rep))),
            Err(e) => Err(core_error(e)),
        },
        ("GET", "/api/v1/screen/cursor") => Ok(json(
            200,
            &crate::apiv1::model::screen_cursor_of(node.screen_cursor_report()),
        )),
        ("PATCH", "/api/v1/screen/cursor") => patch_screen_cursor(node, body).await,
        ("GET", "/api/v1/screen/trendline") => Ok(json(
            200,
            &crate::apiv1::model::screen_trendline_of(node.screen_trendline_report()),
        )),
        ("PATCH", "/api/v1/screen/trendline") => patch_screen_trendline(node, body).await,
        ("GET", "/api/v1/screen/squelch") => Ok(json(
            200,
            &crate::apiv1::model::screen_squelch_of(node.screen_squelch_report()),
        )),
        ("PATCH", "/api/v1/screen/squelch") => patch_screen_squelch(node, body).await,
        ("GET", "/api/v1/screen/fec") => Ok(json(
            200,
            &crate::apiv1::model::screen_fec_of(node.screen_fec_report()),
        )),
        ("PATCH", "/api/v1/screen/fec") => patch_screen_fec(node, body).await,
        ("GET", "/api/v1/screen/pacer") => Ok(json(
            200,
            &crate::apiv1::model::screen_pacer_of(node.screen_pacer_report()),
        )),
        ("PATCH", "/api/v1/screen/pacer") => patch_screen_pacer(node, body).await,
        ("GET", "/api/v1/screen/harq") => Ok(json(
            200,
            &crate::apiv1::model::screen_harq_of(node.screen_harq_report()),
        )),
        ("PATCH", "/api/v1/screen/harq") => patch_screen_harq(node, body).await,
        ("GET", "/api/v1/screen/twcc") => Ok(json(
            200,
            &crate::apiv1::model::screen_twcc_of(node.screen_twcc_report()),
        )),
        ("PATCH", "/api/v1/screen/twcc") => patch_screen_twcc(node, body).await,
        ("GET", "/api/v1/screen/sync") => Ok(json(
            200,
            &crate::apiv1::model::screen_sync_of(node.screen_sync_report()),
        )),
        ("PATCH", "/api/v1/screen/sync") => patch_screen_sync(node, body).await,
        ("GET", "/api/v1/screen/latency") => Ok(json(
            200,
            &crate::apiv1::model::screen_latency_of(node.screen_latency_report()),
        )),
        ("PATCH", "/api/v1/screen/latency") => patch_screen_latency(node, body).await,
        ("DELETE", "/api/v1/screen/latency") => delete_screen_latency(node).await,
        ("GET", "/api/v1/screen/tuning") => Ok(json(
            200,
            &crate::apiv1::model::screen_tuning_of(node.screen_tuning_report()),
        )),
        ("PATCH", "/api/v1/screen/tuning") => patch_screen_tuning(node, body).await,
        ("DELETE", "/api/v1/screen/tuning") => delete_screen_tuning(node).await,
        ("GET", "/api/v1/screen/playout") => Ok(json(
            200,
            &crate::apiv1::model::screen_playout_of(node.screen_playout_report()),
        )),
        ("PATCH", "/api/v1/screen/playout") => patch_screen_playout(node, body).await,
        ("GET", "/api/v1/screen/matrix") => Ok(json(
            200,
            &crate::apiv1::model::screen_matrix_of(node.screen_matrix_report()),
        )),
        ("PATCH", "/api/v1/screen/matrix") => patch_screen_matrix(node, body).await,
        ("GET", "/api/v1/screen/bitrate") => Ok(json(
            200,
            &crate::apiv1::model::screen_bitrate_of(node.screen_bitrate_report()),
        )),
        ("PATCH", "/api/v1/screen/bitrate") => patch_screen_bitrate(node, body).await,
        ("GET", "/api/v1/screen/fps") => Ok(json(
            200,
            &crate::apiv1::model::screen_fps_of(node.screen_fps_report()),
        )),
        ("PATCH", "/api/v1/screen/fps") => patch_screen_fps(node, body).await,
        ("GET", "/api/v1/screen/scale") => Ok(json(
            200,
            &crate::apiv1::model::screen_scale_of(node.screen_scale_report()),
        )),
        ("PATCH", "/api/v1/screen/scale") => patch_screen_scale(node, body).await,
        ("GET", "/api/v1/screen/profile") => Ok(json(
            200,
            &crate::apiv1::model::screen_profile_of(node.screen_profile_report()),
        )),
        ("PATCH", "/api/v1/screen/profile") => patch_screen_profile(node, body).await,
        ("GET", "/api/v1/screen/color") => Ok(json(
            200,
            &crate::apiv1::model::screen_color_of(node.screen_color_report()),
        )),
        ("PATCH", "/api/v1/screen/color") => patch_screen_color(node, body).await,
        ("DELETE", "/api/v1/screen/color") => delete_screen_color(node).await,
        ("GET", "/api/v1/screen/delta") => Ok(json(
            200,
            &crate::apiv1::model::screen_delta_of(node.screen_delta_report()),
        )),
        ("PATCH", "/api/v1/screen/delta") => patch_screen_delta(node, body).await,
        ("DELETE", "/api/v1/screen/delta") => delete_screen_delta(node).await,
        ("GET", "/api/v1/screen/metric") => Ok(json(
            200,
            &crate::apiv1::model::screen_quality_metric_of(node.screen_quality_metric_report()),
        )),
        ("PATCH", "/api/v1/screen/metric") => patch_screen_metric(node, body).await,
        ("DELETE", "/api/v1/screen/metric") => delete_screen_metric(node).await,
        ("GET", "/api/v1/screen/qoe") => Ok(json(
            200,
            &crate::apiv1::model::screen_qoe_of(node.screen_qoe_report()),
        )),
        ("PATCH", "/api/v1/screen/qoe") => patch_screen_qoe(node, body).await,
        ("DELETE", "/api/v1/screen/qoe") => delete_screen_qoe(node).await,
        ("GET", "/api/v1/screen/pir") => Ok(json(
            200,
            &crate::apiv1::model::screen_pir_of(node.screen_pir_report()),
        )),
        ("PATCH", "/api/v1/screen/pir") => patch_screen_pir(node, body).await,
        ("DELETE", "/api/v1/screen/pir") => delete_screen_pir(node).await,
        ("GET", "/api/v1/screen/motion") => Ok(json(
            200,
            &crate::apiv1::model::screen_motion_of(node.screen_motion_report()),
        )),
        ("PATCH", "/api/v1/screen/motion") => patch_screen_motion(node, body).await,
        ("DELETE", "/api/v1/screen/motion") => delete_screen_motion(node).await,
        ("GET", "/api/v1/screen/roi") => Ok(json(
            200,
            &crate::apiv1::model::screen_roi_of(node.screen_roi_report()),
        )),
        ("PATCH", "/api/v1/screen/roi") => patch_screen_roi(node, body).await,
        ("DELETE", "/api/v1/screen/roi") => delete_screen_roi(node).await,
        ("GET", "/api/v1/screen/focus") => Ok(json(
            200,
            &crate::apiv1::model::screen_focus_of(node.screen_focus_report()),
        )),
        ("PATCH", "/api/v1/screen/focus") => patch_screen_focus(node, body).await,
        ("DELETE", "/api/v1/screen/focus") => delete_screen_focus(node).await,
        ("GET", "/api/v1/screen/tile") => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_of(node.screen_tile_report()),
        )),
        ("PATCH", "/api/v1/screen/tile") => patch_screen_tile(node, body).await,
        ("DELETE", "/api/v1/screen/tile") => delete_screen_tile(node).await,
        ("GET", "/api/v1/screen/tile/cache") => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_cache_of(node.screen_tile_cache_report()),
        )),
        ("PATCH", "/api/v1/screen/tile/cache") => patch_screen_tile_cache(node, body).await,
        ("DELETE", "/api/v1/screen/tile/cache") => delete_screen_tile_cache(node).await,
        ("GET", "/api/v1/screen/tile/palette") => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_palette_of(node.screen_tile_palette_report()),
        )),
        ("PATCH", "/api/v1/screen/tile/palette") => patch_screen_tile_palette(node, body).await,
        ("DELETE", "/api/v1/screen/tile/palette") => delete_screen_tile_palette(node).await,
        ("GET", "/api/v1/screen/tile/predict") => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_predict_of(node.screen_tile_predict_report()),
        )),
        ("PATCH", "/api/v1/screen/tile/predict") => patch_screen_tile_predict(node, body).await,
        ("DELETE", "/api/v1/screen/tile/predict") => delete_screen_tile_predict(node).await,
        ("GET", "/api/v1/screen/tile/present") => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_present_of(node.screen_tile_present_report()),
        )),
        ("PATCH", "/api/v1/screen/tile/present") => patch_screen_tile_present(node, body).await,
        ("DELETE", "/api/v1/screen/tile/present") => delete_screen_tile_present(node).await,
        ("GET", "/api/v1/screen/tile/nack") => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_nack_of(node.screen_tile_nack_report()),
        )),
        ("PATCH", "/api/v1/screen/tile/nack") => patch_screen_tile_nack(node, body).await,
        ("DELETE", "/api/v1/screen/tile/nack") => delete_screen_tile_nack(node).await,
        ("GET", "/api/v1/screen/tile/saliency") => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_saliency_of(node.screen_tile_saliency_report()),
        )),
        ("PATCH", "/api/v1/screen/tile/saliency") => patch_screen_tile_saliency(node, body).await,
        ("DELETE", "/api/v1/screen/tile/saliency") => delete_screen_tile_saliency(node).await,
        ("GET", "/api/v1/screen/tile/lod") => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_lod_of(node.screen_tile_lod_report()),
        )),
        ("PATCH", "/api/v1/screen/tile/lod") => patch_screen_tile_lod(node, body).await,
        ("DELETE", "/api/v1/screen/tile/lod") => delete_screen_tile_lod(node).await,
        ("GET", "/api/v1/screen/tile/batch") => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_batch_of(node.screen_tile_batch_report()),
        )),
        ("PATCH", "/api/v1/screen/tile/batch") => patch_screen_tile_batch(node, body).await,
        ("DELETE", "/api/v1/screen/tile/batch") => delete_screen_tile_batch(node).await,
        ("GET", "/api/v1/screen/tile/motion") => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_motion_of(node.screen_tile_motion_report()),
        )),
        ("PATCH", "/api/v1/screen/tile/motion") => patch_screen_tile_motion(node, body).await,
        ("DELETE", "/api/v1/screen/tile/motion") => delete_screen_tile_motion(node).await,
        ("GET", "/api/v1/screen/ltr") => Ok(json(
            200,
            &crate::apiv1::model::screen_ltr_of(node.screen_ltr_report()),
        )),
        ("PATCH", "/api/v1/screen/ltr") => patch_screen_ltr(node, body).await,
        ("DELETE", "/api/v1/screen/ltr") => delete_screen_ltr(node).await,
        ("GET", "/api/v1/screen/svc") => Ok(json(
            200,
            &crate::apiv1::model::screen_svc_of(node.screen_svc_report()),
        )),
        ("PATCH", "/api/v1/screen/svc") => patch_screen_svc(node, body).await,
        ("DELETE", "/api/v1/screen/svc") => delete_screen_svc(node).await,
        ("GET", "/api/v1/screen/probe") => Ok(json(
            200,
            &crate::apiv1::model::screen_probe_of(node.screen_probe_report()),
        )),
        ("PATCH", "/api/v1/screen/probe") => patch_screen_probe(node, body).await,

        ("GET", "/api/v1/profiles") => Ok(json(
            200,
            &crate::apiv1::model::peer_profiles(
                node,
                profile_filter(query, "tag"),
                profile_filter(query, "group"),
                profile_filter(query, "type"),
            ),
        )),
        ("GET", p) if profile_item(p).is_some() => {
            profile_json(node, &profile_item(p).unwrap_or_default())
        }
        ("PATCH", p) if profile_item(p).is_some() => {
            patch_profile(node, &profile_item(p).unwrap_or_default(), body)
        }
        ("GET", "/api/v1/caps") => Ok(json(200, &crate::apiv1::model::local_caps(node))),
        ("PATCH", "/api/v1/caps") => patch_caps(node, body).await,
        ("GET", "/api/v1/caps/peers") => match crate::apiv1::model::peer_caps_all(node) {
            Ok(peers) => Ok(json(200, &peers)),
            Err(e) => Err(core_error(e)),
        },
        // §156b: topology domain.
        ("GET", "/api/v1/topology") => Ok(json(200, &crate::apiv1::model::topology(node))),
        ("GET", "/api/v1/topology/peers") => {
            Ok(json(200, &crate::apiv1::model::peer_topologies(node)))
        }
        ("GET", "/api/v1/topology/policy") => {
            Ok(json(200, &crate::apiv1::model::topology_policy(node)))
        }
        ("PATCH", "/api/v1/topology/policy") => patch_topology_policy(node, body),
        ("POST", "/api/v1/topology/links") => post_topology_link(node, body).await,
        ("POST", "/api/v1/topology/unlinks") => post_topology_unlink(node, body).await,
        ("POST", "/api/v1/topology/sync") => post_topology_sync(node).await,
        ("PATCH", "/api/v1/topology/config") => patch_topology_config(node, body).await,
        // §156b: routes / network domain.
        ("GET", "/api/v1/routes") => Ok(json(200, &crate::apiv1::model::routes(node))),
        ("POST", "/api/v1/routes") => post_route(node, body),
        ("DELETE", "/api/v1/routes") => delete_route(node, query),
        ("GET", "/api/v1/net") => Ok(json(200, &crate::apiv1::model::network(node))),
        ("POST", "/api/v1/net/refresh") => {
            let rep = node.refresh_network().await;
            Ok(json(200, &crate::apiv1::model::network_refresh(rep)))
        }
        // §160: discovery + daemon lifecycle.
        ("GET", "/api/v1/discovery") => Ok(json(200, &crate::apiv1::model::discovery(node))),
        ("POST", "/api/v1/discovery") => {
            Ok(json(200, &crate::apiv1::model::discovery_rescan(node)))
        }
        ("POST", "/api/v1/system/shutdown") => Ok(post_system_shutdown(node)),
        ("POST", "/api/v1/system/restart") => post_system_restart(node, body).await,
        // §161: notify domain.
        ("POST", "/api/v1/notify") => post_notify_send(node, body).await,
        ("GET", "/api/v1/notify/candidates") => get_notify_candidates(node, query).await,
        ("GET", "/api/v1/notify/replies") => Ok(get_notify_replies(node)),
        ("GET", "/api/v1/notify/config") => Ok(get_notify_config(node)),
        ("PATCH", "/api/v1/notify/config") => Ok(patch_notify_config(node, query, body)),
        ("POST", "/api/v1/notify/dismissals") => post_notify_dismiss(node, query, body).await,
        ("POST", "/api/v1/notify/otp") => Ok(post_notify_otp(node, body)),
        ("POST", "/api/v1/notify/open") => Ok(post_notify_open(node, body)),
        ("POST", "/api/v1/notify/test") => Ok(post_notify_test(node, query, body)),
        ("POST", "/api/v1/notify/copies") => post_notify_copy(node, body).await,
        ("POST", "/api/v1/notify/insertions") => Ok(post_notify_insert(node, body)),
        // §161: reply domain.
        ("POST", "/api/v1/replies") => post_notify_reply(node, body).await,
        ("POST", "/api/v1/replies/take") => Ok(post_notify_reply_take(node, body)),
        // §161: message domain.
        ("GET", "/api/v1/messages") => get_message_history(node, query, body).await,
        ("POST", "/api/v1/messages") => post_message_send(node, query, body).await,
        ("DELETE", "/api/v1/messages") => post_message_clear(node, query, body).await,
        ("GET", p) if matches!(device_action(p), Some((_, "caps"))) => peer_caps_json(
            node,
            &device_action(p).map(|(id, _)| id).unwrap_or_default(),
        ),
        ("GET", p) if device_item(p).is_some() => {
            Ok(device_json(node, &device_item(p).unwrap_or_default()))
        }
        ("PATCH", p) if device_item(p).is_some() => {
            patch_device(node, &device_item(p).unwrap_or_default(), body)
        }
        ("DELETE", p) if device_item(p).is_some() => {
            forget_device(node, &device_item(p).unwrap_or_default()).await
        }
        (m, p) => match (m, device_action(p)) {
            ("POST", Some((id, "connect"))) => connect_device(node, &id, body).await,
            ("POST", Some((id, "ping"))) => ping_device(node, &id).await,
            ("POST", Some((id, "probe"))) => probe_device(node, &id, body).await,
            ("POST", Some((id, "disconnect"))) => disconnect_device(node, &id).await,
            ("POST", Some((id, "block"))) => block_device(node, &id, body).await,
            ("DELETE", Some((id, "block"))) => unblock_device(node, &id).await,
            _ => match (m, transfer_action(p)) {
                ("POST", Some((id, "cancel"))) => cancel_transfer(node, id).await,
                _ => match (m, inbox_content_item(p)) {
                    // §162b: raw file bytes (legacy inbox download route).
                    ("GET", Some(name)) => get_inbox_content(node, &name),
                    _ => match (m, inbox_open_action(p)) {
                        ("POST", Some(name)) => open_inbox_file(node, &name, body, granted).await,
                        _ => match (m, inbox_item(p)) {
                            ("DELETE", Some(name)) => delete_inbox_file(node, &name),
                            _ => Ok(match allowed(path) {
                                Some(methods) => error(
                                    ErrorCode::MethodNotAllowed,
                                    format!("{method} not allowed; use {}", methods.join(", ")),
                                )
                                .with_header("Allow", methods.join(", ")),
                                None => {
                                    error(ErrorCode::NotFound, format!("no such resource: {path}"))
                                }
                            }),
                        },
                    },
                },
            },
        },
    };
    result.unwrap_or_else(|e| e)
}

type Handled = Result<UiResponse, UiResponse>;

/// `PATCH /settings`: applies general config writes and the auto-accept
/// master toggle, then returns the full settings resource.
fn patch_settings(node: &Node, body: &[u8]) -> Handled {
    let patch: SettingsPatch = parse_body(body)?;
    if patch.is_empty() {
        return Err(error(ErrorCode::BadRequest, "patch sets no field"));
    }
    if let Some(owner) = patch.tray_owner {
        let (owner, changed) = node.set_tray_owner(owner).map_err(core_error)?;
        if changed {
            // §6.3: fold the claim into the built-in tray's visibility.
            crate::platform::tray::tray_set_visible(owner == tetherly_api::TrayOwner::Daemon);
        }
    }
    if patch.name.is_some() || patch.advertise.is_some() || patch.reconnect.is_some() {
        node.update_config(patch.name.as_deref(), patch.advertise, patch.reconnect)
            .map_err(core_error)?;
    }
    if let Some(enabled) = patch.auto_accept {
        let mut policy = node.auto_accept_policy();
        policy.enabled = enabled;
        node.set_auto_accept_policy(policy).map_err(core_error)?;
    }
    Ok(json(200, &crate::apiv1::model::settings(node)))
}

/// `POST /transfers/{id}/cancel`: aborts one active transfer; returns the
/// updated transfers resource.
async fn cancel_transfer(node: &Node, id: &str) -> Handled {
    node.cancel_transfer(id).await.map_err(core_error)?;
    Ok(json(200, &crate::apiv1::model::transfers(node)))
}

/// `DELETE /inbox/{name}`: removes one received file; returns the updated
/// inbox resource.
fn delete_inbox_file(node: &Node, name: &str) -> Handled {
    node.delete_inbox_file(name).map_err(core_error)?;
    Ok(json(200, &crate::apiv1::model::inbox(node, None, None)))
}

/// `POST /inbox/{name}/open`: reveals the file with the OS opener. §162a
/// returns the open report (name + on-disk path) instead of an empty 204 so
/// the CLI's human/JSON printers keep the legacy detail level; both client
/// transports tolerate a body where they used to see 204.
async fn open_inbox_file(
    node: &Node,
    name: &str,
    body: &[u8],
    granted: Option<crate::apitoken::Tier>,
) -> Handled {
    // Name comes from the path; the body (optional) is parsed for strictness
    // parity with other v1 POSTs but carries nothing today.
    let _req: Option<tetherly_api::InboxOpenRequest> = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        Some(parse_body(body)?)
    };
    if node
        .inbox_report(None, None)
        .files
        .iter()
        .all(|f| f.name != name)
    {
        return Err(error(
            ErrorCode::NotFound,
            format!("no such inbox file: {name}"),
        ));
    }
    let opener = node.opener();
    let (name, node) = (name.to_string(), node.clone_handle());
    let rep =
        tokio::task::spawn_blocking(move || node.open_inbox_file(Some(&name), opener.as_ref()))
            .await
            .map_err(|_| error(ErrorCode::Internal, "open task failed"))?
            .map_err(core_error)?;
    let shown = redact_inbox_open_path(granted, rep.name.as_deref(), &rep.path);
    Ok(json(
        200,
        &tetherly_api::InboxOpenReport {
            name: rep.name,
            path: shown,
        },
    ))
}

/// `POST /inbox` (no name): reveals the inbox directory itself. §162a keeps
/// the CLI's `inbox open` (name omitted) on the v1 surface; the report's
/// `name` is null to mirror the directory branch of the legacy report.
async fn open_inbox_dir(node: &Node, granted: Option<crate::apitoken::Tier>) -> Handled {
    let opener = node.opener();
    let node = node.clone_handle();
    let rep = tokio::task::spawn_blocking(move || node.open_inbox_file(None, opener.as_ref()))
        .await
        .map_err(|_| error(ErrorCode::Internal, "open task failed"))?
        .map_err(core_error)?;
    let shown = redact_inbox_open_path(granted, rep.name.as_deref(), &rep.path);
    Ok(json(
        200,
        &tetherly_api::InboxOpenReport {
            name: rep.name,
            path: shown,
        },
    ))
}

/// P3: the OS opener runs inside the daemon, so a remote caller never needs
/// the absolute on-disk path. Only the Admin tier (the local operator's CLI)
/// keeps the real path; lower tiers get an inbox-relative name.
pub(super) fn redact_inbox_open_path(
    granted: Option<crate::apitoken::Tier>,
    name: Option<&str>,
    path: &str,
) -> String {
    if granted >= Some(crate::apitoken::Tier::Admin) {
        return path.to_string();
    }
    match name {
        Some(name) => format!("inbox/{name}"),
        None => "inbox".to_string(),
    }
}

// ---- §162b: file-domain handlers (offers / send / outbox / storage) ----

/// `POST /offers/accept` | `POST /offers/reject`: one offer by `id`, or every
/// pending offer from `peer` with `all` (legacy decision semantics; the
/// v1 report folds both shapes).
async fn post_file_decision(
    node: &Node,
    query: Option<&str>,
    body: &[u8],
    accept: bool,
) -> Handled {
    let req: tetherly_api::FileDecisionRequest = parse_body(body)?;
    let peer = req.peer.clone().or_else(|| profile_filter(query, "peer"));
    let id = req.id.clone().or_else(|| profile_filter(query, "id"));
    let all = req.all.unwrap_or(false);
    if all || (id.is_none() && peer.is_some()) {
        let rep = if accept {
            node.batch_accept_files(peer.as_deref()).await
        } else {
            node.batch_reject_files(peer.as_deref()).await
        }
        .map_err(core_error)?;
        Ok(json(200, &crate::apiv1::model::file_decision_batch(rep)))
    } else if let Some(id) = id {
        let rep = if accept {
            node.user_accept_file_reported(&id).await
        } else {
            node.user_reject_file_reported(&id).await
        }
        .map_err(core_error)?;
        Ok(json(
            200,
            &crate::apiv1::model::file_decision_single(accept, rep),
        ))
    } else {
        Err(error(
            ErrorCode::BadRequest,
            "missing required 'id', 'all', or 'peer' parameter",
        ))
    }
}

/// Resolve the single-peer send target: explicit peer must be connected;
/// empty falls to the only live peer (legacy ambiguity preserved).
fn resolve_send_target(
    node: &Node,
    peer: Option<&str>,
) -> Result<tetherly_core::DeviceId, UiResponse> {
    let peers = node.live_peers();
    match peer.filter(|s| !s.is_empty()) {
        Some(raw) => {
            let id = node
                .resolve_peer(raw)
                .map_err(|e| error(ErrorCode::BadRequest, format!("bad peer id: {e}")))?;
            if peers.iter().any(|p| p.device_id == id) {
                Ok(id)
            } else {
                Err(error(ErrorCode::BadRequest, "peer is not connected"))
            }
        }
        None => match peers.len() {
            0 => Err(error(ErrorCode::Conflict, "no connected peer")),
            1 => Ok(peers[0].device_id.clone()),
            _ => Err(error(
                ErrorCode::BadRequest,
                "multiple peers connected; pass peer (tdev_…)",
            )),
        },
    }
}

/// Decode inline `data_base64` (path OR bytes must be present).
fn decode_send_data(
    node: &Node,
    path: &Option<String>,
    data_base64: &Option<String>,
) -> Result<SendSource, UiResponse> {
    if let Some(b64) = data_base64 {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .map_err(|_| error(ErrorCode::BadRequest, "invalid base64"))?;
        if bytes.len() as u64 > tetherly_core::FILE_MAX {
            return Err(error(
                ErrorCode::BadRequest,
                "file exceeds FILE_MAX (64 GiB)",
            ));
        }
        return Ok(SendSource::Bytes(bytes));
    }
    let Some(path_str) = path else {
        return Err(error(
            ErrorCode::BadRequest,
            "either path or data_base64 must be provided",
        ));
    };
    let requested = std::path::Path::new(path_str);
    let canonical = node
        .validate_send_source_path(requested)
        .map_err(core_error)?;
    Ok(SendSource::Path(canonical))
}

enum SendSource {
    Path(std::path::PathBuf),
    Bytes(Vec<u8>),
}

/// `POST /transfers`: one file to one peer (legacy `/api/file/send`).
async fn post_file_send(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::FileSendRequest = parse_body(body)?;
    let target = resolve_send_target(node, req.peer.as_deref())?;
    let name = req.name.clone();
    match decode_send_data(node, &req.path, &req.data_base64)? {
        SendSource::Path(p) => {
            let id = node
                .offer_and_send_file(&target, &p, name.as_deref())
                .await
                .map_err(core_error)?;
            let fname = name.unwrap_or_else(|| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file.bin")
                    .to_string()
            });
            let safe = tetherly_core::sanitize_file_name(&fname)
                .map_err(|e| error(ErrorCode::BadRequest, format!("invalid file name: {e}")))?;
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            Ok(json(
                200,
                &tetherly_api::FileSendReport {
                    transfer_id: id,
                    name: safe,
                    size,
                    peer: target.as_str().to_string(),
                    sha256: None,
                },
            ))
        }
        SendSource::Bytes(bytes) => {
            let digest = tetherly_core::hex_sha256(&bytes);
            let size = bytes.len() as u64;
            let safe_name =
                tetherly_core::sanitize_file_name(name.as_deref().unwrap_or("upload.bin"))
                    .map_err(|e| error(ErrorCode::BadRequest, format!("invalid file name: {e}")))?;
            let id = node
                .offer_and_send_bytes(&target, &safe_name, &bytes, false)
                .await
                .map_err(core_error)?;
            Ok(json(
                200,
                &tetherly_api::FileSendReport {
                    transfer_id: id,
                    name: safe_name,
                    size,
                    peer: target.as_str().to_string(),
                    sha256: Some(digest),
                },
            ))
        }
    }
}

/// `POST /transfers/multi`: one file fanned out to a selector (legacy
/// `/api/file/send-multi`; selector axes fold exactly one target set).
async fn post_file_send_multi(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::FileSendMultiRequest = parse_body(body)?;
    let selector = crate::store::TargetSelector::from_query(
        req.peer.as_deref(),
        req.tag.as_deref(),
        req.group.as_deref(),
        req.device_type.as_deref(),
        req.all.unwrap_or(false),
    )
    .map_err(|e| error(ErrorCode::BadRequest, e))?;
    let name = req.name.clone();
    let rep = match decode_send_data(node, &req.path, &req.data_base64)? {
        SendSource::Path(p) => node
            .offer_and_send_file_multi(&selector, &p, name.as_deref())
            .await
            .map_err(core_error)?,
        SendSource::Bytes(bytes) => {
            let fname = name.as_deref().unwrap_or("uploaded_file");
            node.offer_and_send_bytes_multi(&selector, fname, &bytes)
                .await
                .map_err(core_error)?
        }
    };
    Ok(json(200, &crate::apiv1::model::file_dispatch(rep)))
}

/// `POST /transfers/batch`: many files to one peer (legacy
/// `/api/file/send-batch`).
async fn post_file_send_batch(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::FileBatchRequest = parse_body(body)?;
    let target = resolve_send_target(node, req.peer.as_deref())?;
    let mut all_items: Vec<crate::runtime::BatchSendItem> = req
        .items
        .into_iter()
        .map(|i| crate::runtime::BatchSendItem {
            path: i.path,
            name: i.name,
            data_base64: i.data_base64,
        })
        .collect();
    for p in req.paths {
        all_items.push(crate::runtime::BatchSendItem {
            path: Some(p),
            name: None,
            data_base64: None,
        });
    }
    if all_items.is_empty() {
        return Err(error(
            ErrorCode::BadRequest,
            "no files specified in paths or items",
        ));
    }
    let rep = node
        .send_batch_items(&target, all_items)
        .await
        .map_err(core_error)?;
    Ok(json(200, &crate::apiv1::model::file_batch(rep)))
}

/// `POST /outbox/{id}/resend`: re-offer a sent file from the outbox ring.
async fn post_outbox_resend(node: &Node, id: &str) -> Handled {
    let rep = node.resend_outbox_file(id).await.map_err(core_error)?;
    Ok(json(200, &crate::apiv1::model::outbox_resend(rep)))
}

/// `PATCH /policy/auto-accept`: partial policy update; omitted fields stay
/// untouched, JSON `null` on `max_file_size` lifts the size cap.
fn patch_auto_accept_policy(node: &Node, body: &[u8]) -> Handled {
    let patch: tetherly_api::AutoAcceptPolicyPatch = parse_body(body)?;
    if patch.enabled.is_none()
        && patch.allowed_groups.is_none()
        && patch.allowed_tags.is_none()
        && patch.allowed_types.is_none()
        && matches!(patch.max_file_size, tetherly_api::Patch::Leave)
    {
        return Err(error(ErrorCode::BadRequest, "patch sets no field"));
    }
    let mut policy = node.auto_accept_policy();
    if let Some(en) = patch.enabled {
        policy.enabled = en;
    }
    if let Some(groups) = patch.allowed_groups {
        policy.allowed_groups = groups;
    }
    if let Some(tags) = patch.allowed_tags {
        policy.allowed_tags = tags;
    }
    if let Some(types) = patch.allowed_types {
        let mut dtypes = Vec::new();
        for t in types {
            match t.parse::<crate::store::DeviceType>() {
                Ok(dt) => dtypes.push(dt),
                Err(e) => {
                    return Err(error(
                        ErrorCode::BadRequest,
                        format!("invalid device type '{t}': {e}"),
                    ));
                }
            }
        }
        policy.allowed_types = dtypes;
    }
    match patch.max_file_size {
        tetherly_api::Patch::Set(sz) => policy.max_file_size = Some(sz),
        tetherly_api::Patch::Clear => policy.max_file_size = None,
        tetherly_api::Patch::Leave => {}
    }
    node.set_auto_accept_policy(policy).map_err(core_error)?;
    Ok(json(200, &crate::apiv1::model::auto_accept_policy(node)))
}

/// `POST /clip/sync` (§163): push explicit `text` or the current local
/// clipboard to one peer (explicit, connected) or the single live peer.
async fn post_clip_sync(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::ClipSyncRequest = parse_body(body)?;
    let peers = node.live_peers();
    let target_peer = match req.peer.as_deref().filter(|s| !s.is_empty()) {
        Some(raw) => {
            let id = node
                .resolve_peer(raw)
                .map_err(|e| error(ErrorCode::BadRequest, format!("bad peer id: {e}")))?;
            if !peers.iter().any(|p| p.device_id == id) {
                return Err(error(ErrorCode::BadRequest, "peer is not connected"));
            }
            Some(id)
        }
        None => None,
    };
    if let Some(text) = req.text {
        if text.len() > tetherly_core::CLIP_TEXT_MAX {
            return Err(error(
                ErrorCode::BadRequest,
                "clipboard text exceeds 1 MiB limit",
            ));
        }
        let hash = tetherly_core::hex_sha256(text.as_bytes());
        let bytes = text.len() as u64;
        let sent = node
            .send_clipboard_text(&text, target_peer.as_ref())
            .await
            .map_err(core_error)?;
        Ok(json(
            200,
            &tetherly_api::ClipSyncReport {
                bytes,
                hash,
                sent_peers: sent as u64,
            },
        ))
    } else {
        let cur = match node.get_clipboard_text() {
            Ok(Some(t)) => t,
            Ok(None) => return Err(error(ErrorCode::BadRequest, "local clipboard is empty")),
            Err(e) => return Err(core_error(e)),
        };
        if cur.len() > tetherly_core::CLIP_TEXT_MAX {
            return Err(error(
                ErrorCode::BadRequest,
                "clipboard text exceeds 1 MiB limit",
            ));
        }
        let hash = tetherly_core::hex_sha256(cur.as_bytes());
        let bytes = cur.len() as u64;
        let sent = node
            .send_clipboard_to(target_peer.as_ref())
            .await
            .map_err(core_error)?;
        Ok(json(
            200,
            &tetherly_api::ClipSyncReport {
                bytes,
                hash,
                sent_peers: sent as u64,
            },
        ))
    }
}

/// `PATCH /clip/policy` (§163): sparse policy patch with the legacy
/// query-object fallback (body wins) and the `clear_filters` sweep.
async fn patch_clip_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let patch: tetherly_api::ClipPolicyPatch = if body.iter().all(u8::is_ascii_whitespace) {
        tetherly_api::ClipPolicyPatch::default()
    } else {
        parse_body(body)?
    };
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |key: &str| qmap.get(key).cloned();
    let mut policy = node.clipboard_policy();

    if let Some(m) = patch.mode.clone().or_else(|| q_str("mode")) {
        match m.parse::<tetherly_core::ClipSyncMode>() {
            Ok(mode) => policy.mode = mode,
            Err(e) => {
                return Err(error(
                    ErrorCode::BadRequest,
                    format!("invalid clip sync mode '{m}': {e}"),
                ));
            }
        }
    }
    if let Some(sg) = patch
        .sensitive_guard
        .or_else(|| q_str("sensitive_guard").map(|s| s == "true" || s == "1"))
    {
        policy.sensitive_guard = sg;
    }
    let groups = patch
        .allowed_groups
        .clone()
        .or_else(|| q_list(&qmap, "group").or_else(|| q_list(&qmap, "groups")));
    if let Some(groups) = groups {
        policy.allowed_groups = groups;
    }
    let tags = patch
        .allowed_tags
        .clone()
        .or_else(|| q_list(&qmap, "tag").or_else(|| q_list(&qmap, "tags")));
    if let Some(tags) = tags {
        policy.allowed_tags = tags;
    }
    if let Some(types) = patch.allowed_types.clone().or_else(|| {
        q_str("type")
            .or_else(|| q_str("types"))
            .map(|raw| split_csv(&raw))
    }) {
        let mut dtypes = Vec::new();
        for t in types {
            match t.parse::<crate::store::DeviceType>() {
                Ok(dt) => dtypes.push(dt),
                Err(e) => {
                    return Err(error(
                        ErrorCode::BadRequest,
                        format!("invalid device type '{t}': {e}"),
                    ));
                }
            }
        }
        policy.allowed_types = dtypes;
    }
    let bytes = patch.max_payload_bytes.or_else(|| {
        q_str("max_payload_bytes")
            .or_else(|| q_str("max_bytes"))
            .and_then(|s| s.parse::<u64>().ok())
    });
    if let Some(bytes) = bytes {
        policy.max_payload_bytes = usize::try_from(bytes).unwrap_or(usize::MAX);
    }
    if let Some(secs) = patch.auto_clear_timeout_secs.or_else(|| {
        q_str("auto_clear_timeout_secs")
            .or_else(|| q_str("auto_clear"))
            .and_then(|s| s.parse::<u64>().ok())
    }) {
        policy.auto_clear_timeout_secs = if secs == 0 { None } else { Some(secs) };
    }
    let clear_filters = patch.clear_filters.unwrap_or(false)
        || q_str("clear_filters")
            .map(|s| s == "true" || s == "1")
            .unwrap_or(false);
    if clear_filters {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    node.set_clipboard_policy(policy).map_err(core_error)?;
    Ok(json(200, &crate::apiv1::model::clip_policy(node)))
}

/// Comma-separated query list (legacy group/tag folding).
fn q_list(qmap: &std::collections::HashMap<String, String>, key: &str) -> Option<Vec<String>> {
    let raw = qmap.get(key)?;
    Some(
        raw.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
    )
}

/// Comma-separated list to trimmed non-empty strings.
fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// `GET /audit` (§163): filtered trail; query params or a JSON body mirror.
fn get_audit_v1(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let mut cat = qmap.get("category").or_else(|| qmap.get("cat")).cloned();
    let mut sev = qmap.get("severity").or_else(|| qmap.get("sev")).cloned();
    let mut lim = qmap.get("limit").and_then(|s| s.parse::<u64>().ok());
    let mut since = qmap
        .get("since")
        .or_else(|| qmap.get("since_ms"))
        .and_then(|s| s.parse::<u64>().ok());
    if !body.iter().all(u8::is_ascii_whitespace) {
        let b: tetherly_api::AuditQuery = parse_body(body)?;
        if b.category.is_some() {
            cat = b.category;
        }
        if b.severity.is_some() {
            sev = b.severity;
        }
        if b.limit.is_some() {
            lim = b.limit;
        }
        if b.since.is_some() {
            since = b.since;
        }
    }
    let cat = match cat.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(c) => match c.parse::<crate::runtime::AuditCategory>() {
            Ok(cat) => Some(cat),
            Err(e) => return Err(error(ErrorCode::BadRequest, e)),
        },
        None => None,
    };
    let sev = match sev.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(s) => match s.parse::<crate::runtime::AuditSeverity>() {
            Ok(sev) => Some(sev),
            Err(e) => return Err(error(ErrorCode::BadRequest, e)),
        },
        None => None,
    };
    Ok(json(
        200,
        &crate::apiv1::model::audit_trail(node, cat, sev, lim, since),
    ))
}

/// Check how the legacy api_action/api_bench handlers were reached from
/// dispatch to mirror their semantics in the §164 v1 handlers below.
/// `POST /action/local` (§164): register or replace one local action.
async fn post_action_local(node: &Node, body: &[u8]) -> Handled {
    let req: LocalActionRequest = parse_body_strict(body)?;
    if req.id.trim().is_empty() {
        return Err(error(ErrorCode::BadRequest, "missing id"));
    }
    if req.command.trim().is_empty() {
        return Err(error(ErrorCode::BadRequest, "missing command"));
    }
    let def = crate::store::LocalActionDefinition {
        id: req.id,
        name: req.name,
        description: req.description,
        icon: req.icon,
        command: req.command,
        args: req.args,
        timeout_secs: req.timeout_secs.unwrap_or(10),
        enabled: req.enabled.unwrap_or(true),
    };
    match crate::apiv1::model::local_action_add(node, def) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /action/policy` (§164): sparse update, body wins over legacy query.
async fn patch_action_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<ActionPolicyPatch>(body)?;
    if patch.mode.is_none() {
        patch.mode = q_str("mode");
    }
    if patch.max_timeout_secs.is_none() {
        patch.max_timeout_secs = q_str("max_timeout_secs")
            .or_else(|| q_str("timeout"))
            .and_then(|s| s.parse().ok());
    }
    if patch.expose_action_list.is_none() {
        patch.expose_action_list = q_str("expose_action_list")
            .or_else(|| q_str("expose"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_list(&qmap, "allowed_groups").or_else(|| q_list(&qmap, "group"));
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_list(&qmap, "allowed_tags").or_else(|| q_list(&qmap, "tag"));
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_list(&qmap, "allowed_types").or_else(|| q_list(&qmap, "type"));
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.action_policy();
    if let Some(m) = &patch.mode {
        policy.mode = m
            .parse()
            .map_err(|e: CoreError| error(ErrorCode::BadRequest, e.to_string()))?;
    }
    if let Some(t) = patch.max_timeout_secs {
        policy.max_timeout_secs = tetherly_core::ActionValidator::clamp_timeout(t);
    }
    if let Some(e) = patch.expose_action_list {
        policy.expose_action_list = e;
    }
    // Legacy query variant *adds* to the allow-lists; body replaces.
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    Ok(json(
        200,
        &crate::apiv1::model::set_action_policy(node, policy).map_err(core_error)?,
    ))
}

/// `POST /bench/start` (§164): body or legacy query params.
async fn post_bench_start(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let req: BenchStartRequest = if !body.iter().all(u8::is_ascii_whitespace) {
        parse_body_strict(body)?
    } else {
        let peer = qmap.get("peer").or_else(|| qmap.get("peer_id")).cloned();
        let Some(peer) = peer else {
            return Err(error(ErrorCode::BadRequest, "missing peer"));
        };
        let direction = qmap.get("direction").or_else(|| qmap.get("dir")).cloned();
        let duration_secs = qmap
            .get("duration_secs")
            .or_else(|| qmap.get("duration"))
            .and_then(|s| s.parse().ok());
        let chunk_size = qmap
            .get("chunk_size")
            .or_else(|| qmap.get("chunk"))
            .and_then(|s| s.parse().ok());
        BenchStartRequest {
            peer,
            direction: direction.unwrap_or_else(|| "push".to_string()),
            duration_secs,
            chunk_size,
        }
    };
    let direction = match req.direction.to_ascii_lowercase().as_str() {
        "pull" | "download" | "rx" | "recv" => tetherly_core::BenchDirection::Pull,
        _ => tetherly_core::BenchDirection::Push,
    };
    match node
        .bench_start(&req.peer, direction, req.duration_secs, req.chunk_size)
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::bench_decision(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /bench/policy` (§164): sparse update, body wins over legacy query.
async fn patch_bench_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<BenchPolicyPatch>(body)?;
    if patch.enabled.is_none() {
        patch.enabled = q_str("enabled").and_then(|s| s.parse().ok());
    }
    if patch.allow_inbound_bench.is_none() {
        patch.allow_inbound_bench = q_str("allow_inbound_bench")
            .or_else(|| q_str("inbound"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_duration_secs.is_none() {
        patch.max_duration_secs = q_str("max_duration_secs")
            .or_else(|| q_str("duration"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_bandwidth_mbps.is_none() {
        patch.max_bandwidth_mbps = q_str("max_bandwidth_mbps")
            .or_else(|| q_str("bandwidth"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_list(&qmap, "allowed_groups").or_else(|| q_list(&qmap, "group"));
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_list(&qmap, "allowed_tags").or_else(|| q_list(&qmap, "tag"));
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_list(&qmap, "allowed_types").or_else(|| q_list(&qmap, "type"));
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.bench_policy();
    if let Some(e) = patch.enabled {
        policy.enabled = e;
    }
    if let Some(a) = patch.allow_inbound_bench {
        policy.allow_inbound_bench = a;
    }
    if let Some(d) = patch.max_duration_secs {
        policy.max_duration_secs = d;
    }
    if let Some(b) = patch.max_bandwidth_mbps {
        policy.max_bandwidth_mbps = b;
    }
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    Ok(json(
        200,
        &crate::apiv1::model::set_bench_policy(node, policy).map_err(core_error)?,
    ))
}

/// `GET /battery/state?peer=` (§165): local state or a peer's last broadcast.
async fn get_battery_state(node: &Node, query: Option<&str>) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let peer = qmap
        .get("peer")
        .or_else(|| qmap.get("target"))
        .map(|s| s.as_str());
    match crate::apiv1::model::battery_state(node, peer) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /battery/state` (§165): sparse local state update; body wins over
/// the legacy query aliases (`level`/`percent`, `charging`, `source`,
/// `saver`, `health`).
async fn patch_battery_state(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<BatteryStatePatch>(body)?;
    if patch.battery_level.is_none() {
        patch.battery_level = q_str("level")
            .or_else(|| q_str("battery_level"))
            .or_else(|| q_str("percent"))
            .and_then(|s| s.parse().ok());
    }
    if patch.is_charging.is_none() {
        patch.is_charging = q_str("charging")
            .or_else(|| q_str("is_charging"))
            .and_then(|s| s.parse().ok());
    }
    if patch.power_source.is_none() {
        patch.power_source = q_str("source")
            .or_else(|| q_str("power_source"))
            .or_else(|| q_str("source_type"));
    }
    if patch.battery_saver.is_none() {
        patch.battery_saver = q_str("saver")
            .or_else(|| q_str("battery_saver"))
            .or_else(|| q_str("low_power_mode"))
            .and_then(|s| s.parse().ok());
    }
    if patch.health_percent.is_none() {
        patch.health_percent = q_str("health")
            .or_else(|| q_str("health_percent"))
            .and_then(|s| s.parse().ok());
    }

    // Start from the current frame, apply the sparse set, let the node clamp.
    let current = match node.battery_get_state(None) {
        Ok(rep) => rep.state.unwrap_or_default(),
        Err(e) => return Err(core_error(e)),
    };
    let mut state = current;
    if let Some(lvl) = patch.battery_level {
        state.battery_level = lvl.min(100);
    }
    if let Some(chg) = patch.is_charging {
        state.is_charging = chg;
    }
    if let Some(src) = patch.power_source {
        state.power_source = src
            .parse::<tetherly_core::PowerSource>()
            .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?;
    }
    if let Some(saver) = patch.battery_saver {
        state.battery_saver = saver;
    }
    if let Some(hp) = patch.health_percent {
        state.health_percent = Some(hp.min(100));
    }
    match crate::apiv1::model::set_battery_state(node, state).await {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /battery/policy` (§165): sparse update, body wins over legacy
/// query (threshold/alert/throttle aliases + append-dedup filters).
async fn patch_battery_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<BatteryPolicyPatch>(body)?;
    if patch.low_battery_threshold.is_none() {
        patch.low_battery_threshold = q_str("low_battery_threshold")
            .or_else(|| q_str("low_threshold"))
            .or_else(|| q_str("low"))
            .and_then(|s| s.parse().ok());
    }
    if patch.critical_battery_threshold.is_none() {
        patch.critical_battery_threshold = q_str("critical_battery_threshold")
            .or_else(|| q_str("critical_threshold"))
            .or_else(|| q_str("critical"))
            .and_then(|s| s.parse().ok());
    }
    if patch.alert_on_low_battery.is_none() {
        patch.alert_on_low_battery = q_str("alert_on_low_battery")
            .or_else(|| q_str("alert"))
            .and_then(|s| s.parse().ok());
    }
    if patch.auto_throttle_on_battery_saver.is_none() {
        patch.auto_throttle_on_battery_saver = q_str("auto_throttle_on_battery_saver")
            .or_else(|| q_str("throttle"))
            .or_else(|| q_str("auto_throttle"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_list(&qmap, "allowed_groups").or_else(|| q_list(&qmap, "group"));
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_list(&qmap, "allowed_tags").or_else(|| q_list(&qmap, "tag"));
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_list(&qmap, "allowed_types").or_else(|| q_list(&qmap, "type"));
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.battery_policy();
    if let Some(v) = patch.low_battery_threshold {
        policy.low_battery_threshold = v.min(100);
    }
    if let Some(v) = patch.critical_battery_threshold {
        policy.critical_battery_threshold = v.min(100);
    }
    if let Some(v) = patch.alert_on_low_battery {
        policy.alert_on_low_battery = v;
    }
    if let Some(v) = patch.auto_throttle_on_battery_saver {
        policy.auto_throttle_on_battery_saver = v;
    }
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    Ok(json(
        200,
        &crate::apiv1::model::set_battery_policy(node, policy).map_err(core_error)?,
    ))
}
/// `POST /media/command` (§165): body or legacy query; single peer or
/// selector fan-out (`all`/`tag`/`group`/`device_type`). Body wins.
async fn post_media_command(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<MediaCommandRequest>(body)?;
    if req.peer.is_none() {
        req.peer = q_str("peer").or_else(|| q_str("target"));
    }
    if req.command.is_none() {
        req.command = q_str("command")
            .or_else(|| q_str("cmd"))
            .or_else(|| q_str("action"));
    }
    if req.volume.is_none() {
        req.volume = q_str("volume")
            .or_else(|| q_str("volume_percent"))
            .and_then(|s| s.parse().ok());
    }
    if req.offset_ms.is_none() {
        req.offset_ms = q_str("offset_ms")
            .or_else(|| q_str("offset"))
            .and_then(|s| s.parse().ok());
    }
    if req.target_player.is_none() {
        req.target_player = q_str("target_player").or_else(|| q_str("player"));
    }
    if req.tag.is_none() {
        req.tag = q_str("tag");
    }
    if req.group.is_none() {
        req.group = q_str("group");
    }
    if req.device_type.is_none() {
        req.device_type = q_str("device_type").or_else(|| q_str("type"));
    }
    if req.all.is_none() {
        req.all = q_str("all")
            .map(|s| s == "true" || s == "1")
            .or_else(|| qmap.contains_key("all").then_some(true));
    }

    let cmd_str = req
        .command
        .clone()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if cmd_str.is_empty() {
        return Err(error(
            ErrorCode::BadRequest,
            "missing required 'command' or 'action'",
        ));
    }
    let command = match cmd_str.as_str() {
        "play" => tetherly_core::MediaCommand::Play,
        "pause" => tetherly_core::MediaCommand::Pause,
        "play_pause" | "playpause" | "toggle" => tetherly_core::MediaCommand::PlayPause,
        "stop" => tetherly_core::MediaCommand::Stop,
        "next" => tetherly_core::MediaCommand::Next,
        "prev" | "previous" => tetherly_core::MediaCommand::Previous,
        "seek" => tetherly_core::MediaCommand::Seek {
            offset_ms: req.offset_ms.unwrap_or(0),
        },
        "volume" | "set_volume" => tetherly_core::MediaCommand::SetVolume {
            volume_percent: req.volume.unwrap_or(0),
        },
        "toggle_mute" | "mute" => tetherly_core::MediaCommand::ToggleMute,
        other => {
            return Err(error(
                ErrorCode::BadRequest,
                format!("unsupported media command: '{other}'"),
            ));
        }
    };
    let target_player = req.target_player.clone();

    let is_all = req.all.unwrap_or(false);
    let selector_tag = req.tag.clone().filter(|t| !t.is_empty());
    let selector_group = req.group.clone().filter(|g| !g.is_empty());
    let selector_type = req.device_type.clone().filter(|t| !t.is_empty());
    let peer = req.peer.clone().filter(|p| !p.is_empty());

    if is_all || selector_tag.is_some() || selector_group.is_some() || selector_type.is_some() {
        let selector = crate::store::TargetSelector::from_query(
            peer.as_deref(),
            selector_tag.as_deref(),
            selector_group.as_deref(),
            selector_type.as_deref(),
            is_all,
        )
        .map_err(|e| error(ErrorCode::BadRequest, e))?;
        return match crate::apiv1::model::media_command_multi(
            node,
            &selector,
            command,
            target_player,
        )
        .await
        {
            Ok(rep) => Ok(json(200, &rep)),
            Err(e) => Err(core_error(e)),
        };
    }

    match crate::apiv1::model::media_command(
        node,
        &peer.unwrap_or_default(),
        command,
        target_player,
    )
    .await
    {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `GET /media/state?peer=` (§165).
fn get_media_state(node: &Node, query: Option<&str>) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let peer = qmap
        .get("peer")
        .or_else(|| qmap.get("target"))
        .map(String::as_str);
    match crate::apiv1::model::media_state(node, peer) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /media/state` (§165): sparse local playback update.
fn patch_media_state(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<MediaStatePatch>(body)?;
    let current = match node.media_get_state(None) {
        Ok(rep) => rep.state.unwrap_or_default(),
        Err(e) => return Err(core_error(e)),
    };
    let mut state = current;
    if let Some(pid) = patch.player_id {
        state.player_id = pid;
    }
    if let Some(ref st) = patch.state {
        state.state = st
            .parse::<tetherly_core::PlaybackState>()
            .map_err(|e| error(ErrorCode::BadRequest, e))?;
    }
    if let Some(v) = patch.title {
        state.title = v;
    }
    if let Some(v) = patch.artist {
        state.artist = v;
    }
    if let Some(v) = patch.album {
        state.album = v;
    }
    if let Some(v) = patch.position_ms {
        state.position_ms = v;
    }
    if let Some(v) = patch.duration_ms {
        state.duration_ms = v;
    }
    if let Some(v) = patch.volume_percent {
        state.volume_percent = v;
    }
    if let Some(v) = patch.muted {
        state.muted = v;
    }
    if let Some(v) = patch.can_play {
        state.can_play = v;
    }
    if let Some(v) = patch.can_pause {
        state.can_pause = v;
    }
    if let Some(v) = patch.can_go_next {
        state.can_go_next = v;
    }
    if let Some(v) = patch.can_go_previous {
        state.can_go_previous = v;
    }
    if let Some(v) = patch.can_seek {
        state.can_seek = v;
    }
    match crate::apiv1::model::set_media_state(node, state) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /media/policy` (§165): sparse update, body wins over legacy query.
async fn patch_media_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<MediaPolicyPatch>(body)?;
    if patch.mode.is_none() {
        patch.mode = q_str("mode");
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_list(&qmap, "allowed_groups").or_else(|| q_list(&qmap, "group"));
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_list(&qmap, "allowed_tags").or_else(|| q_list(&qmap, "tag"));
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_list(&qmap, "allowed_types").or_else(|| q_list(&qmap, "type"));
    }
    if patch.max_volume_limit.is_none() {
        patch.max_volume_limit = q_str("max_volume_limit")
            .or_else(|| q_str("max_volume"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_seek.is_none() {
        patch.allow_seek = q_str("allow_seek").and_then(|s| s.parse().ok());
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.media_policy();
    if let Some(ref m) = patch.mode {
        policy.mode = m
            .parse::<tetherly_core::MediaControlMode>()
            .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?;
    }
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if let Some(v) = patch.max_volume_limit {
        policy.max_volume_limit = v.min(100);
    }
    if let Some(v) = patch.allow_seek {
        policy.allow_seek = v;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    Ok(json(
        200,
        &crate::apiv1::model::set_media_policy(node, policy).map_err(core_error)?,
    ))
}

/// `POST /ancs/connect` (§165): trigger ANCS subscription.
async fn post_ancs_connect(node: &Node) -> Handled {
    match crate::apiv1::model::ancs_connect(node) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(error(ErrorCode::BadRequest, e.to_string())),
    }
}

/// `POST /ancs/allow` (§165): add one open-rule (`app_id` required, `url`
/// defaults to empty per the contract).
async fn post_ancs_allow(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body_strict::<AncsAllowRequest>(body)?;
    match crate::apiv1::model::ancs_allow(node, req) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(error(ErrorCode::BadRequest, e.to_string())),
    }
}

/// `DELETE /ancs/allow` (§165): remove one open-rule; query `app_id` wins
/// over the JSON body (legacy append semantics preserved).
async fn delete_ancs_allow(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let app_id = qmap
        .get("app_id")
        .cloned()
        .or_else(|| {
            parse_body::<AncsRemoveRequest>(body)
                .ok()
                .and_then(|b| b.app_id)
        })
        .filter(|s| !s.trim().is_empty());
    match app_id {
        Some(app_id) => Ok(json(
            200,
            &crate::apiv1::model::ancs_remove_rule(node, &app_id),
        )),
        None => Err(error(
            ErrorCode::BadRequest,
            "missing required 'app_id' in query or body",
        )),
    }
}

/// `GET /input` (§165): input sharing status.
async fn get_input_status(node: &Node) -> Handled {
    Ok(json(200, &crate::apiv1::model::input_status(node).await))
}

/// `GET /input/trace` (§165): input-engine decision ring.
async fn get_input_trace(node: &Node) -> Handled {
    Ok(json(200, &crate::apiv1::model::input_trace(node).await))
}

/// `POST /input/start` (§165): start input sharing (optional `peer`).
async fn post_input_start(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<InputStartRequest>(body)?;
    match crate::apiv1::model::input_control_start(node, req.peer).await {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /input/stop` (§165): stop input sharing.
async fn post_input_stop(node: &Node) -> Handled {
    match crate::apiv1::model::input_control_stop(node).await {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `GET /input/tunnel` (§165): tunnel session + policy status.
async fn get_input_tunnel(node: &Node) -> Handled {
    Ok(json(
        200,
        &crate::apiv1::model::input_tunnel_status(node).await,
    ))
}

/// `POST /input/tunnel/attach` (§165): body wins over legacy query aliases
/// (`peer_id`/`target` for `peer`).
async fn post_input_attach(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<InputAttachRequest>(body)?;
    if req.peer.is_none() {
        req.peer = q_str("peer")
            .or_else(|| q_str("peer_id"))
            .or_else(|| q_str("target"));
    }
    if req.edge.is_none() {
        req.edge = q_str("edge");
    }
    if req.x.is_none() {
        req.x = q_str("x").and_then(|s| s.parse().ok());
    }
    if req.y.is_none() {
        req.y = q_str("y").and_then(|s| s.parse().ok());
    }
    let peer = match req.peer.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => p.to_string(),
        None => return Err(error(ErrorCode::BadRequest, "missing 'peer' parameter")),
    };
    let edge = match req.edge.as_deref() {
        Some(s) => Some(
            s.parse::<tetherly_core::DisplayEdge>()
                .map_err(|e| error(ErrorCode::BadRequest, format!("invalid edge '{s}': {e}")))?,
        ),
        None => None,
    };
    match crate::apiv1::model::input_tunnel_attach(node, &peer, edge, req.x, req.y).await {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /input/tunnel/release` (§165): body wins over legacy query `reason`.
async fn post_input_release(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let mut req = parse_body::<InputReleaseRequest>(body)?;
    if req.reason.is_none() {
        req.reason = qmap.get("reason").cloned();
    }
    match crate::apiv1::model::input_tunnel_release(node, req.reason.as_deref()).await {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /input/policy` (§165): sparse update; body wins over legacy query
/// aliases, single-value `group`/`tag`/`type` query params append.
async fn patch_input_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<InputPolicyPatch>(body)?;
    if patch.enabled.is_none() {
        patch.enabled = q_str("enable")
            .or_else(|| q_str("enabled"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_inbound_control.is_none() {
        patch.allow_inbound_control = q_str("allow_inbound")
            .or_else(|| q_str("allow_inbound_control"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_keyboard.is_none() {
        patch.allow_keyboard = q_str("keyboard")
            .or_else(|| q_str("allow_keyboard"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_mouse.is_none() {
        patch.allow_mouse = q_str("mouse")
            .or_else(|| q_str("allow_mouse"))
            .and_then(|s| s.parse().ok());
    }
    if patch.hotkey_release_enabled.is_none() {
        patch.hotkey_release_enabled = q_str("hotkey")
            .or_else(|| q_str("hotkey_release_enabled"))
            .and_then(|s| s.parse().ok());
    }
    if patch.idle_timeout_secs.is_none() {
        patch.idle_timeout_secs = q_str("timeout")
            .or_else(|| q_str("idle_timeout_secs"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_list(&qmap, "allowed_groups").or_else(|| {
            q_str("group")
                .or_else(|| q_str("allowed_group"))
                .map(|g| vec![g])
        });
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_list(&qmap, "allowed_tags").or_else(|| {
            q_str("tag")
                .or_else(|| q_str("allowed_tag"))
                .map(|t| vec![t])
        });
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_list(&qmap, "allowed_types").or_else(|| {
            q_str("type")
                .or_else(|| q_str("device_type"))
                .map(|t| vec![t])
        });
    }
    if patch.keymap.is_none() {
        patch.keymap = q_str("keymap").or_else(|| q_str("key_map"));
    }
    if patch.edge.is_none() {
        patch.edge = q_str("edge").or_else(|| q_str("screen_edge"));
    }
    if patch.wheel_speed.is_none() {
        patch.wheel_speed = q_str("wheel_speed")
            .or_else(|| q_str("scroll_speed"))
            .and_then(|s| s.parse().ok());
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.input_policy();
    if let Some(v) = patch.enabled {
        policy.enabled = v;
    }
    if let Some(v) = patch.allow_inbound_control {
        policy.allow_inbound_control = v;
    }
    if let Some(v) = patch.allow_keyboard {
        policy.allow_keyboard = v;
    }
    if let Some(v) = patch.allow_mouse {
        policy.allow_mouse = v;
    }
    if let Some(v) = patch.hotkey_release_enabled {
        policy.hotkey_release_enabled = v;
    }
    if let Some(v) = patch.idle_timeout_secs {
        policy.idle_timeout_secs = v;
    }
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if let Some(km) = &patch.keymap {
        policy.keymap = if km.trim().is_empty() || km == "none" || km == "identity" || km == "off" {
            None
        } else {
            Some(km.trim().to_lowercase().replace('-', "_"))
        };
    }
    if let Some(edge_val) = &patch.edge {
        policy.edge = if edge_val.trim().is_empty() || edge_val == "none" || edge_val == "default" {
            None
        } else if let Ok(e) = edge_val
            .trim()
            .to_lowercase()
            .parse::<tetherly_core::ScreenEdge>()
        {
            Some(e.to_string())
        } else {
            return Err(error(
                ErrorCode::BadRequest,
                format!("invalid edge '{edge_val}' (left|right|top|bottom)"),
            ));
        };
    }
    if let Some(ws) = patch.wheel_speed {
        if !(ws > 0.0 && ws.is_finite() && ws <= 20.0) {
            return Err(error(
                ErrorCode::BadRequest,
                "wheel_speed must be within (0, 20]",
            ));
        }
        policy.wheel_speed = ws;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    match crate::apiv1::model::set_input_policy(node, policy) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// Resolve the handoff fan-out selector from a request + legacy query
/// aliases (body wins; `all` query flag also forces selector mode).
fn handoff_selector(
    req: &HandoffSendRequest,
    qmap: &std::collections::HashMap<String, String>,
) -> Result<Option<crate::store::TargetSelector>, UiResponse> {
    let q_bool = |k: &str| qmap.get(k).map(|s| s == "true" || s == "1");
    let q_str = |k: &str| qmap.get(k).cloned();
    let is_all =
        req.all.unwrap_or(false) || q_bool("all").unwrap_or(false) || qmap.contains_key("all");
    let tag = req.tag.clone().or_else(|| q_str("tag"));
    let group = req.group.clone().or_else(|| q_str("group"));
    let dev_type = req
        .device_type
        .clone()
        .or_else(|| q_str("device_type"))
        .or_else(|| q_str("type"));
    if is_all || tag.is_some() || group.is_some() || dev_type.is_some() {
        let peer = req.peer.clone().or_else(|| q_str("peer"));
        crate::store::TargetSelector::from_query(
            peer.as_deref(),
            tag.as_deref(),
            group.as_deref(),
            dev_type.as_deref(),
            is_all,
        )
        .map(Some)
        .map_err(|e| error(ErrorCode::BadRequest, e))
    } else {
        Ok(None)
    }
}

/// `POST /handoff/send` (§165): single peer, or selector fan-out when
/// `tag`/`group`/`device_type`/`all` present (legacy `/api/handoff/send`
/// dispatch split preserved).
async fn post_handoff_send(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<HandoffSendRequest>(body)?;
    if req.url.is_empty() {
        if let Some(u) = q_str("url") {
            req.url = u;
        }
    }
    if req.peer.is_none() {
        req.peer = q_str("peer");
    }
    if req.title.is_none() {
        req.title = q_str("title");
    }
    if req.auto_open.is_none() {
        req.auto_open = q_str("auto_open").and_then(|s| match s.as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        });
    }
    let url = req.url.trim();
    if url.is_empty() {
        return Err(error(ErrorCode::BadRequest, "url cannot be empty"));
    }
    match handoff_selector(&req, &qmap) {
        Err(resp) => Err(resp),
        Ok(Some(selector)) => match crate::apiv1::model::handoff_send_multi(
            node,
            &selector,
            url,
            req.title.as_deref(),
            req.auto_open.unwrap_or(false),
        )
        .await
        {
            Ok(rep) => Ok(json(200, &rep)),
            Err(e) => Err(core_error(e)),
        },
        Ok(None) => {
            let peer = req.peer.clone().unwrap_or_default();
            match crate::apiv1::model::handoff_send(
                node,
                &peer,
                url,
                req.title.as_deref(),
                req.auto_open.unwrap_or(false),
            )
            .await
            {
                Ok(rep) => Ok(json(200, &rep)),
                Err(e) => Err(core_error(e)),
            }
        }
    }
}

/// `POST /handoff/send/multi` (§165): mandatory selector fan-out.
async fn post_handoff_send_multi(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<HandoffSendRequest>(body)?;
    if req.url.is_empty() {
        if let Some(u) = q_str("url") {
            req.url = u;
        }
    }
    if req.title.is_none() {
        req.title = q_str("title");
    }
    if req.auto_open.is_none() {
        req.auto_open = q_str("auto_open").and_then(|s| match s.as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        });
    }
    let url = req.url.trim();
    if url.is_empty() {
        return Err(error(ErrorCode::BadRequest, "url cannot be empty"));
    }
    let is_all = req.all.unwrap_or(false) || qmap.contains_key("all");
    let selector = match crate::store::TargetSelector::from_query(
        req.peer.as_deref(),
        req.tag.as_deref(),
        req.group.as_deref(),
        req.device_type.as_deref(),
        is_all,
    ) {
        Ok(s) => s,
        Err(e) => return Err(error(ErrorCode::BadRequest, e)),
    };
    match crate::apiv1::model::handoff_send_multi(
        node,
        &selector,
        url,
        req.title.as_deref(),
        req.auto_open.unwrap_or(false),
    )
    .await
    {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `GET /handoff/inbox` (§165): list entries with an optional status filter.
fn get_handoff_inbox(node: &Node, query: Option<&str>) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let status_filter = qmap
        .get("status")
        .and_then(|s| s.parse::<tetherly_core::HandoffStatus>().ok());
    Ok(json(
        200,
        &crate::apiv1::model::handoff_inbox(node, status_filter),
    ))
}

/// Resolve an inbox entry id from query `id` or the JSON body (query wins).
fn handoff_action_id(query: Option<&str>, body: &[u8]) -> Result<String, UiResponse> {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let id = qmap.get("id").cloned().or_else(|| {
        parse_body::<HandoffInboxActionRequest>(body)
            .ok()
            .and_then(|b| b.id)
    });
    match id {
        Some(id) => Ok(id),
        None => Err(error(
            ErrorCode::BadRequest,
            "missing required 'id' parameter",
        )),
    }
}

/// `POST /handoff/inbox/open` (§165): open one inbox entry.
async fn post_handoff_inbox_open(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let id = handoff_action_id(query, body)?;
    match crate::apiv1::model::handoff_inbox_open(node, &id) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /handoff/inbox/dismiss` (§165): dismiss one inbox entry.
async fn post_handoff_inbox_dismiss(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let id = handoff_action_id(query, body)?;
    match crate::apiv1::model::handoff_inbox_dismiss(node, &id) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /handoff/inbox` (§165): clear all entries.
async fn delete_handoff_inbox(node: &Node) -> Handled {
    match crate::apiv1::model::handoff_inbox_clear(node) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /focus/state` (§165): set local focus; body wins over legacy query.
async fn post_focus_state(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<FocusSetRequest>(body)?;
    if req.enabled.is_none() {
        req.enabled = q_str("enabled").and_then(|s| s.parse().ok());
    }
    if req.mode.is_none() {
        req.mode = q_str("mode");
    }
    if req.duration_mins.is_none() {
        req.duration_mins = q_str("duration_mins")
            .or_else(|| q_str("duration"))
            .and_then(|s| s.parse().ok());
    }
    if req.reason.is_none() {
        req.reason = q_str("reason");
    }
    if req.broadcast.is_none() {
        req.broadcast = q_str("broadcast").and_then(|s| s.parse().ok());
    }
    let mode = match &req.mode {
        Some(m) => Some(m.parse::<tetherly_core::FocusMode>().map_err(|e| {
            error(
                ErrorCode::BadRequest,
                format!("invalid focus mode '{m}': {e}"),
            )
        })?),
        None => None,
    };
    match crate::apiv1::model::set_focus_state(
        node,
        req.enabled.unwrap_or(true),
        mode,
        req.duration_mins,
        req.reason,
        req.broadcast.unwrap_or(true),
    )
    .await
    {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /focus/sync` (§165): request focus state from eligible peers.
async fn post_focus_sync(node: &Node) -> Handled {
    match crate::apiv1::model::focus_sync(node).await {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /focus/policy` (§165): sparse update; body wins over legacy query
/// aliases (`enable`/`mirror`/`breakthrough`/`quiet_hours`/`start`/`end`,
/// single-value `group`/`tag`/`type` append); `HH:MM` folds to minutes.
async fn patch_focus_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<FocusPolicyPatch>(body)?;
    if patch.enabled.is_none() {
        patch.enabled = q_str("enabled")
            .or_else(|| q_str("enable"))
            .and_then(|s| s.parse().ok());
    }
    if patch.default_mode.is_none() {
        patch.default_mode = q_str("default_mode").or_else(|| q_str("mode"));
    }
    if patch.mirror_inbound_focus.is_none() {
        patch.mirror_inbound_focus = q_str("mirror")
            .or_else(|| q_str("mirror_inbound_focus"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_emergency_breakthrough.is_none() {
        patch.allow_emergency_breakthrough = q_str("breakthrough")
            .or_else(|| q_str("allow_emergency_breakthrough"))
            .and_then(|s| s.parse().ok());
    }
    if patch.quiet_hours_enabled.is_none() {
        patch.quiet_hours_enabled = q_str("quiet_hours")
            .or_else(|| q_str("quiet_hours_enabled"))
            .and_then(|s| s.parse().ok());
    }
    if patch.quiet_hours_start.is_none() {
        patch.quiet_hours_start = q_str("start").or_else(|| q_str("quiet_hours_start"));
    }
    if patch.quiet_hours_end.is_none() {
        patch.quiet_hours_end = q_str("end").or_else(|| q_str("quiet_hours_end"));
    }
    let from_query = !qmap.is_empty();
    if from_query {
        if let Some(s) = q_str("start").or_else(|| q_str("quiet_hours_start")) {
            if let Some(min) = super::dto::parse_hh_mm(&s) {
                patch.quiet_hours_start = None;
                patch.quiet_hours_start_minute = Some(min);
            }
        }
        if let Some(e) = q_str("end").or_else(|| q_str("quiet_hours_end")) {
            if let Some(min) = super::dto::parse_hh_mm(&e) {
                patch.quiet_hours_end = None;
                patch.quiet_hours_end_minute = Some(min);
            }
        }
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_str("group")
            .filter(|g| !g.trim().is_empty())
            .map(|g| vec![g]);
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_str("tag")
            .filter(|t| !t.trim().is_empty())
            .map(|t| vec![t]);
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_str("type")
            .or_else(|| q_str("device_type"))
            .map(|t| vec![t]);
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.focus_policy();
    if let Some(v) = patch.enabled {
        policy.enabled = v;
    }
    if let Some(m) = &patch.default_mode {
        policy.default_mode = m.parse::<tetherly_core::FocusMode>().map_err(|e| {
            error(
                ErrorCode::BadRequest,
                format!("invalid default focus mode '{m}': {e}"),
            )
        })?;
    }
    if let Some(v) = patch.mirror_inbound_focus {
        policy.mirror_inbound_focus = v;
    }
    if let Some(v) = patch.allow_emergency_breakthrough {
        policy.allow_emergency_breakthrough = v;
    }
    if let Some(v) = patch.quiet_hours_enabled {
        policy.quiet_hours_enabled = v;
    }
    if let Some(s) = &patch.quiet_hours_start {
        match super::dto::parse_hh_mm(s) {
            Some(min) => policy.quiet_hours_start_minute = min,
            None => {
                return Err(error(
                    ErrorCode::BadRequest,
                    "invalid quiet_hours_start format (expected HH:MM)",
                ));
            }
        }
    }
    if let Some(min) = patch.quiet_hours_start_minute {
        policy.quiet_hours_start_minute = min % 1440;
    }
    if let Some(e) = &patch.quiet_hours_end {
        match super::dto::parse_hh_mm(e) {
            Some(min) => policy.quiet_hours_end_minute = min,
            None => {
                return Err(error(
                    ErrorCode::BadRequest,
                    "invalid quiet_hours_end format (expected HH:MM)",
                ));
            }
        }
    }
    if let Some(min) = patch.quiet_hours_end_minute {
        policy.quiet_hours_end_minute = min % 1440;
    }
    if let Some(mut g) = patch.allowed_groups {
        if from_query && !policy.allowed_groups.contains(&g[0]) {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query && !policy.allowed_tags.contains(&t[0]) {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    match crate::apiv1::model::set_focus_policy(node, policy) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /camera/attach` (§165): body wins over legacy query aliases.
async fn post_camera_attach(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<CameraAttachRequest>(body)?;
    if req.peer.is_none() {
        req.peer = q_str("peer")
            .or_else(|| q_str("peer_id"))
            .or_else(|| q_str("target"));
    }
    if req.lens_id.is_none() {
        req.lens_id = q_str("lens_id").or_else(|| q_str("lens"));
    }
    if req.width.is_none() {
        req.width = q_str("width").and_then(|s| s.parse().ok());
    }
    if req.height.is_none() {
        req.height = q_str("height").and_then(|s| s.parse().ok());
    }
    if req.fps.is_none() {
        req.fps = q_str("target_fps")
            .or_else(|| q_str("fps"))
            .and_then(|s| s.parse().ok());
    }
    if req.codec.is_none() {
        req.codec = q_str("codec");
    }
    if req.flashlight.is_none() {
        req.flashlight = q_str("flashlight")
            .or_else(|| q_str("torch"))
            .and_then(|s| s.parse().ok());
    }
    if req.zoom.is_none() {
        req.zoom = q_str("zoom").and_then(|s| s.parse().ok());
    }
    let codec = req
        .codec
        .as_deref()
        .map(tetherly_core::CameraCodec::from_str_lenient);
    match crate::apiv1::model::camera_attach(
        node,
        &req.peer.unwrap_or_default(),
        req.lens_id.as_deref(),
        req.width,
        req.height,
        req.fps,
        codec,
        req.flashlight,
        req.zoom,
    )
    .await
    {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /camera/detach` (§165): body wins over legacy query.
async fn post_camera_detach(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let mut req = parse_body::<CameraDetachRequest>(body)?;
    if req.reason.is_none() {
        req.reason = qmap.get("reason").cloned();
    }
    match crate::apiv1::model::camera_detach(node, req.reason.as_deref()).await {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /camera/policy` (§165): sparse update; body wins over legacy
/// query aliases (`enable`/`allow_inbound`/`inbound`/`allow_capture`,
/// single-value `group`/`tag`/`type` append).
async fn patch_camera_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<CameraPolicyPatch>(body)?;
    if patch.enabled.is_none() {
        patch.enabled = q_str("enabled")
            .or_else(|| q_str("enable"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_inbound_stream.is_none() {
        patch.allow_inbound_stream = q_str("allow_inbound_stream")
            .or_else(|| q_str("allow_inbound"))
            .or_else(|| q_str("inbound"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_remote_capture.is_none() {
        patch.allow_remote_capture = q_str("allow_remote_capture")
            .or_else(|| q_str("allow_capture"))
            .or_else(|| q_str("capture"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_resolution_width.is_none() {
        patch.max_resolution_width = q_str("max_resolution_width")
            .or_else(|| q_str("max_width"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_resolution_height.is_none() {
        patch.max_resolution_height = q_str("max_resolution_height")
            .or_else(|| q_str("max_height"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_fps.is_none() {
        patch.max_fps = q_str("max_fps").and_then(|s| s.parse().ok());
    }
    if patch.enforce_privacy_shutter_indicator.is_none() {
        patch.enforce_privacy_shutter_indicator = q_str("enforce_privacy_shutter_indicator")
            .or_else(|| q_str("shutter"))
            .and_then(|s| s.parse().ok());
    }
    if patch.idle_timeout_secs.is_none() {
        patch.idle_timeout_secs = q_str("idle_timeout_secs")
            .or_else(|| q_str("timeout"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_str("group")
            .filter(|g| !g.trim().is_empty())
            .map(|g| vec![g]);
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_str("tag")
            .filter(|t| !t.trim().is_empty())
            .map(|t| vec![t]);
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_str("type")
            .or_else(|| q_str("device_type"))
            .map(|t| vec![t]);
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.camera_policy();
    if let Some(v) = patch.enabled {
        policy.enabled = v;
    }
    if let Some(v) = patch.allow_inbound_stream {
        policy.allow_inbound_stream = v;
    }
    if let Some(v) = patch.allow_remote_capture {
        policy.allow_remote_capture = v;
    }
    if let Some(v) = patch.max_resolution_width {
        policy.max_resolution_width = v;
    }
    if let Some(v) = patch.max_resolution_height {
        policy.max_resolution_height = v;
    }
    if let Some(v) = patch.max_fps {
        policy.max_fps = v;
    }
    if let Some(v) = patch.enforce_privacy_shutter_indicator {
        policy.enforce_privacy_shutter_indicator = v;
    }
    if let Some(v) = patch.idle_timeout_secs {
        policy.idle_timeout_secs = v;
    }
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query && !policy.allowed_groups.contains(&g[0]) {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query && !policy.allowed_tags.contains(&t[0]) {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    match crate::apiv1::model::set_camera_policy(node, policy) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /audio/policy` (§165): sparse update; body wins over legacy query
/// aliases (`enable`/`allow_sink`/`sink`/`allow_outbound`/`routing`/
/// `max_volume`/`volume_limit`/`allow_spatial`/`spatial`/`timeout`/
/// `idle_timeout`, single-value `group`/`tag`/`type` append).
async fn patch_audio_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<AudioPolicyPatch>(body)?;
    if patch.enabled.is_none() {
        patch.enabled = q_str("enabled")
            .or_else(|| q_str("enable"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_inbound_sink.is_none() {
        patch.allow_inbound_sink = q_str("allow_inbound_sink")
            .or_else(|| q_str("allow_sink"))
            .or_else(|| q_str("sink"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_outbound_routing.is_none() {
        patch.allow_outbound_routing = q_str("allow_outbound_routing")
            .or_else(|| q_str("allow_outbound"))
            .or_else(|| q_str("routing"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_volume_limit.is_none() {
        patch.max_volume_limit = q_str("max_volume_limit")
            .or_else(|| q_str("max_volume"))
            .or_else(|| q_str("volume_limit"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_spatial_rendering.is_none() {
        patch.allow_spatial_rendering = q_str("allow_spatial_rendering")
            .or_else(|| q_str("allow_spatial"))
            .or_else(|| q_str("spatial"))
            .and_then(|s| s.parse().ok());
    }
    if patch.idle_timeout_secs.is_none() {
        patch.idle_timeout_secs = q_str("idle_timeout_secs")
            .or_else(|| q_str("timeout"))
            .or_else(|| q_str("idle_timeout"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_str("group")
            .or_else(|| q_str("allowed_group"))
            .filter(|g| !g.trim().is_empty())
            .map(|g| vec![g]);
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_str("tag")
            .or_else(|| q_str("allowed_tag"))
            .filter(|t| !t.trim().is_empty())
            .map(|t| vec![t]);
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_str("type")
            .or_else(|| q_str("device_type"))
            .map(|t| vec![t]);
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.audio_routing_policy();
    if let Some(v) = patch.enabled {
        policy.enabled = v;
    }
    if let Some(v) = patch.allow_inbound_sink {
        policy.allow_inbound_sink = v;
    }
    if let Some(v) = patch.allow_outbound_routing {
        policy.allow_outbound_routing = v;
    }
    if let Some(v) = patch.max_volume_limit {
        policy.max_volume_limit = v.min(100);
    }
    if let Some(v) = patch.allow_spatial_rendering {
        policy.allow_spatial_rendering = v;
    }
    if let Some(v) = patch.idle_timeout_secs {
        policy.idle_timeout_secs = v;
    }
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query && !policy.allowed_groups.contains(&g[0]) {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query && !policy.allowed_tags.contains(&t[0]) {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    match crate::apiv1::model::set_audio_policy(node, policy) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /mic/attach` (§165): body wins over legacy query aliases
/// (`peer`/`peer_id`/`target`, `mic_id`/`mic`).
async fn post_mic_attach(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<MicAttachRequest>(body)?;
    if req.peer.is_none() {
        req.peer = q_str("peer")
            .or_else(|| q_str("peer_id"))
            .or_else(|| q_str("target"));
    }
    if req.mic_id.is_none() {
        req.mic_id = q_str("mic_id").or_else(|| q_str("mic"));
    }
    if req.mode.is_none() {
        req.mode = q_str("mode");
    }
    if req.codec.is_none() {
        req.codec = q_str("codec");
    }
    if req.sample_rate.is_none() {
        req.sample_rate = q_str("sample_rate").and_then(|s| s.parse().ok());
    }
    if req.channels.is_none() {
        req.channels = q_str("channels").and_then(|s| s.parse().ok());
    }
    if req.gain.is_none() {
        req.gain = q_str("gain").and_then(|s| s.parse().ok());
    }
    if req.muted.is_none() {
        req.muted = q_str("muted").and_then(|s| s.parse().ok());
    }
    if req.noise_suppression.is_none() {
        req.noise_suppression = q_str("noise_suppression")
            .or_else(|| q_str("ns"))
            .and_then(|s| s.parse().ok());
    }
    if req.echo_cancellation.is_none() {
        req.echo_cancellation = q_str("echo_cancellation")
            .or_else(|| q_str("aec"))
            .and_then(|s| s.parse().ok());
    }
    if req.auto_gain.is_none() {
        req.auto_gain = q_str("auto_gain")
            .or_else(|| q_str("agc"))
            .and_then(|s| s.parse().ok());
    }
    let mode = req
        .mode
        .as_deref()
        .map(tetherly_core::MicMode::from_str_lenient)
        .unwrap_or(tetherly_core::MicMode::VirtualMic);
    let codec = req
        .codec
        .as_deref()
        .map(tetherly_core::MicCodec::from_str_lenient);
    match crate::apiv1::model::mic_attach(
        node,
        &req.peer.unwrap_or_default(),
        req.mic_id.as_deref(),
        mode,
        codec,
        req.sample_rate,
        req.channels,
        req.gain,
        req.muted,
        req.noise_suppression,
        req.echo_cancellation,
        req.auto_gain,
    )
    .await
    {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /mic/detach` (§165): body wins over legacy query.
async fn post_mic_detach(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let mut req = parse_body::<MicDetachRequest>(body)?;
    if req.reason.is_none() {
        req.reason = qmap.get("reason").cloned();
    }
    match crate::apiv1::model::mic_detach(node, req.reason.as_deref()).await {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /audio/attach` (§165): body wins over legacy query aliases
/// (`peer`/`peer_id`/`target`, `gain`/`volume`/`volume_gain`,
/// `x`/`spatial_x`, `y`/`spatial_y`, `z`/`spatial_z`).
async fn post_audio_attach(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<AudioAttachRequest>(body)?;
    if req.peer.is_none() {
        req.peer = q_str("peer")
            .or_else(|| q_str("peer_id"))
            .or_else(|| q_str("target"));
    }
    if req.layout.is_none() {
        req.layout = q_str("layout");
    }
    if req.sample_rate.is_none() {
        req.sample_rate = q_str("sample_rate").and_then(|s| s.parse().ok());
    }
    if req.channels.is_none() {
        req.channels = q_str("channels").and_then(|s| s.parse().ok());
    }
    // Flatten embedded spatial_position into x/y/z (body wins over query).
    if let Some(pos) = req.spatial_position.take() {
        if req.spatial_x.is_none() {
            req.spatial_x = Some(pos.x);
        }
        if req.spatial_y.is_none() {
            req.spatial_y = Some(pos.y);
        }
        if req.spatial_z.is_none() {
            req.spatial_z = Some(pos.z);
        }
    }
    if req.spatial_x.is_none() {
        req.spatial_x = q_str("x")
            .or_else(|| q_str("spatial_x"))
            .and_then(|s| s.parse().ok());
    }
    if req.spatial_y.is_none() {
        req.spatial_y = q_str("y")
            .or_else(|| q_str("spatial_y"))
            .and_then(|s| s.parse().ok());
    }
    if req.spatial_z.is_none() {
        req.spatial_z = q_str("z")
            .or_else(|| q_str("spatial_z"))
            .and_then(|s| s.parse().ok());
    }
    if req.volume_gain.is_none() {
        req.volume_gain = q_str("gain")
            .or_else(|| q_str("volume"))
            .or_else(|| q_str("volume_gain"))
            .and_then(|s| s.parse().ok());
    }
    let layout = req
        .layout
        .as_deref()
        .map(tetherly_core::AudioChannelLayout::from_str_lenient);
    match crate::apiv1::model::audio_attach(
        node,
        req.peer.as_deref().unwrap_or_default(),
        layout,
        req.sample_rate,
        req.channels,
        req.spatial_x,
        req.spatial_y,
        req.spatial_z,
        req.volume_gain,
    )
    .await
    {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /audio/detach` (§165): body wins over legacy query.
async fn post_audio_detach(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let mut req = parse_body::<AudioDetachRequest>(body)?;
    if req.reason.is_none() {
        req.reason = qmap.get("reason").cloned();
    }
    match crate::apiv1::model::audio_detach(node, req.reason.as_deref()).await {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /locate/ring` (§165): body wins over legacy query aliases
/// (`peer`/`target`, `duration`/`duration_secs`, `override_silent`/`override`).
async fn post_locate_ring(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<LocateRingRequest>(body)?;
    if req.peer.is_none() {
        req.peer = q_str("peer").or_else(|| q_str("target"));
    }
    if req.duration_secs.is_none() {
        req.duration_secs = q_str("duration")
            .or_else(|| q_str("duration_secs"))
            .and_then(|s| s.parse().ok());
    }
    if req.pattern.is_none() {
        req.pattern = q_str("pattern");
    }
    if req.override_silent.is_none() {
        req.override_silent = q_str("override_silent")
            .or_else(|| q_str("override"))
            .and_then(|s| s.parse().ok());
    }
    if req.message.is_none() {
        req.message = q_str("message");
    }
    let Some(peer) = req.peer.filter(|p| !p.trim().is_empty()) else {
        return Err(error(
            ErrorCode::BadRequest,
            "missing target peer (specify 'peer' or 'target')",
        ));
    };
    let pattern = match req.pattern.as_deref() {
        Some(p) => Some(p.parse::<tetherly_core::LocatePattern>().map_err(|e| {
            error(
                ErrorCode::BadRequest,
                format!("invalid locate pattern: {e}"),
            )
        })?),
        None => None,
    };
    match crate::apiv1::model::locate_ring_request(
        node,
        &peer,
        req.duration_secs,
        pattern,
        req.override_silent,
        req.message,
    )
    .await
    {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /locate/stop` (§165): body wins over legacy query aliases
/// (`peer`/`target`, `id`/`request_id`).
async fn post_locate_stop(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<LocateStopRequest>(body)?;
    if req.peer.is_none() {
        req.peer = q_str("peer").or_else(|| q_str("target"));
    }
    if req.request_id.is_none() {
        req.request_id = q_str("id").or_else(|| q_str("request_id"));
    }
    if req.reason.is_none() {
        req.reason = q_str("reason");
    }
    match crate::apiv1::model::locate_stop_request(
        node,
        req.peer.as_deref(),
        req.request_id,
        req.reason,
    )
    .await
    {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /locate/policy` (§165): sparse update; body wins over legacy query
/// aliases (`override_silent`/`override`, `max_duration`, single-value
/// `group`/`tag`/`type` append).
async fn patch_locate_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<LocatePolicyPatch>(body)?;
    if patch.enabled.is_none() {
        patch.enabled = q_str("enabled").and_then(|s| s.parse().ok());
    }
    if patch.allow_override_silent.is_none() {
        patch.allow_override_silent = q_str("allow_override_silent")
            .or_else(|| q_str("override_silent"))
            .or_else(|| q_str("override"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_duration_secs.is_none() {
        patch.max_duration_secs = q_str("max_duration_secs")
            .or_else(|| q_str("max_duration"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_str("group")
            .filter(|g| !g.trim().is_empty())
            .map(|g| vec![g]);
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_str("tag")
            .filter(|t| !t.trim().is_empty())
            .map(|t| vec![t]);
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_str("type")
            .or_else(|| q_str("device_type"))
            .map(|t| vec![t]);
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.locator_policy();
    if let Some(v) = patch.enabled {
        policy.enabled = v;
    }
    if let Some(v) = patch.allow_override_silent {
        policy.allow_override_silent = v;
    }
    if let Some(v) = patch.max_duration_secs {
        policy.max_duration_secs = v.clamp(1, 60);
    }
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query && !policy.allowed_groups.contains(&g[0]) {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query && !policy.allowed_tags.contains(&t[0]) {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    match crate::apiv1::model::set_locate_policy(node, policy) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /display/virtual/attach` (§165): body wins over legacy query aliases
/// (`peer`/`peer_id`/`target`, `fps`/`target_fps`).
async fn post_display_attach(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut req = parse_body::<DisplayAttachRequest>(body)?;
    if req.peer.is_none() {
        req.peer = q_str("peer")
            .or_else(|| q_str("peer_id"))
            .or_else(|| q_str("target"));
    }
    if req.edge.is_none() {
        req.edge = q_str("edge");
    }
    if req.width.is_none() {
        req.width = q_str("width").and_then(|s| s.parse().ok());
    }
    if req.height.is_none() {
        req.height = q_str("height").and_then(|s| s.parse().ok());
    }
    if req.fps.is_none() {
        req.fps = q_str("fps")
            .or_else(|| q_str("target_fps"))
            .and_then(|s| s.parse().ok());
    }
    let edge = match req.edge.as_deref() {
        Some(s) => Some(
            s.parse::<tetherly_core::DisplayEdge>()
                .map_err(|e| error(ErrorCode::BadRequest, format!("invalid edge '{s}': {e}")))?,
        ),
        None => None,
    };
    match crate::apiv1::model::display_attach(
        node,
        req.peer.as_deref().unwrap_or_default(),
        edge,
        req.width,
        req.height,
        req.fps,
    )
    .await
    {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /display/virtual/detach` (§165): body wins over legacy query.
async fn post_display_detach(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let mut req = parse_body::<DisplayDetachRequest>(body)?;
    if req.reason.is_none() {
        req.reason = qmap.get("reason").cloned();
    }
    match crate::apiv1::model::display_detach(node, req.reason.as_deref()).await {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /display/virtual/policy` (§165): sparse update; body wins over
/// legacy query aliases (`enable`/`allow_host`/`hosting`/`allow_extend`/
/// `remote_extend`/`max_width`/`max_height`/`fps`/`timeout`, single-value
/// `group`/`tag`/`type` append).
async fn patch_display_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<DisplayPolicyPatch>(body)?;
    if patch.enabled.is_none() {
        patch.enabled = q_str("enabled")
            .or_else(|| q_str("enable"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_hosting.is_none() {
        patch.allow_hosting = q_str("allow_hosting")
            .or_else(|| q_str("allow_host"))
            .or_else(|| q_str("hosting"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_remote_extend.is_none() {
        patch.allow_remote_extend = q_str("allow_remote_extend")
            .or_else(|| q_str("allow_extend"))
            .or_else(|| q_str("remote_extend"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_resolution_width.is_none() {
        patch.max_resolution_width = q_str("max_resolution_width")
            .or_else(|| q_str("max_width"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_resolution_height.is_none() {
        patch.max_resolution_height = q_str("max_resolution_height")
            .or_else(|| q_str("max_height"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_fps.is_none() {
        patch.max_fps = q_str("max_fps")
            .or_else(|| q_str("fps"))
            .and_then(|s| s.parse().ok());
    }
    if patch.idle_timeout_secs.is_none() {
        patch.idle_timeout_secs = q_str("idle_timeout_secs")
            .or_else(|| q_str("timeout"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_str("group")
            .or_else(|| q_str("allowed_group"))
            .filter(|g| !g.trim().is_empty())
            .map(|g| vec![g]);
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_str("tag")
            .or_else(|| q_str("allowed_tag"))
            .filter(|t| !t.trim().is_empty())
            .map(|t| vec![t]);
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_str("type")
            .or_else(|| q_str("device_type"))
            .map(|t| vec![t]);
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.virtual_display_policy();
    if let Some(v) = patch.enabled {
        policy.enabled = v;
    }
    if let Some(v) = patch.allow_hosting {
        policy.allow_hosting = v;
    }
    if let Some(v) = patch.allow_remote_extend {
        policy.allow_remote_extend = v;
    }
    if let Some(v) = patch.max_resolution_width {
        policy.max_resolution_width = v;
    }
    if let Some(v) = patch.max_resolution_height {
        policy.max_resolution_height = v;
    }
    if let Some(v) = patch.max_fps {
        policy.max_fps = v;
    }
    if let Some(v) = patch.idle_timeout_secs {
        policy.idle_timeout_secs = v;
    }
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query && !policy.allowed_groups.contains(&g[0]) {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query && !policy.allowed_tags.contains(&t[0]) {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    match crate::apiv1::model::set_display_policy(node, policy) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `GET /screen` (§167): full aggregate streaming status.
async fn get_screen_status(node: &Node) -> Handled {
    Ok(json(
        200,
        &crate::apiv1::model::screen_status_of(node.screen_status_report().await),
    ))
}

// ==== §167: screen streaming handlers ====

/// `POST /screen/start` (§167): body `{peer}` optional.
async fn post_screen_start(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenStartRequest>(body)?;
    match node.screen_start_reported(req.peer.as_deref()).await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_control_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /screen/switch` (§167): body `{display_id}` required (0-255).
async fn post_screen_switch(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenSwitchRequest>(body)?;
    let Some(display_id) = req.display_id else {
        return Err(error(
            ErrorCode::BadRequest,
            "missing required field: display_id (expected integer 0-255)",
        ));
    };
    match node.screen_switch_display(display_id).await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_displays_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/privacy` (§167): body `{enabled, lock_input}`; enabled required.
async fn patch_screen_privacy(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenPrivacyPatch>(body)?;
    let Some(enabled) = patch.enabled else {
        return Err(error(
            ErrorCode::BadRequest,
            "missing required field: enabled (expected boolean true/false)",
        ));
    };
    match node
        .screen_set_privacy_mode(enabled, patch.lock_input.unwrap_or(false))
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_privacy_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /screen/recording/start` (§167): body `{path}` optional.
async fn post_screen_recording_start(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenRecordingStartRequest>(body)?;
    match node.screen_start_recording(req.path).await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_recording_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `GET /screen/recording/replay` (§167): `?path=` required, `limit` default 32.
async fn get_screen_recording_replay(node: &Node, query: Option<&str>) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let Some(path) = qmap.get("path").cloned().filter(|p| !p.trim().is_empty()) else {
        return Err(error(
            ErrorCode::BadRequest,
            "recording replay requires a ?path= parameter",
        ));
    };
    let limit = qmap
        .get("limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(32);
    match node.screen_recording_replay(path, limit) {
        Ok(rep) => Ok(json(
            200,
            &crate::apiv1::model::screen_recording_replay_of(rep),
        )),
        Err(e) => Err(core_error(e)),
    }
}

/// `GET|POST /screen/recording/snapshot` (§167): `path` required; `format=bmp`
/// streams binary, otherwise JSON.
async fn screen_recording_snapshot(
    node: &Node,
    query: Option<&str>,
    body: &[u8],
    allow_output: bool,
) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let req = parse_body::<ScreenRecordingSnapshotRequest>(body)?;
    let q_str = |k: &str| qmap.get(k).cloned();
    let Some(path) = req
        .path
        .clone()
        .or_else(|| q_str("path"))
        .filter(|p| !p.trim().is_empty())
    else {
        return Err(error(
            ErrorCode::BadRequest,
            "recording snapshot requires a path parameter or JSON body",
        ));
    };
    let frame = req
        .frame
        .or_else(|| q_str("frame").and_then(|s| s.parse().ok()));
    let pts = req
        .pts
        .or_else(|| q_str("pts").and_then(|s| s.parse().ok()));
    let out = req.out.clone().or_else(|| q_str("out"));
    if out.is_some() && !allow_output {
        return Err(error(
            ErrorCode::Forbidden,
            "recording snapshot output requires POST with an admin token",
        ));
    }
    let mut export_file = if allow_output {
        create_screen_export(node, out.as_deref())?
    } else {
        None
    };
    let format = req.format.clone().or_else(|| q_str("format"));
    if format.as_deref().map(|f| f.eq_ignore_ascii_case("bmp")) == Some(true) {
        let (bytes, mut report) = node
            .screen_recording_snapshot_data(&path, frame, pts)
            .map_err(core_error)?;
        if let Some(file) = export_file.as_mut() {
            use std::io::Write;
            file.write_all(&bytes).map_err(|e| {
                error(
                    ErrorCode::Internal,
                    format!("cannot write recording export: {e}"),
                )
            })?;
            report.path = out;
            report.data_base64 = None;
        }
        Ok(UiResponse::new(200, "image/bmp", bytes))
    } else {
        if export_file.is_some() {
            return Err(error(
                ErrorCode::BadRequest,
                "recording export output requires format=bmp",
            ));
        }
        let rep = node
            .screen_recording_snapshot(&path, frame, pts)
            .map_err(core_error)?;
        Ok(json(200, &crate::apiv1::model::screen_snapshot_of(rep)))
    }
}

/// `POST /screen/playback/start` (§167): body `{path, speed, loop}`; path required.
async fn post_screen_playback_start(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenPlaybackStartRequest>(body)?;
    let Some(path) = req.path.clone().filter(|p| !p.trim().is_empty()) else {
        return Err(error(
            ErrorCode::BadRequest,
            "playback requires a path parameter or JSON body",
        ));
    };
    match node
        .screen_playback_start(path, req.speed, req.loop_enabled)
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_playback_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

fn create_screen_export(
    node: &Node,
    out: Option<&str>,
) -> Result<Option<std::fs::File>, UiResponse> {
    let Some(raw) = out.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if raw.contains('/') || raw.contains('\\') {
        return Err(error(
            ErrorCode::BadRequest,
            "export output must be a filename",
        ));
    }
    let path = std::path::Path::new(raw);
    let mut components = path.components();
    let is_single_filename = matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    if path.is_absolute() || !is_single_filename || raw == "." || raw == ".." {
        return Err(error(
            ErrorCode::BadRequest,
            "export output must be a safe filename",
        ));
    }
    let data_dir = std::fs::canonicalize(node.data_dir()).map_err(|e| {
        error(
            ErrorCode::Internal,
            format!("cannot resolve node data directory: {e}"),
        )
    })?;
    let dir = node.data_dir().join("exports");
    let canonical_dir = crate::store::ensure_private_dir(&dir).map_err(|e| {
        error(
            ErrorCode::Forbidden,
            format!("export directory is not private: {e}"),
        )
    })?;
    if !canonical_dir.starts_with(&data_dir) {
        return Err(error(
            ErrorCode::Forbidden,
            "export directory is outside the node data directory",
        ));
    }
    let destination = canonical_dir.join(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&destination).map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            error(ErrorCode::Conflict, "export output already exists")
        } else {
            error(
                ErrorCode::Internal,
                format!("cannot create export output: {e}"),
            )
        }
    })?;
    Ok(Some(file))
}

/// `GET /screen/snapshot` (§167): `?format=` only; no frame → 503.
async fn get_screen_snapshot(node: &Node, query: Option<&str>) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    if qmap.get("out").is_some_and(|out| !out.trim().is_empty()) {
        return Err(error(
            ErrorCode::Forbidden,
            "screen snapshot output requires a POST export request",
        ));
    }
    let format = qmap.get("format").cloned();
    if format.as_deref().map(|f| f.eq_ignore_ascii_case("bmp")) == Some(true) {
        match node.screen_snapshot_data(Some("bmp".to_string())) {
            Ok((bytes, _)) => Ok(UiResponse::new(200, "image/bmp", bytes)),
            Err(e) => Err(core_error(e)),
        }
    } else {
        match node.screen_snapshot(format) {
            Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_snapshot_of(rep))),
            Err(e) => Err(core_error(e)),
        }
    }
}

/// `POST /screen/snapshot`: body `{format, out}`. The output path is passed to
/// the runtime export guard; GET remains side-effect free.
async fn post_screen_snapshot(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenSnapshotRequest>(body)?;
    let Some(out) = req.out.filter(|path| !path.trim().is_empty()) else {
        return Err(error(
            ErrorCode::BadRequest,
            "screen snapshot export requires out",
        ));
    };
    let format = req.format.unwrap_or_else(|| "bmp".to_string());
    if !matches!(format.to_ascii_lowercase().as_str(), "bmp" | "raw") {
        return Err(error(
            ErrorCode::BadRequest,
            "screen snapshot export format must be bmp or raw",
        ));
    }
    let mut file = create_screen_export(node, Some(&out))?
        .ok_or_else(|| error(ErrorCode::BadRequest, "screen snapshot export requires out"))?;
    let (bytes, report) = node
        .screen_snapshot_data(Some(format))
        .map_err(core_error)?;
    use std::io::Write;
    file.write_all(&bytes).map_err(|e| {
        error(
            ErrorCode::Internal,
            format!("cannot write screen snapshot export: {e}"),
        )
    })?;
    let mut report = crate::apiv1::model::screen_snapshot_of(report);
    report.path = Some(out);
    report.data_base64 = None;
    Ok(json(200, &report))
}

fn screen_laser_report_of(node: &Node) -> ScreenLaserReport {
    let rep = node.screen_annotation_report();
    ScreenLaserReport {
        laser_active: rep.laser_active,
        laser: rep.laser.map(crate::apiv1::model::laser_pointer_dto_of),
        laser_updates_total: rep.laser_updates_total,
        timestamp_ms: rep.timestamp_ms,
    }
}

/// `POST /screen/laser` (§167): body `{x, y, active, color, color_rgb, radius}`.
async fn post_screen_laser(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenLaserPatch>(body)?;
    let color_rgb = patch.color_rgb.unwrap_or_else(|| {
        patch
            .color
            .as_deref()
            .and_then(parse_hex_rgb)
            .unwrap_or([255, 30, 30])
    });
    let state = tetherly_core::LaserPointerState {
        x: patch.x.unwrap_or(0),
        y: patch.y.unwrap_or(0),
        active: patch.active.unwrap_or(true),
        color_rgb,
        radius: patch.radius.unwrap_or(12),
    };
    node.screen_set_laser_pointer(state)
        .await
        .map_err(core_error)?;
    Ok(json(200, &screen_laser_report_of(node)))
}

/// Strips `#` and parses exactly six hex digits into an RGB triple.
fn parse_hex_rgb(s: &str) -> Option<[u8; 3]> {
    let hex = s.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some([r, g, b])
}

/// Parses a legacy `"x,y"` coordinate string.
fn parse_coord_str(s: &str) -> Option<(u16, u16)> {
    let mut it = s.trim().split(',');
    let x = it.next()?.trim().parse().ok()?;
    let y = it.next()?.trim().parse().ok()?;
    Some((x, y))
}

fn parse_points_str(s: &str) -> Vec<(u16, u16)> {
    s.split(';')
        .filter_map(|pair| {
            let mut it = pair.split(',');
            let x = it.next()?.trim().parse().ok()?;
            let y = it.next()?.trim().parse().ok()?;
            Some((x, y))
        })
        .collect()
}

/// `POST /screen/annotation` (§167): builds an `AnnotationItem` with legacy
/// kind dispatch (`stroke|polyline|line`, `rect|box`, `arrow`, `text|note`).
async fn post_screen_annotation(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenAnnotationCreateRequest>(body)?;
    let kind_str = req
        .kind
        .clone()
        .unwrap_or_else(|| "stroke".to_string())
        .to_lowercase();
    let points = req.points.clone().unwrap_or_else(|| {
        req.points_str
            .as_deref()
            .map(parse_points_str)
            .unwrap_or_default()
    });
    let color_rgb = req
        .color_rgb
        .or_else(|| req.color.as_deref().and_then(parse_hex_rgb))
        .unwrap_or([255, 0, 0]);
    let kind = match kind_str.as_str() {
        "stroke" | "polyline" | "line" => {
            if points.is_empty() {
                return Err(error(
                    ErrorCode::BadRequest,
                    "stroke annotation requires non-empty points",
                ));
            }
            tetherly_core::screen::AnnotationKind::Stroke { points }
        }
        "rect" | "box" => {
            let w = req.w.unwrap_or(0);
            let h = req.h.unwrap_or(0);
            if w == 0 || h == 0 {
                return Err(error(
                    ErrorCode::BadRequest,
                    "rect annotation requires non-zero width and height",
                ));
            }
            tetherly_core::screen::AnnotationKind::Rect {
                x: req.x.unwrap_or(0),
                y: req.y.unwrap_or(0),
                w,
                h,
            }
        }
        "arrow" => {
            let from = req
                .from
                .or_else(|| req.from_str.as_deref().and_then(parse_coord_str))
                .unwrap_or((0, 0));
            let to = req
                .to
                .or_else(|| req.to_str.as_deref().and_then(parse_coord_str))
                .unwrap_or((0, 0));
            if from == to {
                return Err(error(
                    ErrorCode::BadRequest,
                    "arrow requires distinct from and to points",
                ));
            }
            tetherly_core::screen::AnnotationKind::Arrow { from, to }
        }
        "text" | "note" => {
            let text = req.text.clone().unwrap_or_default();
            if text.trim().is_empty() {
                return Err(error(
                    ErrorCode::BadRequest,
                    "text annotation requires non-empty text",
                ));
            }
            tetherly_core::screen::AnnotationKind::Text {
                x: req.x.unwrap_or(0),
                y: req.y.unwrap_or(0),
                text,
                scale: req.scale.unwrap_or(1),
            }
        }
        other => {
            return Err(error(
                ErrorCode::BadRequest,
                format!("unsupported annotation kind: {other}"),
            ));
        }
    };
    let item = tetherly_core::AnnotationItem {
        id: 0,
        author: req.author.unwrap_or_default(),
        kind,
        color_rgb,
        thickness: req.thickness.unwrap_or(3),
        alpha: req.alpha.unwrap_or(255),
        ttl_ms: req.ttl_ms.unwrap_or(0),
        created_at_ms: 0,
    };
    match node.screen_add_annotation(item).await {
        Ok(id) => {
            let rep = node.screen_annotation_report();
            Ok(json(
                200,
                &ScreenAnnotationAdded {
                    id,
                    report: crate::apiv1::model::screen_annotations_of(rep),
                },
            ))
        }
        Err(e) => Err(core_error(e)),
    }
}

fn screen_annotations_now(node: &Node) -> ScreenAnnotations {
    crate::apiv1::model::screen_annotations_of(node.screen_annotation_report())
}

/// `POST /screen/whiteboard/undo` (§167): body `{author}` optional.
async fn post_screen_whiteboard_undo(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenAuthorRequest>(body)?;
    match node.screen_whiteboard_undo(req.author.as_deref()).await {
        Ok(undone) => Ok(json(
            200,
            &ScreenWhiteboardUndoReport {
                undone: undone.map(crate::apiv1::model::annotation_item_dto),
                report: screen_annotations_now(node),
            },
        )),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /screen/whiteboard/redo` (§167): body `{author}` optional.
async fn post_screen_whiteboard_redo(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenAuthorRequest>(body)?;
    match node.screen_whiteboard_redo(req.author.as_deref()).await {
        Ok(redone) => Ok(json(
            200,
            &ScreenWhiteboardRedoReport {
                redone: redone.map(crate::apiv1::model::annotation_item_dto),
                report: screen_annotations_now(node),
            },
        )),
        Err(e) => Err(core_error(e)),
    }
}

/// `GET /screen/whiteboard/export` (§167): pure snapshot only.
async fn get_screen_whiteboard_export(node: &Node, query: Option<&str>) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    if qmap.get("out").is_some_and(|out| !out.trim().is_empty()) {
        return Err(error(
            ErrorCode::Forbidden,
            "whiteboard export output requires a POST export request",
        ));
    }
    match node.screen_whiteboard_export() {
        Ok(snapshot) => Ok(json(
            200,
            &ScreenWhiteboardExportReport {
                export: crate::apiv1::model::whiteboard_snapshot_dto_of(snapshot),
                report: screen_annotations_now(node),
            },
        )),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /screen/whiteboard/export`: body `{out}`. The handler reserves the
/// destination atomically before serializing into its open file handle.
async fn post_screen_whiteboard_export(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenWhiteboardExportRequest>(body)?;
    let Some(out) = req.out.filter(|path| !path.trim().is_empty()) else {
        return Err(error(
            ErrorCode::BadRequest,
            "whiteboard export requires out",
        ));
    };
    let mut file = create_screen_export(node, Some(&out))?
        .ok_or_else(|| error(ErrorCode::BadRequest, "whiteboard export requires out"))?;
    let snapshot = node.screen_whiteboard_export().map_err(core_error)?;
    let dto = crate::apiv1::model::whiteboard_snapshot_dto_of(snapshot);
    let bytes = serde_json::to_vec_pretty(&dto)
        .map_err(|e| error(ErrorCode::Internal, format!("cannot encode export: {e}")))?;
    use std::io::Write;
    file.write_all(&bytes)
        .map_err(|e| error(ErrorCode::Internal, format!("cannot write export: {e}")))?;
    Ok(json(
        200,
        &ScreenWhiteboardExportReport {
            export: dto,
            report: screen_annotations_now(node),
        },
    ))
}

/// `POST /screen/whiteboard/import` (§167): body `{snapshot|items|file, replace}`.
async fn post_screen_whiteboard_import(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenWhiteboardImportRequest>(body)?;
    let snapshot = if let Some(snap) = req.snapshot {
        crate::apiv1::model::whiteboard_snapshot_runtime(snap)
    } else if let Some(items) = req.items {
        tetherly_core::WhiteboardSnapshot {
            version: 1,
            created_at_ms: 0,
            session_id: "import".to_string(),
            active_items: items
                .into_iter()
                .map(crate::apiv1::model::annotation_item_runtime)
                .collect(),
            total_added_counter: 0,
        }
    } else if let Some(file) = req.file {
        // Validate BEFORE reading: an absolute path or traversal must never
        // touch the filesystem (a prior version read first and checked after).
        if file.contains("..") || std::path::Path::new(&file).is_absolute() {
            return Err(error(ErrorCode::BadRequest, "path traversal rejected"));
        }
        let raw = std::fs::read(&file)
            .map_err(|e| error(ErrorCode::BadRequest, format!("failed to read {file}: {e}")))?;
        let dto: tetherly_api::WhiteboardSnapshotDto = serde_json::from_slice(&raw)
            .map_err(|e| error(ErrorCode::BadRequest, format!("bad snapshot json: {e}")))?;
        crate::apiv1::model::whiteboard_snapshot_runtime(dto)
    } else {
        return Err(error(
            ErrorCode::BadRequest,
            "missing snapshot, items or file",
        ));
    };
    match node
        .screen_whiteboard_import(snapshot, req.replace.unwrap_or(false))
        .await
    {
        Ok(count) => Ok(json(
            200,
            &ScreenWhiteboardImportReport {
                imported_count: count,
                report: screen_annotations_now(node),
            },
        )),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/whiteboard/policy` (§167): sparse; absent fields keep current.
async fn patch_screen_whiteboard_policy(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenWhiteboardPolicyPatch>(body)?;
    let cur = node.screen_whiteboard_get_policy();
    let policy = tetherly_core::WhiteboardPolicy {
        read_only: patch.read_only.unwrap_or(cur.read_only),
        allow_peer_clear: patch.allow_peer_clear.unwrap_or(cur.allow_peer_clear),
        max_items: patch.max_items.unwrap_or(cur.max_items),
    };
    node.screen_whiteboard_set_policy(policy)
        .await
        .map_err(core_error)?;
    Ok(json(
        200,
        &ScreenWhiteboardPolicyView {
            policy: crate::apiv1::model::whiteboard_policy_dto_of(
                node.screen_whiteboard_get_policy(),
            ),
        },
    ))
}

/// `PATCH /screen/watermark` (§167): sparse `ScreenWatermarkUpdate`.
async fn patch_screen_watermark(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenWatermarkPatch>(body)?;
    let update = crate::runtime::ScreenWatermarkUpdate {
        enabled: patch.enabled,
        host_enforced: patch.host_enforced,
        text_template: patch.text,
        opacity: patch.opacity,
        color_rgb: patch
            .color_rgb
            .or_else(|| patch.color.as_deref().and_then(parse_hex_rgb)),
        scale: patch.scale,
        pattern: patch.pattern,
        spacing_x: patch.spacing_x,
        spacing_y: patch.spacing_y,
    };
    match node.screen_set_watermark(update).await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_watermark_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /screen/watermark/enable` (§167): body `{host_enforced}` default false.
async fn post_screen_watermark_enable(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenWatermarkEnableRequest>(body)?;
    match node
        .screen_enable_watermark(req.host_enforced.unwrap_or(false))
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_watermark_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

// ==== §167: screen mode-group PATCH handlers ====

/// Uniform mode parse for `FromStr` enums (invalid → 400).
fn parse_screen_mode<T: std::str::FromStr>(s: &str, what: &str) -> Result<T, UiResponse>
where
    T::Err: std::fmt::Display,
{
    match s.parse::<T>() {
        Ok(m) => Ok(m),
        Err(e) => Err(error(
            ErrorCode::BadRequest,
            format!("invalid {what} mode: {e}"),
        )),
    }
}

/// `PATCH /screen/cursor` (§167): body `{mode}`; absent → current report.
async fn patch_screen_cursor(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenCursorPatch>(body)?;
    let Some(mode_s) = patch.mode else {
        return Ok(json(
            200,
            &crate::apiv1::model::screen_cursor_of(node.screen_cursor_report()),
        ));
    };
    let Some(mode) = tetherly_core::ScreenCursorMode::parse(&mode_s) else {
        return Err(error(
            ErrorCode::BadRequest,
            "invalid cursor mode: expected decoupled, embedded, or hidden",
        ));
    };
    let rep = node.screen_set_cursor_mode(mode);
    Ok(json(200, &crate::apiv1::model::screen_cursor_of(rep)))
}

/// `PATCH /screen/trendline` (§167): body `{mode}`; absent → current report.
async fn patch_screen_trendline(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTrendlinePatch>(body)?;
    let Some(mode_s) = patch.mode else {
        return Ok(json(
            200,
            &crate::apiv1::model::screen_trendline_of(node.screen_trendline_report()),
        ));
    };
    let mode = parse_screen_mode::<tetherly_core::ScreenTrendlineMode>(&mode_s, "trendline")?;
    let rep = node.screen_set_trendline_mode(mode);
    Ok(json(200, &crate::apiv1::model::screen_trendline_of(rep)))
}

/// `PATCH /screen/squelch` (§167): body `{mode}`; absent → current report.
async fn patch_screen_squelch(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenSquelchPatch>(body)?;
    let Some(mode_s) = patch.mode else {
        return Ok(json(
            200,
            &crate::apiv1::model::screen_squelch_of(node.screen_squelch_report()),
        ));
    };
    let mode = parse_screen_mode::<tetherly_core::ScreenSquelchMode>(&mode_s, "squelch")?;
    let rep = node.screen_set_squelch_mode(mode);
    Ok(json(200, &crate::apiv1::model::screen_squelch_of(rep)))
}

/// `PATCH /screen/fec` (§167): body `{mode}`; absent → current report.
async fn patch_screen_fec(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenFecPatch>(body)?;
    let Some(mode_s) = patch.mode else {
        return Ok(json(
            200,
            &crate::apiv1::model::screen_fec_of(node.screen_fec_report()),
        ));
    };
    let mode = parse_screen_mode::<tetherly_core::ScreenFecMode>(&mode_s, "fec")?;
    let rep = node.screen_set_fec_mode(mode);
    Ok(json(200, &crate::apiv1::model::screen_fec_of(rep)))
}

/// `PATCH /screen/pacer` (§167): body `{mode}`; absent → current report.
async fn patch_screen_pacer(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenPacerPatch>(body)?;
    let Some(mode_s) = patch.mode else {
        return Ok(json(
            200,
            &crate::apiv1::model::screen_pacer_of(node.screen_pacer_report()),
        ));
    };
    let mode = parse_screen_mode::<tetherly_core::ScreenPacerMode>(&mode_s, "pacer")?;
    let rep = node.screen_set_pacer_mode(mode);
    Ok(json(200, &crate::apiv1::model::screen_pacer_of(rep)))
}

/// `PATCH /screen/harq` (§167): body `{mode}`; absent → current report.
async fn patch_screen_harq(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenHarqPatch>(body)?;
    let Some(mode_s) = patch.mode else {
        return Ok(json(
            200,
            &crate::apiv1::model::screen_harq_of(node.screen_harq_report()),
        ));
    };
    let mode = parse_screen_mode::<tetherly_core::ScreenHarqMode>(&mode_s, "harq")?;
    let rep = node.screen_set_harq_mode(mode);
    Ok(json(200, &crate::apiv1::model::screen_harq_of(rep)))
}

/// `PATCH /screen/twcc` (§167): body `{mode}`; absent → current report.
async fn patch_screen_twcc(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTwccPatch>(body)?;
    let Some(mode_s) = patch.mode else {
        return Ok(json(
            200,
            &crate::apiv1::model::screen_twcc_of(node.screen_twcc_report()),
        ));
    };
    let mode = parse_screen_mode::<tetherly_core::ScreenTwccMode>(&mode_s, "twcc")?;
    let rep = node.screen_set_twcc_mode(mode);
    Ok(json(200, &crate::apiv1::model::screen_twcc_of(rep)))
}

/// `PATCH /screen/sync` (§167): body `{mode, trigger}`; absent mode → report.
async fn patch_screen_sync(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenSyncPatch>(body)?;
    let mode_opt = patch.mode.clone();
    if let Some(mode_s) = mode_opt {
        let mode = parse_screen_mode::<tetherly_core::ScreenAudioSyncMode>(&mode_s, "sync")?;
        node.screen_set_sync_mode(mode);
        return Ok(json(
            200,
            &crate::apiv1::model::screen_sync_of(node.screen_sync_report()),
        ));
    }
    if patch.trigger == Some(true) {
        let rep = node.trigger_screen_sync().await.map_err(core_error)?;
        return Ok(json(200, &crate::apiv1::model::screen_sync_of(rep)));
    }
    Ok(json(
        200,
        &crate::apiv1::model::screen_sync_of(node.screen_sync_report()),
    ))
}

/// `POST /screen/sync/trigger` (§167).
async fn post_screen_sync_trigger(node: &Node) -> Handled {
    match node.trigger_screen_sync().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_sync_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/latency` (§167): body `{mode, reset}`; absent mode → report.
async fn patch_screen_latency(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenLatencyPatch>(body)?;
    if let Some(mode_s) = patch.mode {
        let mode =
            parse_screen_mode::<tetherly_core::ScreenProfilerMode>(&mode_s, "latency profiler")?;
        node.screen_set_latency_mode(mode);
    }
    if patch.reset == Some(true) {
        let rep = node.screen_reset_latency().await.map_err(core_error)?;
        return Ok(json(200, &crate::apiv1::model::screen_latency_of(rep)));
    }
    Ok(json(
        200,
        &crate::apiv1::model::screen_latency_of(node.screen_latency_report()),
    ))
}

/// `DELETE /screen/latency` (§167): reset the profiler.
async fn delete_screen_latency(node: &Node) -> Handled {
    match node.screen_reset_latency().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_latency_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tuning` (§167): body `{mode, reset}`; absent mode → report.
async fn patch_screen_tuning(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTuningPatch>(body)?;
    if let Some(mode_s) = patch.mode {
        let mode = parse_screen_mode::<tetherly_core::ScreenTunerMode>(&mode_s, "tuning")?;
        let rep = node.screen_set_tuning_mode(mode).await;
        if patch.reset == Some(true) {
            let rep = node.screen_reset_tuning().await.map_err(core_error)?;
            return Ok(json(200, &crate::apiv1::model::screen_tuning_of(rep)));
        }
        return Ok(json(200, &crate::apiv1::model::screen_tuning_of(rep)));
    }
    if patch.reset == Some(true) {
        let rep = node.screen_reset_tuning().await.map_err(core_error)?;
        return Ok(json(200, &crate::apiv1::model::screen_tuning_of(rep)));
    }
    Ok(json(
        200,
        &crate::apiv1::model::screen_tuning_of(node.screen_tuning_report()),
    ))
}

/// `DELETE /screen/tuning` (§167): reset the autonomous tuner.
async fn delete_screen_tuning(node: &Node) -> Handled {
    match node.screen_reset_tuning().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tuning_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/playout` (§167): body `{mode}`; absent → current report.
async fn patch_screen_playout(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenPlayoutPatch>(body)?;
    let Some(mode_s) = patch.mode else {
        return Ok(json(
            200,
            &crate::apiv1::model::screen_playout_of(node.screen_playout_report()),
        ));
    };
    let Some(mode) = tetherly_core::ScreenPlayoutMode::parse(&mode_s) else {
        return Err(error(
            ErrorCode::BadRequest,
            "invalid playout mode: expected adaptive, zerolatency, or smooth",
        ));
    };
    let rep = node.screen_set_playout_mode(mode);
    Ok(json(200, &crate::apiv1::model::screen_playout_of(rep)))
}

/// `PATCH /screen/matrix` (§167): body `{mode}`; absent → current report.
async fn patch_screen_matrix(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenMatrixModePatch>(body)?;
    let Some(mode_s) = patch.mode else {
        return Ok(json(
            200,
            &crate::apiv1::model::screen_matrix_of(node.screen_matrix_report()),
        ));
    };
    let Some(mode) = tetherly_core::ScreenMatrixMode::from_str_loose(&mode_s) else {
        return Err(error(
            ErrorCode::BadRequest,
            "invalid matrix mode: expected auto, conservative, or manual",
        ));
    };
    let rep = node.screen_set_matrix_mode(mode);
    Ok(json(200, &crate::apiv1::model::screen_matrix_of(rep)))
}

/// `PATCH /screen/color` (§167): body `{mode, threshold}`; mode defaults `full`.
async fn patch_screen_color(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenColorPatch>(body)?;
    let mode_s = patch.mode.unwrap_or_else(|| "full".to_string());
    let mode = parse_screen_mode::<tetherly_core::ScreenColorMode>(&mode_s, "color")?;
    match node.screen_set_color_mode(mode, patch.threshold).await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_color_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/color` (§167): reset to full color.
async fn delete_screen_color(node: &Node) -> Handled {
    match node.screen_reset_color_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_color_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/delta` (§167): body `{enabled, keyframe_interval}`; enabled default true.
async fn patch_screen_delta(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenDeltaPatch>(body)?;
    match node
        .screen_set_delta_mode(patch.enabled.unwrap_or(true), patch.keyframe_interval)
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_delta_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/delta` (§167): reset delta streaming.
async fn delete_screen_delta(node: &Node) -> Handled {
    match node.screen_reset_delta_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_delta_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/metric` (§167): body `{enabled, sample_interval}`; enabled default true.
async fn patch_screen_metric(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenQualityMetricPatch>(body)?;
    match node
        .screen_set_quality_metric_mode(patch.enabled.unwrap_or(true), patch.sample_interval)
        .await
    {
        Ok(rep) => Ok(json(
            200,
            &crate::apiv1::model::screen_quality_metric_of(rep),
        )),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/metric` (§167): reset the PSNR/SSIM sampler.
async fn delete_screen_metric(node: &Node) -> Handled {
    match node.screen_reset_quality_metric_mode().await {
        Ok(rep) => Ok(json(
            200,
            &crate::apiv1::model::screen_quality_metric_of(rep),
        )),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/qoe` (§167): body `{mode, target_psnr_db, target_ssim_pct}`; mode default `balanced`.
async fn patch_screen_qoe(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenQoePatch>(body)?;
    let mode_s = patch.mode.unwrap_or_else(|| "balanced".to_string());
    let mode = parse_screen_mode::<tetherly_core::ScreenQoeMode>(&mode_s, "qoe")?;
    match node
        .screen_set_qoe_mode(mode, patch.target_psnr_db, patch.target_ssim_pct)
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_qoe_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/qoe` (§167): reset QoE governor.
async fn delete_screen_qoe(node: &Node) -> Handled {
    match node.screen_reset_qoe_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_qoe_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/pir` (§167): body `{mode, bands}`; mode default `continuous`.
async fn patch_screen_pir(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenPirPatch>(body)?;
    let mode_s = patch.mode.unwrap_or_else(|| "continuous".to_string());
    let mode = parse_screen_mode::<tetherly_core::ScreenPirMode>(&mode_s, "pir")?;
    match node.screen_set_pir_mode(mode, patch.bands).await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_pir_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/pir` (§167): reset PIR.
async fn delete_screen_pir(node: &Node) -> Handled {
    match node.screen_reset_pir_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_pir_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/motion` (§167): body `{enabled, max_displacement}`; defaults true / 64.
async fn patch_screen_motion(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenMotionPatch>(body)?;
    match node
        .screen_set_motion_mode(
            patch.enabled.unwrap_or(true),
            patch
                .max_displacement
                .unwrap_or(tetherly_core::SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT),
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_motion_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/motion` (§167): reset frame-motion pipeline.
async fn delete_screen_motion(node: &Node) -> Handled {
    match node.screen_reset_motion_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_motion_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/roi` (§167): body `{x, y, w, h, zoom_percent}` (legacy defaults 0).
async fn patch_screen_roi(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenRoiPatch>(body)?;
    match node
        .screen_set_roi(
            patch.x.unwrap_or(0),
            patch.y.unwrap_or(0),
            patch.w.unwrap_or(0),
            patch.h.unwrap_or(0),
            patch.zoom_percent,
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_roi_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/roi` (§167): reset ROI viewport.
async fn delete_screen_roi(node: &Node) -> Handled {
    match node.screen_reset_roi().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_roi_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/focus` (§167): body `{enabled, mode, radius, peripheral_skip_ratio, box_*}`.
async fn patch_screen_focus(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenFocusPatch>(body)?;
    let mode = match patch.mode.as_deref() {
        Some(s) => tetherly_core::ScreenFocusMode::from_str_loose(s),
        None => None,
    };
    let box_coords = match (patch.box_x, patch.box_y, patch.box_w, patch.box_h) {
        (Some(x), Some(y), Some(w), Some(h)) => Some((x, y, w, h)),
        _ => None,
    };
    match node
        .screen_set_focus_mode(
            patch.enabled.unwrap_or(true),
            mode,
            patch.radius,
            patch.peripheral_skip_ratio,
            box_coords,
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_focus_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/focus` (§167): reset focus region.
async fn delete_screen_focus(node: &Node) -> Handled {
    match node.screen_reset_focus_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_focus_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tile` (§167): body `{enabled, tile_size}`; defaults true / 64.
async fn patch_screen_tile(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTilePatch>(body)?;
    match node
        .screen_set_tile_mode(
            patch.enabled.unwrap_or(true),
            patch
                .tile_size
                .unwrap_or(tetherly_core::SCREEN_TILE_DEFAULT_SIZE),
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/tile` (§167): reset tile mode.
async fn delete_screen_tile(node: &Node) -> Handled {
    match node.screen_reset_tile_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tile/cache` (§167): body `{enabled, max_entries}`; defaults true / 512.
async fn patch_screen_tile_cache(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTileCachePatch>(body)?;
    match node
        .screen_set_tile_cache_mode(
            patch.enabled.unwrap_or(true),
            patch
                .max_entries
                .unwrap_or(tetherly_core::SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES),
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_cache_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/tile/cache` (§167): reset tile cache.
async fn delete_screen_tile_cache(node: &Node) -> Handled {
    match node.screen_reset_tile_cache_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_cache_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tile/palette` (§167): body `{enabled, max_colors, enable_rle}`.
async fn patch_screen_tile_palette(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTilePalettePatch>(body)?;
    match node
        .screen_set_tile_palette_mode(
            patch.enabled.unwrap_or(true),
            patch.max_colors,
            patch.enable_rle,
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_palette_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/tile/palette` (§167): reset palette compression.
async fn delete_screen_tile_palette(node: &Node) -> Handled {
    match node.screen_reset_tile_palette_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_palette_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tile/predict` (§167): body `{enabled, filter_mode, enable_rct}`.
async fn patch_screen_tile_predict(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTilePredictPatch>(body)?;
    let filter_mode = patch
        .filter_mode
        .as_deref()
        .and_then(tetherly_core::ScreenTilePredictFilter::from_str_loose);
    match node
        .screen_set_tile_predict_mode(patch.enabled.unwrap_or(true), filter_mode, patch.enable_rct)
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_predict_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/tile/predict` (§167): reset predictive filter.
async fn delete_screen_tile_predict(node: &Node) -> Handled {
    match node.screen_reset_tile_predict_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_predict_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tile/present` (§167): body `{enabled, vsync_mode, double_buffer, tear_prevention}`.
async fn patch_screen_tile_present(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTilePresentPatch>(body)?;
    let vsync_mode = patch
        .vsync_mode
        .as_deref()
        .and_then(tetherly_core::ScreenTilePresentVsyncMode::from_str_loose);
    match node
        .screen_set_tile_present_mode(
            patch.enabled.unwrap_or(true),
            vsync_mode,
            patch.double_buffer,
            patch.tear_prevention,
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_present_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/tile/present` (§167): reset present pipeline.
async fn delete_screen_tile_present(node: &Node) -> Handled {
    match node.screen_reset_tile_present_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_present_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tile/nack` (§167): body `{enabled, timeout_ms, max_retries}`.
async fn patch_screen_tile_nack(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTileNackPatch>(body)?;
    match node
        .screen_set_tile_nack_mode(
            patch.enabled.unwrap_or(true),
            patch.timeout_ms,
            patch.max_retries,
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_nack_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/tile/nack` (§167): reset tile NACK.
async fn delete_screen_tile_nack(node: &Node) -> Handled {
    match node.screen_reset_tile_nack_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_nack_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tile/saliency` (§167): body `{enabled, high_threshold, low_threshold, drop_background_on_congestion}`.
async fn patch_screen_tile_saliency(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTileSaliencyPatch>(body)?;
    match node
        .screen_set_tile_saliency_mode(
            patch.enabled.unwrap_or(true),
            patch.high_threshold,
            patch.low_threshold,
            patch.drop_background_on_congestion,
        )
        .await
    {
        Ok(rep) => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_saliency_of(rep),
        )),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/tile/saliency` (§167): reset saliency tiering.
async fn delete_screen_tile_saliency(node: &Node) -> Handled {
    match node.screen_reset_tile_saliency_mode().await {
        Ok(rep) => Ok(json(
            200,
            &crate::apiv1::model::screen_tile_saliency_of(rep),
        )),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tile/lod` (§167): body `{enabled, max_lod, auto_lod_on_congestion}`.
async fn patch_screen_tile_lod(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTileLodPatch>(body)?;
    match node
        .screen_set_tile_lod_mode(
            patch.enabled.unwrap_or(true),
            patch.max_lod,
            patch.auto_lod_on_congestion,
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_lod_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/tile/lod` (§167): reset tile LOD.
async fn delete_screen_tile_lod(node: &Node) -> Handled {
    match node.screen_reset_tile_lod_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_lod_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tile/batch` (§167): body `{enabled, target_chunk_bytes, pacing_interval_us}`.
async fn patch_screen_tile_batch(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTileBatchPatch>(body)?;
    match node
        .screen_set_tile_batch_mode(
            patch.enabled.unwrap_or(true),
            patch.target_chunk_bytes,
            patch.pacing_interval_us,
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_batch_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/tile/batch` (§167): reset tile batching.
async fn delete_screen_tile_batch(node: &Node) -> Handled {
    match node.screen_reset_tile_batch_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_batch_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/tile/motion` (§167): body `{enabled, max_displacement, min_match_ratio}`.
async fn patch_screen_tile_motion(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenTileMotionPatch>(body)?;
    match node
        .screen_set_tile_motion_mode(
            patch.enabled.unwrap_or(true),
            patch.max_displacement,
            patch.min_match_ratio,
        )
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_motion_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/tile/motion` (§167): reset tile motion matching.
async fn delete_screen_tile_motion(node: &Node) -> Handled {
    match node.screen_reset_tile_motion_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_tile_motion_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/ltr` (§167): body `{enabled, auto_interval}`; enabled default true.
async fn patch_screen_ltr(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenLtrPatch>(body)?;
    match node
        .screen_set_ltr_mode(patch.enabled.unwrap_or(true), patch.auto_interval)
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_ltr_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/ltr` (§167): reset LTR.
async fn delete_screen_ltr(node: &Node) -> Handled {
    match node.screen_reset_ltr_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_ltr_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/svc` (§167): body `{mode, drop_t1}`; mode default `adaptive`, drop_t1 default false.
async fn patch_screen_svc(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenSvcPatch>(body)?;
    let mode =
        tetherly_core::ScreenSvcMode::parse_mode(patch.mode.as_deref().unwrap_or("adaptive"));
    match node
        .screen_set_svc_mode(mode, patch.drop_t1.unwrap_or(false))
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_svc_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /screen/svc` (§167): reset SVC layering.
async fn delete_screen_svc(node: &Node) -> Handled {
    match node.screen_reset_svc_mode().await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_svc_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `POST /screen/svc/drop-t1` (§167): body `{drop_t1}` default true.
async fn post_screen_svc_drop_t1(node: &Node, body: &[u8]) -> Handled {
    let req = parse_body::<ScreenSvcDropT1Request>(body)?;
    match node
        .screen_set_svc_drop_t1(req.drop_t1.unwrap_or(true))
        .await
    {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_svc_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /screen/probe` (§167): body `{mode, target_bps, trigger}`; absent → report.
async fn patch_screen_probe(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenProbePatch>(body)?;
    if let Some(mode_s) = patch.mode {
        let mode = parse_screen_mode::<tetherly_core::ScreenProbeMode>(&mode_s, "probe")?;
        node.screen_set_probe_mode(mode);
    }
    if patch.trigger == Some(true) || patch.target_bps.is_some() {
        let rep = node
            .trigger_screen_probe(patch.target_bps)
            .await
            .map_err(core_error)?;
        return Ok(json(200, &crate::apiv1::model::screen_probe_of(rep)));
    }
    Ok(json(
        200,
        &crate::apiv1::model::screen_probe_of(node.screen_probe_report()),
    ))
}

/// `POST /screen/probe/trigger` (§167): body `{target_bps}` optional.
async fn post_screen_probe_trigger(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenProbePatch>(body)?;
    match node.trigger_screen_probe(patch.target_bps).await {
        Ok(rep) => Ok(json(200, &crate::apiv1::model::screen_probe_of(rep))),
        Err(e) => Err(core_error(e)),
    }
}

// -- quality scalar setters --

/// `PATCH /screen/bitrate` (§167): body `{kbps}` or `{bps}`; required.
async fn patch_screen_bitrate(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenBitratePatch>(body)?;
    let kbps = match (patch.kbps, patch.bps) {
        (Some(k), _) => Some(k),
        (None, Some(bps)) => Some(bps / 1000),
        (None, None) => None,
    };
    let Some(kbps) = kbps.filter(|k| *k > 0) else {
        return Err(error(
            ErrorCode::BadRequest,
            "missing bitrate parameter (kbps or bps)",
        ));
    };
    let rep = node.screen_set_bitrate_reported(kbps);
    Ok(json(200, &crate::apiv1::model::screen_bitrate_of(rep)))
}

/// `PATCH /screen/fps` (§167): body `{fps}`; required.
async fn patch_screen_fps(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenFpsPatch>(body)?;
    let Some(fps) = patch.fps else {
        return Err(error(ErrorCode::BadRequest, "missing fps parameter"));
    };
    let rep = node.screen_set_fps_reported(fps);
    Ok(json(200, &crate::apiv1::model::screen_fps_of(rep)))
}

/// `PATCH /screen/scale` (§167): body `{scale_pct}`; required.
async fn patch_screen_scale(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenScalePatch>(body)?;
    let Some(scale_pct) = patch.scale_pct else {
        return Err(error(
            ErrorCode::BadRequest,
            "missing scale parameter (scale_pct)",
        ));
    };
    let rep = node.screen_set_scale_reported(scale_pct);
    Ok(json(200, &crate::apiv1::model::screen_scale_of(rep)))
}

/// `PATCH /screen/profile` (§167): body `{profile, min_qp, max_qp, qp}`; absent → report.
async fn patch_screen_profile(node: &Node, body: &[u8]) -> Handled {
    let patch = parse_body::<ScreenProfilePatch>(body)?;
    if let Some(p) = patch.profile.as_deref() {
        let profile =
            parse_screen_mode::<tetherly_core::ScreenContentProfile>(p, "content profile")?;
        node.screen_set_profile_reported(profile);
    }
    if let Some(qp) = patch.qp {
        node.screen_set_qp_range_reported(qp, qp);
    } else if patch.min_qp.is_some() || patch.max_qp.is_some() {
        let cur = node.screen_qp_range();
        let min = patch.min_qp.unwrap_or(cur.0);
        let max = patch.max_qp.unwrap_or(cur.1);
        node.screen_set_qp_range_reported(min, max);
    }
    let rep = node.screen_profile_report();
    Ok(json(200, &crate::apiv1::model::screen_profile_of(rep)))
}

/// `PATCH /mic/policy` (§165): sparse update; body wins over legacy query
/// aliases (`enable`/`allow_inbound`/`inbound`/`allow_capture`/`capture`/
/// `ptt`/`max_rate`/`max_duration`/`privacy`/`start_muted`/`timeout`/
/// `idle_timeout`, single-value `group`/`tag`/`type` append).
async fn patch_mic_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<MicPolicyPatch>(body)?;
    if patch.enabled.is_none() {
        patch.enabled = q_str("enabled")
            .or_else(|| q_str("enable"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_inbound_stream.is_none() {
        patch.allow_inbound_stream = q_str("allow_inbound_stream")
            .or_else(|| q_str("allow_inbound"))
            .or_else(|| q_str("inbound"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_remote_capture.is_none() {
        patch.allow_remote_capture = q_str("allow_remote_capture")
            .or_else(|| q_str("allow_capture"))
            .or_else(|| q_str("capture"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allow_intercom.is_none() {
        patch.allow_intercom = q_str("allow_intercom")
            .or_else(|| q_str("intercom"))
            .or_else(|| q_str("ptt"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_sample_rate.is_none() {
        patch.max_sample_rate = q_str("max_sample_rate")
            .or_else(|| q_str("max_rate"))
            .and_then(|s| s.parse().ok());
    }
    if patch.max_session_duration_secs.is_none() {
        patch.max_session_duration_secs = q_str("max_session_duration_secs")
            .or_else(|| q_str("max_duration"))
            .and_then(|s| s.parse().ok());
    }
    if patch.enforce_privacy_indicator.is_none() {
        patch.enforce_privacy_indicator = q_str("enforce_privacy_indicator")
            .or_else(|| q_str("privacy_indicator"))
            .or_else(|| q_str("privacy"))
            .and_then(|s| s.parse().ok());
    }
    if patch.default_muted.is_none() {
        patch.default_muted = q_str("default_muted")
            .or_else(|| q_str("start_muted"))
            .and_then(|s| s.parse().ok());
    }
    if patch.idle_timeout_secs.is_none() {
        patch.idle_timeout_secs = q_str("idle_timeout_secs")
            .or_else(|| q_str("idle_timeout"))
            .or_else(|| q_str("timeout"))
            .and_then(|s| s.parse().ok());
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_str("group")
            .or_else(|| q_str("allowed_group"))
            .filter(|g| !g.trim().is_empty())
            .map(|g| vec![g]);
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_str("tag")
            .or_else(|| q_str("allowed_tag"))
            .filter(|t| !t.trim().is_empty())
            .map(|t| vec![t]);
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_str("type")
            .or_else(|| q_str("device_type"))
            .map(|t| vec![t]);
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.microphone_policy();
    if let Some(v) = patch.enabled {
        policy.enabled = v;
    }
    if let Some(v) = patch.allow_inbound_stream {
        policy.allow_inbound_stream = v;
    }
    if let Some(v) = patch.allow_remote_capture {
        policy.allow_remote_capture = v;
    }
    if let Some(v) = patch.allow_intercom {
        policy.allow_intercom = v;
    }
    if let Some(v) = patch.max_sample_rate {
        policy.max_sample_rate = v;
    }
    if let Some(v) = patch.max_session_duration_secs {
        policy.max_session_duration_secs = v;
    }
    if let Some(v) = patch.enforce_privacy_indicator {
        policy.enforce_privacy_indicator = v;
    }
    if let Some(v) = patch.default_muted {
        policy.default_muted = v;
    }
    if let Some(v) = patch.idle_timeout_secs {
        policy.idle_timeout_secs = v;
    }
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query && !policy.allowed_groups.contains(&g[0]) {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query && !policy.allowed_tags.contains(&t[0]) {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    match crate::apiv1::model::set_mic_policy(node, policy) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /handoff/policy` (§165): sparse update; body wins over legacy
/// query aliases (`max_entries`, single-value `group`/`tag`/`type` append).
async fn patch_handoff_policy(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let qmap = query.map(super::parse_query_map).unwrap_or_default();
    let q_str = |k: &str| qmap.get(k).cloned();
    let mut patch = parse_body::<HandoffPolicyPatch>(body)?;
    if patch.action.is_none() {
        patch.action = q_str("action");
    }
    if patch.allowed_groups.is_none() {
        patch.allowed_groups = q_str("group")
            .filter(|g| !g.trim().is_empty())
            .map(|g| vec![g]);
    }
    if patch.allowed_tags.is_none() {
        patch.allowed_tags = q_str("tag")
            .filter(|t| !t.trim().is_empty())
            .map(|t| vec![t]);
    }
    if patch.allowed_types.is_none() {
        patch.allowed_types = q_str("type")
            .or_else(|| q_str("device_type"))
            .map(|t| vec![t]);
    }
    if patch.max_inbox_entries.is_none() {
        patch.max_inbox_entries = q_str("max_inbox_entries")
            .or_else(|| q_str("max_entries"))
            .and_then(|s| s.parse().ok());
    }
    if patch.clear_filters.is_none() {
        patch.clear_filters = q_str("clear_filters").and_then(|s| s.parse().ok());
    }

    let mut policy = node.handoff_policy();
    if let Some(a) = &patch.action {
        policy.action = a.parse::<tetherly_core::HandoffAction>().map_err(|e| {
            error(
                ErrorCode::BadRequest,
                format!("invalid handoff action '{a}': {e}"),
            )
        })?;
    }
    let from_query = !qmap.is_empty();
    if let Some(mut g) = patch.allowed_groups {
        if from_query && !policy.allowed_groups.contains(&g[0]) {
            g.append(&mut policy.allowed_groups);
            g.sort();
            g.dedup();
        }
        policy.allowed_groups = g;
    }
    if let Some(mut t) = patch.allowed_tags {
        if from_query && !policy.allowed_tags.contains(&t[0]) {
            t.append(&mut policy.allowed_tags);
            t.sort();
            t.dedup();
        }
        policy.allowed_tags = t;
    }
    if let Some(types) = &patch.allowed_types {
        let mut parsed = Vec::new();
        for t in types {
            parsed.push(
                t.parse::<crate::store::DeviceType>()
                    .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?,
            );
        }
        if from_query {
            for ty in policy.allowed_types.drain(..) {
                if !parsed.contains(&ty) {
                    parsed.push(ty);
                }
            }
        }
        policy.allowed_types = parsed;
    }
    if let Some(m) = patch.max_inbox_entries {
        policy.max_inbox_entries = m;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    match crate::apiv1::model::set_handoff_policy(node, policy) {
        Ok(rep) => Ok(json(200, &rep)),
        Err(e) => Err(core_error(e)),
    }
}

/// `GET /inbox/{name}/content`: the raw file bytes with the sniffed mime
/// type (§162b, legacy `/api/file/inbox/download` route shape).
fn get_inbox_content(node: &Node, name: &str) -> Handled {
    match node.open_inbox_download(name) {
        Ok(download) => Ok(
            UiResponse::from_file(200, download.mime, download.file, download.size).with_header(
                "Content-Disposition",
                format!("attachment; filename=\"{}\"", download.name),
            ),
        ),
        Err(e) => Err(core_error(e)),
    }
}

/// `PATCH /link/heartbeat`: dynamic keepalive interval/deadline (§152).
fn patch_heartbeat(node: &Node, body: &[u8]) -> Handled {
    let patch: HeartbeatPatch = parse_body(body)?;
    if patch.is_empty() {
        return Err(error(ErrorCode::BadRequest, "patch sets no field"));
    }
    let cfg = node
        .update_heartbeat_config(patch.interval_secs, patch.deadline_secs)
        .map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::HeartbeatConfig {
            interval_secs: cfg.interval_secs,
            deadline_secs: cfg.deadline_secs,
        },
    ))
}

fn patch_device(node: &Node, id: &str, body: &[u8]) -> Handled {
    let patch: DevicePatch = parse_body(body)?;
    if patch.is_empty() {
        return Err(error(ErrorCode::BadRequest, "patch sets no field"));
    }
    let device_id = node.resolve_peer(id).map_err(core_error)?;
    let id = device_id.to_string();
    if let Some(alias) = &patch.alias {
        node.set_peer_alias(&device_id, alias).map_err(core_error)?;
    }
    if let Some(tags) = patch.tags {
        node.update_peer_profile(&id, None, Some(tags), None, None)
            .map_err(core_error)?;
    }
    Ok(device_json(node, &id))
}

async fn forget_device(node: &Node, id: &str) -> Handled {
    let device_id = node.resolve_peer(id).map_err(core_error)?;
    if node.unpair(&device_id).await {
        Ok(no_content())
    } else {
        Err(error(ErrorCode::NotFound, format!("not paired: {id}")))
    }
}

async fn connect_device(node: &Node, id: &str, body: &[u8]) -> Handled {
    let req: ConnectRequest = parse_body(body)?;
    let id = resolve(node, id)?;
    node.connect_peer(&id, req.addr.as_deref())
        .await
        .map_err(core_error)?;
    Ok(device_json(node, &id))
}

/// `POST /devices/{id}/disconnect`: tear down an active session. The
/// legacy PeerDisconnectReport distinctions (already_disconnected, session
/// names) fold into the Device row (§151): `online` after the call is the
/// answer, `name`/`alias` carry the display identity.
async fn disconnect_device(node: &Node, id: &str) -> Handled {
    node.disconnect_peer(id).await.map_err(core_error)?;
    Ok(device_json(node, id))
}

/// `POST /devices/{id}/ping`: one on-demand RTT probe over the Noise
/// session (§152). The id must resolve; the "single connected peer"
/// auto-target of the legacy endpoint is a CLI-side concern now.
async fn ping_device(node: &Node, id: &str) -> Handled {
    let device_id = node.resolve_peer(id).map_err(core_error)?;
    let rep = node.ping_peer(&device_id).await.map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::DevicePingReport {
            device_id: rep.device_id,
            rtt_ms: rep.rtt_ms,
            path: rep.path,
            addr: rep.addr,
        },
    ))
}

/// `POST /devices/{id}/probe`: an on-demand multi-ping over the Noise
/// session (§152). Defaults match the legacy endpoint (count 5, 100 ms).
async fn probe_device(node: &Node, id: &str, body: &[u8]) -> Handled {
    let req: LinkProbeRequest = parse_body(body)?;
    node.resolve_peer(id).map_err(core_error)?;
    let rep = node
        .probe_peer_link(
            id,
            req.count.unwrap_or(5) as usize,
            req.interval_ms.unwrap_or(100),
        )
        .await
        .map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::LinkProbeReport {
            device_id: rep.peer,
            count: rep.count as u64,
            sent_count: rep.sent as u64,
            recv_count: rep.received as u64,
            min_rtt_ms: rep.min_rtt_ms,
            max_rtt_ms: rep.max_rtt_ms,
            avg_rtt_ms: rep.avg_rtt_ms,
            jitter_ms: rep.jitter_ms,
            packet_loss_rate: rep.packet_loss_rate,
            samples_ms: rep.samples,
            grade: rep.grade.as_str().to_string(),
        },
    ))
}

async fn block_device(node: &Node, id: &str, body: &[u8]) -> Handled {
    let req: BlockRequest = parse_body(body)?;
    let id = resolve(node, id)?;
    node.block_peer(&id, req.reason.as_deref())
        .await
        .map_err(core_error)?;
    Ok(device_json(node, &id))
}

async fn unblock_device(node: &Node, id: &str) -> Handled {
    let id = resolve(node, id)?;
    node.unblock_peer(&id).await.map_err(core_error)?;
    Ok(device_json(node, &id))
}

fn pin_grant(node: &Node) -> Option<PinGrant> {
    let r = node.pin_report();
    match (r.active, r.pin) {
        (true, Some(pin)) => Some(PinGrant {
            pin,
            expires_in_secs: r.expires_in_secs,
        }),
        _ => None,
    }
}

fn get_pin(node: &Node) -> UiResponse {
    match pin_grant(node) {
        Some(g) => json(200, &g),
        None => error(ErrorCode::NotFound, "no active pairing PIN"),
    }
}

/// `POST /pairing/pin/rotate`: a fresh PIN, replacing any active one.
fn rotate_pin(node: &Node) -> Handled {
    node.generate_pin().map_err(core_error)?;
    pin_grant(node)
        .map(|g| json(201, &g))
        .ok_or_else(|| error(ErrorCode::Internal, "PIN not active after generation"))
}

/// `POST /pairing/dial`: optionally prime the PIN, then dial one target by
/// direct address or discovered name/id. §154 folds the legacy `/api/dial`
/// reply (`{ok, addr}`) into the DialReport contract.
async fn dial_peer(node: &Node, body: &[u8]) -> Handled {
    let req: DialRequest = parse_body(body)?;
    if let Some(p) = req.pin.as_deref() {
        let digits = tetherly_core::pin::parse_pin(p)
            .map_err(|e| error(ErrorCode::BadRequest, format!("bad pin: {e}")))?;
        node.set_pin(digits);
    }
    let target = req
        .addr
        .as_deref()
        .or(req.peer.as_deref())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| error(ErrorCode::BadRequest, "missing addr or peer"))?;
    let resolved = node
        .dial_peer(target)
        .await
        .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?;
    Ok(json(
        200,
        &DialReport {
            ok: true,
            addr: resolved.to_string(),
        },
    ))
}

/// `GET /profiles/{id}` — one peer's profile; 404 with the legacy wording
/// when the target cannot be resolved.
fn profile_json(node: &Node, target: &str) -> Handled {
    match node.resolve_peer(target.trim()) {
        Ok(id) => Ok(json(200, &profile_of(node, &id))),
        Err(CoreError::PeerNotFound(e)) => {
            Err(error(ErrorCode::NotFound, format!("peer not found: {e}")))
        }
        Err(e) => Err(error(ErrorCode::BadRequest, e.to_string())),
    }
}

/// One parsed query-param value from the profiles filter set.
fn profile_filter(query: Option<&str>, key: &str) -> Option<String> {
    query
        .map(super::parse_query_map)
        .unwrap_or_default()
        .get(key)
        .cloned()
}

/// `Patch<T>` -> the runtime's `Option<Option<String>>` update shape:
/// `Leave` -> `None` (untouched), `Set` -> `Some(Some)`, `Clear` -> `Some(None)`.
fn patch_owned(p: &Patch<String>) -> Option<Option<String>> {
    match p {
        Patch::Leave => None,
        Patch::Set(s) => Some(Some(s.clone())),
        Patch::Clear => Some(None),
    }
}

fn profile_of(node: &Node, id: &tetherly_core::DeviceId) -> PeerProfileDto {
    let p = node.peer_profile(id);
    PeerProfileDto {
        device_id: p.device_id,
        device_type: p.device_type.as_str().to_string(),
        tags: p.tags,
        group: p.group,
        notes: p.notes,
        updated_ms: p.updated_ms,
    }
}

/// `PATCH /profiles/{id}` — applies the three-state patch and returns the
/// post-patch profile (§151 readback fold).
fn patch_profile(node: &Node, target: &str, body: &[u8]) -> Handled {
    let patch: PeerProfilePatch = parse_body(body)?;
    if patch.is_empty() {
        return Err(error(ErrorCode::BadRequest, "patch sets no field"));
    }
    let device_type = match patch.device_type.as_deref() {
        Some(s) => Some(
            s.parse::<crate::store::DeviceType>()
                .map_err(|e| error(ErrorCode::BadRequest, e))?,
        ),
        None => None,
    };
    let group = patch_owned(&patch.group);
    let notes = patch_owned(&patch.notes);
    let rep = node
        .update_peer_profile(target, device_type, patch.tags, group, notes)
        .map_err(profile_error)?;
    Ok(json(
        200,
        &PeerProfileUpdated {
            device_id: rep.device_id,
            profile: profile_dto_of(rep.profile),
        },
    ))
}

fn profile_dto_of(p: crate::store::PeerProfile) -> PeerProfileDto {
    PeerProfileDto {
        device_id: p.device_id,
        device_type: p.device_type.as_str().to_string(),
        tags: p.tags,
        group: p.group,
        notes: p.notes,
        updated_ms: p.updated_ms,
    }
}

fn profile_error(e: CoreError) -> UiResponse {
    match e {
        CoreError::PeerNotFound(_) => error(ErrorCode::NotFound, e.to_string()),
        _ => error(ErrorCode::BadRequest, e.to_string()),
    }
}

/// `PATCH /caps` — enable/disable local feature flags; empty patch is the
/// legacy "no capability changes specified" 400.
async fn patch_caps(node: &Node, body: &[u8]) -> Handled {
    let patch: CapsUpdate = parse_body(body)?;
    if patch.enable.is_empty() && patch.disable.is_empty() {
        return Err(error(
            ErrorCode::BadRequest,
            "no capability changes specified",
        ));
    }
    node.update_local_caps(&patch.enable, &patch.disable)
        .await
        .map_err(|e| error(ErrorCode::BadRequest, e.to_string()))?;
    Ok(json(200, &crate::apiv1::model::local_caps(node)))
}

/// `GET /devices/{id}/caps` — one peer's capability profile; 404 mirrors the
/// legacy resolve failure.
fn peer_caps_json(node: &Node, target: &str) -> Handled {
    match crate::apiv1::model::peer_caps(node, target) {
        Ok(c) => Ok(json(200, &c)),
        Err(CoreError::PeerNotFound(e)) => {
            Err(error(ErrorCode::NotFound, format!("peer not found: {e}")))
        }
        Err(e) => Err(error(ErrorCode::BadRequest, e.to_string())),
    }
}

// ---- §156b: topology handlers ----

/// `POST /topology/links` — attach a peer to a local display edge. Edge names
/// parse case-insensitively (same FromStr as the legacy body path).
async fn post_topology_link(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::TopologyLinkRequest = parse_body(body)?;
    let edge = req
        .edge
        .trim()
        .to_ascii_lowercase()
        .parse::<tetherly_core::DisplayEdge>()
        .map_err(|e| {
            error(
                ErrorCode::BadRequest,
                format!("invalid edge '{}': {e}", req.edge),
            )
        })?;
    let peer = req.peer.trim().to_string();
    if peer.is_empty() {
        return Err(error(ErrorCode::BadRequest, "missing 'peer' parameter"));
    }
    let peer_edge = req
        .peer_edge
        .as_deref()
        .map(|s| {
            s.trim()
                .to_ascii_lowercase()
                .parse::<tetherly_core::DisplayEdge>()
                .map_err(|e| {
                    error(
                        ErrorCode::BadRequest,
                        format!("invalid peer_edge '{s}': {e}"),
                    )
                })
        })
        .transpose()?;
    let rep = node
        .topology_link_neighbor(edge, &peer, peer_edge, req.offset)
        .await
        .map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::TopologyLinkReport {
            edge: rep.edge.as_str().to_string(),
            peer_id: rep.peer_id,
            peer_edge: rep.peer_edge.as_str().to_string(),
            offset: rep.offset,
        },
    ))
}

/// `POST /topology/unlinks` — detach a local display edge.
async fn post_topology_unlink(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::TopologyUnlinkRequest = parse_body(body)?;
    let edge = req
        .edge
        .trim()
        .to_ascii_lowercase()
        .parse::<tetherly_core::DisplayEdge>()
        .map_err(|e| {
            error(
                ErrorCode::BadRequest,
                format!("invalid edge '{}': {e}", req.edge),
            )
        })?;
    let rep = node
        .topology_unlink_neighbor(edge)
        .await
        .map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::TopologyUnlinkReport {
            edge: rep.edge.as_str().to_string(),
        },
    ))
}

/// `POST /topology/sync` — dispatch sync requests + broadcast local state
/// (legacy `ok`/`message`/duplicate `broadcast_count` folded).
async fn post_topology_sync(node: &Node) -> Handled {
    let sync_res = node.topology_request_sync().await;
    let bc_res = node.topology_broadcast_state().await;
    match (sync_res, bc_res) {
        (Ok(sent), Ok(bc)) => Ok(json(
            200,
            &tetherly_api::TopologySyncReport {
                sync_requests_sent: sent as u64,
                broadcasts_sent: bc as u64,
            },
        )),
        (Err(e), _) | (_, Err(e)) => Err(core_error(e)),
    }
}

/// `PATCH /topology/config` — sparse crossing-geometry update; every absent
/// key keeps its current value (§4.2 sparse PATCH).
async fn patch_topology_config(node: &Node, body: &[u8]) -> Handled {
    let patch: tetherly_api::TopologyConfigPatch = parse_body(body)?;
    let rep = node
        .topology_update_config(
            patch.resistance_px,
            patch.delay_ms,
            patch.corner_lock_size_px,
            patch.wrap_around,
            patch.width,
            patch.height,
            patch.scale_factor,
            patch.redundancy_band_px,
            patch.slow_dwell_ms,
            patch.fast_push_delta_px,
            patch.entry_return_grace_ms,
            patch.remote_speed_percent,
        )
        .await
        .map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::TopologyConfigReport {
            display: crate::apiv1::model::display_rect(rep.display),
            config: crate::apiv1::model::crossing(rep.config),
        },
    ))
}

/// `PATCH /topology/policy` — sparse governance-policy update; the legacy
/// query-parameter variant is folded away (body-only, same as §4.2).
fn patch_topology_policy(node: &Node, body: &[u8]) -> Handled {
    let patch: tetherly_api::TopologyPolicyPatch = parse_body(body)?;
    let mut policy = node.topology_policy();
    if let Some(v) = patch.enabled {
        policy.enabled = v;
    }
    if let Some(v) = patch.allow_inbound_control {
        policy.allow_inbound_control = v;
    }
    if let Some(v) = patch.auto_link_paired_peers {
        policy.auto_link_paired_peers = v;
    }
    if let Some(v) = patch.allowed_groups {
        policy.allowed_groups = v;
    }
    if let Some(v) = patch.allowed_tags {
        policy.allowed_tags = v;
    }
    if let Some(types) = patch.allowed_types {
        let mut dtypes = Vec::new();
        for t in &types {
            match t.parse::<crate::store::DeviceType>() {
                Ok(dt) => dtypes.push(dt),
                Err(e) => {
                    return Err(error(
                        ErrorCode::BadRequest,
                        format!("invalid device type '{t}': {e}"),
                    ));
                }
            }
        }
        policy.allowed_types = dtypes;
    }
    if patch.clear_filters == Some(true) {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }
    node.set_topology_policy(policy).map_err(core_error)?;
    Ok(json(200, &crate::apiv1::model::topology_policy(node)))
}

// ---- §160: discovery / daemon lifecycle handlers ----

/// `POST /system/shutdown` — schedule the daemon's own exit (Admin tier,
/// mirroring the legacy `/api/daemon/shutdown` graceful path: the response
/// is written first, the shutdown fires 50 ms later).
fn post_system_shutdown(node: &Node) -> UiResponse {
    let n = node.clone_handle();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        n.shutdown();
    });
    json(
        200,
        &tetherly_api::DaemonShutdownReport {
            status: "shutting_down".to_string(),
        },
    )
}

/// `POST /system/restart` — hot-replace the daemon binary (Admin tier; the
/// legacy `ok: true`/`message` fields fold away, the action/pid/timestamp
/// triple is the contract). Replacement path failures map through
/// `core_error`-style classes: bad path → 400, in-progress → 409.
async fn post_system_restart(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::DaemonRestartRequest = parse_body(body)?;
    match node.restart_daemon(req.replacement_path.as_deref(), req.delay_ms) {
        Ok(rep) => Ok(json(
            200,
            &tetherly_api::DaemonRestartReport {
                action: rep.action,
                old_pid: rep.old_pid,
                new_pid: rep.new_pid,
                timestamp_ms: rep.timestamp_ms,
            },
        )),
        Err(e) if e.contains("not found") || e.contains("must be placed inside") => {
            Err(error(ErrorCode::BadRequest, e))
        }
        Err(e) if e.contains("already in progress") => Err(error(ErrorCode::Conflict, e)),
        Err(e) => Err(error(ErrorCode::Internal, e)),
    }
}

// ---- §161: notify / reply / message handlers ----

/// `POST /notify` — push a desktop notification text to a peer (legacy
/// `/api/notify`; single-peer ambiguity rules mirror the legacy handler).
async fn post_notify_send(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::NotifySendRequest = parse_body(body)?;
    let mut peers = node.live_peers();
    peers.sort_by_key(|p| p.device_id.to_string());
    let target = match req.peer.as_deref().filter(|s| !s.is_empty()) {
        Some(raw) => {
            let id = node.resolve_peer(raw).map_err(core_error)?;
            if !peers.iter().any(|p| p.device_id == id) {
                return Err(error(ErrorCode::BadRequest, "peer is not connected"));
            }
            id
        }
        None => match peers.len() {
            0 => return Err(error(ErrorCode::Conflict, "no connected peer")),
            1 => peers[0].device_id.clone(),
            _ => {
                return Err(error(
                    ErrorCode::BadRequest,
                    "multiple peers connected; pass peer (tdev_…)",
                ));
            }
        },
    };
    let push = tetherly_core::NotifyPush {
        uid: format!("local-{}", super::node_clock_ms()),
        app_id: req.app_id.unwrap_or_else(|| "com.example.sms".into()),
        app_name: "tetherly".into(),
        title: req.title,
        body: req.body,
        ts: super::node_clock_ms(),
        actions: vec!["copy".into()],
    }
    .truncate();
    node.send_notify(&target, push).await.map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::NotifySendReport {
            peer: target.to_string(),
        },
    ))
}

/// `GET /notify/candidates?peer=&app_id=&has_otp=` — active candidates.
async fn get_notify_candidates(node: &Node, query: Option<&str>) -> Handled {
    let params = query.map(super::parse_query_map).unwrap_or_default();
    let peer = params.get("peer").map(String::as_str);
    let app_id = params.get("app_id").map(String::as_str);
    let has_otp = params.get("has_otp").and_then(|v| v.parse::<bool>().ok());
    let list = node
        .candidates_filtered(peer, app_id, has_otp)
        .map_err(core_error)?;
    let candidates: Vec<tetherly_api::NotifyCandidate> = list
        .into_iter()
        .map(|c| tetherly_api::NotifyCandidate {
            id: c.id,
            source: c.source,
            app_id: c.app_id,
            app_name: c.app_name,
            uid: c.uid,
            title: c.title,
            has_otp: c.has_otp,
            created_ms: c.created_ms,
            expires_ms: c.expires_ms,
            actions: c.actions,
        })
        .collect();
    Ok(json(
        200,
        &tetherly_api::NotifyCandidates {
            count: candidates.len() as u64,
            candidates,
        },
    ))
}

/// `GET /notify/replies` — queued inbound replies (no text: no-content rule).
fn get_notify_replies(node: &Node) -> UiResponse {
    let replies: Vec<tetherly_api::NotifyReplyDto> = node
        .reply_views()
        .into_iter()
        .map(|r| tetherly_api::NotifyReplyDto {
            id: r.id,
            source: r.source.to_string(),
            app_id: r.app_id,
            uid: r.uid,
        })
        .collect();
    json(
        200,
        &tetherly_api::NotifyReplies {
            count: replies.len() as u64,
            replies,
        },
    )
}

/// `POST /notify/otp` — extract the OTP from one candidate.
fn post_notify_otp(node: &Node, body: &[u8]) -> UiResponse {
    let req: tetherly_api::NotifyIdRequest = match parse_body(body) {
        Ok(r) => r,
        Err(e) => return e,
    };
    match node.get_candidate_otp(&req.id) {
        Ok(otp) => json(200, &tetherly_api::NotifyOtpReport { id: req.id, otp }),
        Err(e) => core_error(e),
    }
}

/// `POST /notify/copies` — copy a candidate's OTP to the clipboard.
async fn post_notify_copy(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::NotifyIdRequest = parse_body(body)?;
    node.copy_candidate(&req.id).await.map_err(core_error)?;
    Ok(json(200, &tetherly_api::NotifyActionReport { ok: true }))
}

/// `POST /notify/insertions` — hand a candidate's OTP to the local input path.
fn post_notify_insert(node: &Node, body: &[u8]) -> UiResponse {
    let req: tetherly_api::NotifyIdRequest = match parse_body(body) {
        Ok(r) => r,
        Err(e) => return e,
    };
    match node.insert_candidate(&req.id) {
        Ok(()) => json(200, &tetherly_api::NotifyActionReport { ok: true }),
        Err(e) => core_error(e),
    }
}

/// `POST /notify/dismissals` — dismiss by id, by filter, or all (legacy
/// `/api/dismiss` single-id path folds into the filter call).
async fn post_notify_dismiss(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let req: tetherly_api::NotifyDismissRequest = if body.iter().all(u8::is_ascii_whitespace) {
        let params = query.map(super::parse_query_map).unwrap_or_default();
        tetherly_api::NotifyDismissRequest {
            id: params.get("id").cloned(),
            peer: params.get("peer").cloned(),
            app_id: params.get("app_id").cloned(),
            all: params.get("all").map(|v| v == "true" || v == "1"),
        }
    } else {
        parse_body(body)?
    };
    let all = req.all.unwrap_or(false);
    if all || req.peer.is_some() || req.app_id.is_some() {
        let filter = crate::DismissFilter {
            id: req.id,
            peer: req.peer,
            app_id: req.app_id,
            all: Some(all),
        };
        let rep = node.dismiss_candidates(&filter).await.map_err(core_error)?;
        return Ok(json(
            200,
            &tetherly_api::NotifyDismissReport {
                id: None,
                source: None,
                app_id: None,
                uid: None,
                notified_remote: None,
                count: Some(rep.count as u64),
                ids: Some(rep.ids),
                notified_remote_count: Some(rep.notified_remote_count as u64),
            },
        ));
    }
    let id = req.id.filter(|s| !s.trim().is_empty()).ok_or_else(|| {
        error(
            ErrorCode::BadRequest,
            "dismiss target required: provide id, peer, app_id, or all: true",
        )
    })?;
    let rep = node.dismiss_candidate_sync(&id).await.map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::NotifyDismissReport {
            id: Some(rep.id),
            source: Some(rep.source),
            app_id: Some(rep.app_id),
            uid: Some(rep.uid),
            notified_remote: Some(rep.notified_remote),
            count: None,
            ids: None,
            notified_remote_count: None,
        },
    ))
}

/// `POST /notify/open` — open the candidate's allowlisted app URL.
fn post_notify_open(node: &Node, body: &[u8]) -> UiResponse {
    let req: tetherly_api::NotifyIdRequest = match parse_body(body) {
        Ok(r) => r,
        Err(e) => return e,
    };
    let opener = node.opener();
    match node.open_candidate(&req.id, opener.as_ref()) {
        Ok(url) => json(200, &tetherly_api::NotifyOpenReport { url }),
        Err(e) => core_error(e),
    }
}

/// `POST /replies` — deliver a quick reply to the peer owning the candidate.
async fn post_notify_reply(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::NotifyReplySendRequest = parse_body(body)?;
    node.reply_to_candidate(&req.id, &req.text)
        .await
        .map_err(core_error)?;
    Ok(json(200, &tetherly_api::NotifyActionReport { ok: true }))
}

/// `POST /replies/take` — hand a queued reply to the local OS path exactly
/// once; the text is released only here.
fn post_notify_reply_take(node: &Node, body: &[u8]) -> UiResponse {
    let req: tetherly_api::NotifyReplyTakeRequest = match parse_body(body) {
        Ok(r) => r,
        Err(e) => return e,
    };
    match node.take_reply(&req.id) {
        Some((view, text)) => json(
            200,
            &tetherly_api::NotifyReplyTakeReport {
                id: req.id,
                uid: view.uid,
                app_id: view.app_id,
                text,
            },
        ),
        None => error(ErrorCode::NotFound, "unknown reply"),
    }
}

fn dnd_to_contract(d: crate::runtime::DndMode) -> tetherly_api::NotifyDndMode {
    match d {
        crate::runtime::DndMode::Off => tetherly_api::NotifyDndMode::Off,
        crate::runtime::DndMode::Silent => tetherly_api::NotifyDndMode::Silent,
        crate::runtime::DndMode::All => tetherly_api::NotifyDndMode::All,
    }
}

fn dnd_from_contract(d: tetherly_api::NotifyDndMode) -> crate::runtime::DndMode {
    match d {
        tetherly_api::NotifyDndMode::Off => crate::runtime::DndMode::Off,
        tetherly_api::NotifyDndMode::Silent => crate::runtime::DndMode::Silent,
        tetherly_api::NotifyDndMode::All => crate::runtime::DndMode::All,
    }
}

fn notify_config_json(cfg: crate::runtime::NotifyBannerConfig, totals: (u64, u64)) -> UiResponse {
    json(
        200,
        &tetherly_api::NotifyConfigReportDto {
            config: tetherly_api::NotifyConfigDto {
                banners_enabled: cfg.banners_enabled,
                dnd_mode: dnd_to_contract(cfg.dnd_mode),
                sound_enabled: cfg.sound_enabled,
                mask_otp: cfg.mask_otp,
                allow_otp_in_dnd: cfg.allow_otp_in_dnd,
            },
            totals: tetherly_api::NotifyConfigTotals {
                dispatched: totals.0,
                suppressed: totals.1,
            },
        },
    )
}

/// `GET /notify/config` — banner switches + dispatch/suppression totals.
fn get_notify_config(node: &Node) -> UiResponse {
    let rep = node.notify_config_report();
    notify_config_json(rep.config, (rep.total_dispatched, rep.total_suppressed))
}

/// `PATCH /notify/config` — partial update (query params fold for
/// compatibility with the legacy merge order: body wins).
fn patch_notify_config(node: &Node, query: Option<&str>, body: &[u8]) -> UiResponse {
    let mut cfg = node.notify_config();
    if let Some(q) = query {
        let map = super::parse_query_map(q);
        if let Some(b) = map.get("banners").or_else(|| map.get("banners_enabled")) {
            cfg.banners_enabled = b == "true" || b == "1";
        }
        if let Some(d) = map.get("dnd").or_else(|| map.get("dnd_mode")) {
            cfg.dnd_mode = match d.as_str() {
                "all" => crate::runtime::DndMode::All,
                "silent" => crate::runtime::DndMode::Silent,
                _ => crate::runtime::DndMode::Off,
            };
        }
        if let Some(s) = map.get("sound").or_else(|| map.get("sound_enabled")) {
            cfg.sound_enabled = s == "true" || s == "1";
        }
        if let Some(m) = map.get("mask_otp") {
            cfg.mask_otp = m == "true" || m == "1";
        }
        if let Some(a) = map.get("allow_otp_in_dnd") {
            cfg.allow_otp_in_dnd = a == "true" || a == "1";
        }
    }
    if !body.iter().all(u8::is_ascii_whitespace) {
        let patch: tetherly_api::NotifyConfigPatch = match parse_body(body) {
            Ok(p) => p,
            Err(e) => return e,
        };
        if let Some(b) = patch.banners_enabled {
            cfg.banners_enabled = b;
        }
        if let Some(d) = patch.dnd_mode {
            cfg.dnd_mode = dnd_from_contract(d);
        }
        if let Some(s) = patch.sound_enabled {
            cfg.sound_enabled = s;
        }
        if let Some(m) = patch.mask_otp {
            cfg.mask_otp = m;
        }
        if let Some(a) = patch.allow_otp_in_dnd {
            cfg.allow_otp_in_dnd = a;
        }
    }
    let rep = node.update_notify_config(cfg);
    notify_config_json(rep.config, (rep.total_dispatched, rep.total_suppressed))
}

/// `POST /notify/test` — fire a test banner (query params merge into the
/// body the legacy way).
fn post_notify_test(node: &Node, query: Option<&str>, body: &[u8]) -> UiResponse {
    let mut title: Option<String> = None;
    let mut text: Option<String> = None;
    if let Some(q) = query {
        let map = super::parse_query_map(q);
        title = map.get("title").cloned();
        text = map.get("body").or_else(|| map.get("message")).cloned();
    }
    if !body.iter().all(u8::is_ascii_whitespace) {
        match parse_body::<tetherly_api::NotifyTestRequestDto>(body) {
            Ok(b) => {
                title = b.title.or(title);
                text = b.body.or(text);
            }
            Err(e) => return e,
        }
    }
    match node.test_notification_banner(title.as_deref(), text.as_deref()) {
        Ok(rep) => json(
            200,
            &tetherly_api::NotifyTestReportDto {
                sent: rep.sent,
                suppressed: rep.suppressed,
                suppression_reason: rep.suppression_reason,
                message: rep.message,
                title: rep.title,
            },
        ),
        Err(e) => core_error(e),
    }
}

// ---- §161: message handlers ----

/// `GET /messages?peer=&limit=&unread_only=&mark_read=` — filtered history.
async fn get_message_history(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let mut peer: Option<String> = None;
    let mut limit: Option<usize> = None;
    let mut unread_only = false;
    let mut mark_read = false;
    if let Some(q) = query {
        let map = super::parse_query_map(q);
        peer = map
            .get("peer")
            .or_else(|| map.get("to"))
            .or_else(|| map.get("target"))
            .cloned();
        limit = map.get("limit").and_then(|s| s.parse::<usize>().ok());
        if let Some(u) = map.get("unread_only").or_else(|| map.get("unread")) {
            unread_only = u == "true" || u == "1";
        }
        if let Some(m) = map.get("mark_read").or_else(|| map.get("read")) {
            mark_read = m == "true" || m == "1";
        }
    }
    if !body.iter().all(u8::is_ascii_whitespace) {
        let q: tetherly_api::MessageQuery = parse_body(body)?;
        if q.peer.is_some() {
            peer = q.peer;
        }
        if let Some(l) = q.limit {
            limit = Some(usize::try_from(l).unwrap_or(usize::MAX));
        }
        if let Some(u) = q.unread_only {
            unread_only = u;
        }
        if let Some(m) = q.mark_read {
            mark_read = m;
        }
    }
    let rep = node
        .message_history(peer.as_deref(), limit, unread_only, mark_read)
        .map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::MessageHistory {
            total: rep.total as u64,
            returned: rep.returned as u64,
            unread: rep.unread as u64,
            messages: rep.messages.into_iter().map(message_entry).collect(),
        },
    ))
}

fn message_entry(m: crate::runtime::PeerMessage) -> tetherly_api::MessageEntry {
    tetherly_api::MessageEntry {
        id: m.id,
        peer: m.peer,
        direction: match m.direction {
            crate::runtime::MessageDirection::Inbound => tetherly_api::MessageDirection::Inbound,
            crate::runtime::MessageDirection::Outbound => tetherly_api::MessageDirection::Outbound,
        },
        text: m.text,
        timestamp_ms: m.timestamp_ms,
        read: m.read,
    }
}

fn selector_from_parts(
    peer: Option<String>,
    tag: Option<String>,
    group: Option<String>,
    device_type: Option<String>,
    all: bool,
) -> Result<crate::TargetSelector, UiResponse> {
    crate::TargetSelector::from_query(
        peer.as_deref(),
        tag.as_deref(),
        group.as_deref(),
        device_type.as_deref(),
        all,
    )
    .map_err(|e| error(ErrorCode::BadRequest, e))
}

/// `POST /messages` — send a direct message; tag/group/device_type/all
/// select multi-target dispatch (report shape changes accordingly).
async fn post_message_send(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let mut req: tetherly_api::MessageSendRequest = if body.iter().all(u8::is_ascii_whitespace) {
        let params = query.map(super::parse_query_map).unwrap_or_default();
        tetherly_api::MessageSendRequest {
            peer: params.get("peer").cloned(),
            tag: params.get("tag").cloned(),
            group: params.get("group").cloned(),
            device_type: params
                .get("device_type")
                .or_else(|| params.get("type"))
                .cloned(),
            all: params.get("all").map(|v| v == "true" || v == "1"),
            text: params.get("text").cloned().unwrap_or_default(),
        }
    } else {
        parse_body(body)?
    };
    if req.text.trim().is_empty() {
        if let Some(q) = query {
            let map = super::parse_query_map(q);
            if let Some(t) = map.get("text") {
                req.text = t.clone();
            }
        }
    }
    let is_multi = req.all.unwrap_or(false)
        || req.tag.is_some()
        || req.group.is_some()
        || req.device_type.is_some();
    if is_multi {
        let selector = selector_from_parts(
            req.peer,
            req.tag,
            req.group,
            req.device_type,
            req.all.unwrap_or(false),
        )?;
        let label = selector.as_label();
        let rep = node
            .send_message_multi(&selector, req.text.trim())
            .await
            .map_err(core_error)?;
        return Ok(json(
            200,
            &tetherly_api::MessageDispatchReport {
                selector: label,
                target_count: rep.target_count as u64,
                success_count: rep.success_count as u64,
                skipped_offline_count: rep.skipped_offline_count as u64,
                failed_count: rep.failed_count as u64,
                dispatches: rep
                    .dispatches
                    .into_iter()
                    .map(|d| tetherly_api::MessageDispatchItem {
                        peer: d.peer,
                        peer_alias: d.peer_alias,
                        message_id: d.message_id,
                        success: d.success,
                        error: d.error,
                    })
                    .collect(),
            },
        ));
    }
    let rep = node
        .send_message_to_peer(req.peer.as_deref().unwrap_or(""), req.text.trim())
        .await
        .map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::MessageSendReport {
            message: message_entry(rep.message),
        },
    ))
}

/// `DELETE /messages` — clear all history, or just one peer's.
async fn post_message_clear(node: &Node, query: Option<&str>, body: &[u8]) -> Handled {
    let mut peer: Option<String> = None;
    if let Some(q) = query {
        let map = super::parse_query_map(q);
        peer = map
            .get("peer")
            .or_else(|| map.get("to"))
            .or_else(|| map.get("target"))
            .cloned();
    }
    if !body.iter().all(u8::is_ascii_whitespace) {
        let req: tetherly_api::MessageClearRequest = parse_body(body)?;
        if req.peer.is_some() {
            peer = req.peer;
        }
    }
    let rep = node.clear_messages(peer.as_deref()).map_err(core_error)?;
    Ok(json(
        200,
        &tetherly_api::MessageClearReport {
            target: rep.target,
            cleared_count: rep.cleared_count as u64,
            message: rep.message,
        },
    ))
}

// ---- §156b: routes / network handlers ----

/// `POST /routes` — pin a peer to a physical socket address.
fn post_route(node: &Node, body: &[u8]) -> Handled {
    let req: tetherly_api::RouteSetRequest = parse_body(body)?;
    if req.peer.trim().is_empty() {
        return Err(error(ErrorCode::BadRequest, "peer must be specified"));
    }
    if req.addr.trim().is_empty() {
        return Err(error(ErrorCode::BadRequest, "addr must be specified"));
    }
    match node.add_route(&req.peer, &req.addr) {
        Ok(rep) => Ok(json(
            200,
            &tetherly_api::RouteMutateReport {
                device_id: rep.device_id,
                addr: rep.addr,
                action: rep.action,
            },
        )),
        Err(e) => Err(core_error(e)),
    }
}

/// `DELETE /routes?peer=` — unpin a peer's physical route; 404 mirrors the
/// legacy resolve failure (§4.2 query style, body variant folded away).
fn delete_route(node: &Node, query: Option<&str>) -> Handled {
    let params = query.map(super::parse_query_map).unwrap_or_default();
    let peer = params
        .get("peer")
        .or_else(|| params.get("device_id"))
        .cloned()
        .unwrap_or_default();
    if peer.trim().is_empty() {
        return Err(error(
            ErrorCode::BadRequest,
            "peer must be specified via query",
        ));
    }
    match node.remove_route(&peer) {
        Ok(rep) => Ok(json(
            200,
            &tetherly_api::RouteMutateReport {
                device_id: rep.device_id,
                addr: rep.addr,
                action: rep.action,
            },
        )),
        Err(e) => Err(core_error(e)),
    }
}

async fn log_tail(query: Option<&str>) -> Handled {
    let params = query.map(super::parse_query_map).unwrap_or_default();
    let lines = match params.get("tail") {
        None => 200,
        Some(v) => v
            .parse::<usize>()
            .map_err(|_| error(ErrorCode::BadRequest, "tail must be a line count"))?,
    };
    let min_level = match params.get("level") {
        None => None,
        Some(l) => Some(crate::logging::parse_level(l).ok_or_else(|| {
            error(
                ErrorCode::BadRequest,
                "level must be error|warn|info|debug|trace",
            )
        })?),
    };
    let q = crate::logging::TailQuery {
        lines,
        grep: params.get("grep").filter(|g| !g.is_empty()).cloned(),
        min_level,
    };
    let path = crate::logging::log_path()
        .ok_or_else(|| error(ErrorCode::Unavailable, "file logging not active"))?
        .to_path_buf();
    match tokio::task::spawn_blocking(move || crate::logging::tail(&path, &q)).await {
        Ok(Ok(lines)) => Ok(json(200, &LogTail { lines })),
        Ok(Err(e)) => Err(error(ErrorCode::Internal, format!("read log: {e}"))),
        Err(_) => Err(error(ErrorCode::Internal, "tail task failed")),
    }
}

fn log_level(body: &[u8]) -> Handled {
    let req: LogLevelRequest = parse_body(body)?;
    if !req.reset && req.filter.is_none() {
        return Err(error(
            ErrorCode::BadRequest,
            "body must set \"filter\" or \"reset\": true",
        ));
    }
    let ctl = crate::logging::control()
        .ok_or_else(|| error(ErrorCode::Unavailable, "logging control not active"))?;
    let result = match req.filter.as_deref() {
        Some(spec) if !req.reset => crate::logging::set_filter_with_revert(
            ctl,
            spec,
            req.revert_after_secs
                .filter(|s| *s > 0)
                .map(std::time::Duration::from_secs),
            super::node_clock_ms(),
        ),
        _ => ctl.reset().map(|_| ()),
    };
    result.map_err(|e| error(ErrorCode::BadRequest, e))?;
    let s = crate::logging::status();
    Ok(json(
        200,
        &LogLevel {
            filter: s.filter,
            baseline: s.baseline_filter,
            revert_at_ms: s.revert_at_ms,
        },
    ))
}

/// Keep-alive comment interval; also how fast a dead client is noticed.
/// §147: must stay well under `TRAY_TAKEOVER_RECLAIM_MS` (10s) — on a
/// silent link the ping is the only feed of the §6.3 reclaim window, and
/// the old 15s value could never feed a 10s window (every claim was
/// reclaimed out from under a live shell).
const PING: std::time::Duration = std::time::Duration::from_secs(5);

/// `GET /api/v1/events`: snapshot (or `Last-Event-ID` replay), then live
/// frames. `?topics=a,b` filters; snapshots always pass. A subscriber that
/// falls behind the broadcast buffer is disconnected and resumes by id.
pub(super) async fn events(
    stream: &mut TcpStream,
    node: &Node,
    hub: &Arc<EventHub>,
    query: Option<&str>,
    header_last_id: Option<u64>,
) -> std::io::Result<()> {
    let params = query.map(super::parse_query_map).unwrap_or_default();
    let topics = match params.get("topics").map(|t| Topic::parse_list(t)) {
        None => None,
        Some(Ok(list)) => Some(list),
        Some(Err(bad)) => {
            let resp = error(ErrorCode::BadRequest, format!("unknown topic: {bad}"));
            return write_http_response(stream, &resp).await;
        }
    };
    let wanted = |f: &Frame| match (&topics, f.topic) {
        (Some(list), Some(t)) => list.contains(&t),
        _ => true,
    };
    let last_id =
        header_last_id.or_else(|| params.get("last_event_id").and_then(|v| v.parse().ok()));
    let mut sub = hub.subscribe(last_id);

    // §6.3 tray takeover: an SSE subscriber IS shell activity. Mark it seen
    // and feed the reclaim watchdog once per accepted connection; the frame
    // and ping loops below keep feeding it while the stream lives.
    node.tray_keepalive();
    node.mark_tray_sse_seen();

    let head = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/event-stream; charset=utf-8\r\n\
         Cache-Control: no-cache, no-transform\r\n\
         Connection: keep-alive\r\n\
         {}: {}\r\n\
         {}: {}\r\n\r\n\
         retry: 2000\n\n",
        tetherly_api::API_HEADER,
        API_VERSION,
        tetherly_api::VERSION_HEADER,
        env!("CARGO_PKG_VERSION"),
    );
    stream.write_all(head.as_bytes()).await?;

    let mut sent = sub.current;
    match sub.replay.take() {
        Some(frames) => {
            for f in frames.iter().filter(|f| wanted(f)) {
                write_sse_timeout(stream, f.sse().as_bytes()).await?;
            }
            sent = frames.last().map_or(sent, |f| f.seq.max(sent));
        }
        None => {
            let snapshot = crate::apiv1::model::snapshot(node).await;
            let frame = Frame::new(sub.current, &Event::Snapshot { snapshot });
            write_sse_timeout(stream, frame.sse().as_bytes()).await?;
        }
    }
    stream.flush().await?;

    let mut shutdown = node.shutdown_signal();
    let mut ping = tokio::time::interval(PING);
    ping.tick().await;
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            res = sub.rx.recv() => match res {
                Ok(f) => {
                    if f.seq <= sent {
                        continue;
                    }
                    sent = f.seq;
                    if wanted(&f) {
                        write_sse_timeout(stream, f.sse().as_bytes()).await?;
                        stream.flush().await?;
                    }
                    // Any forwarded frame proves the subscriber is alive.
                    node.tray_keepalive();
                }
                Err(RecvError::Lagged(_)) | Err(RecvError::Closed) => break,
            },
            _ = ping.tick() => {
                write_sse_timeout(stream, b": ping\n\n").await?;
                stream.flush().await?;
                // §147: the ping IS the silent link's keepalive feed — the
                // §6.3 contract says "per frame + ping", but this arm never
                // fed the window, so every claim died at the 10s mark.
                node.tray_keepalive();
            }
        }
    }
    Ok(())
}

/// §158: an SSE write that never completes (a wedged peer, not merely a slow
/// one) would stall this loop forever — the broadcast buffer then lags and
/// the lagged-client disconnect never fires. Each write gets a generous
/// budget (several ping periods); on timeout the stream is torn down and the
/// client reconnects via `Last-Event-ID`.
async fn write_sse_timeout(stream: &mut TcpStream, bytes: &[u8]) -> std::io::Result<()> {
    tokio::time::timeout(PING.saturating_mul(4), stream.write_all(bytes))
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "sse client stopped reading")
        })?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P3: open-path redaction is tier-gated — only Admin keeps the absolute
    /// path, lower tiers see an inbox-relative name.
    #[test]
    fn inbox_open_path_is_redacted_below_admin() {
        use crate::apitoken::Tier;
        assert_eq!(
            redact_inbox_open_path(Some(Tier::Admin), Some("a.txt"), "/real/inbox/a.txt"),
            "/real/inbox/a.txt"
        );
        assert_eq!(
            redact_inbox_open_path(Some(Tier::Write), Some("a.txt"), "/real/inbox/a.txt"),
            "inbox/a.txt"
        );
        assert_eq!(
            redact_inbox_open_path(Some(Tier::Read), None, "/real/inbox"),
            "inbox"
        );
        assert_eq!(redact_inbox_open_path(None, None, "/real/inbox"), "inbox");
    }

    /// §157: every CoreError a v1 route can surface maps onto the class its
    /// legacy route used — client faults never come back as 500.
    #[test]
    fn core_error_maps_each_variant_to_its_legacy_class() {
        use tetherly_api::ErrorCode as C;
        let class = |e: CoreError| core_error(e).code;
        assert_eq!(
            class(CoreError::PeerNotFound("x".into())),
            C::NotFound.http_status()
        );
        assert_eq!(class(CoreError::UnknownTransfer), C::NotFound.http_status());
        assert_eq!(
            class(CoreError::File("file not found: x".into())),
            C::NotFound.http_status()
        );
        assert_eq!(
            class(CoreError::UnsafeFileName),
            C::BadRequest.http_status()
        );
        assert_eq!(
            class(CoreError::OpenRefused("no app".into())),
            C::BadRequest.http_status()
        );
        assert_eq!(class(CoreError::InvalidPin), C::BadRequest.http_status());
        assert_eq!(
            class(CoreError::NoConnectedPeer),
            C::Conflict.http_status(),
            "caller-state errors are 409, not 500 (legacy said 400)"
        );
        assert_eq!(class(CoreError::PingTimeout), C::Conflict.http_status());
        assert_eq!(
            class(CoreError::FileTooLarge(9)),
            C::PayloadTooLarge.http_status()
        );
        assert_eq!(class(CoreError::NoConnectedPeer), C::Conflict.http_status());
        assert_eq!(
            class(CoreError::File("disk full".into())),
            C::Internal.http_status(),
            "genuinely unexpected faults stay 500"
        );
        assert_eq!(
            class(CoreError::Json("bad".into())),
            C::Internal.http_status()
        );
    }

    /// §157: path-item helpers percent-decode their segment, so aliases with
    /// spaces or UTF-8 are addressable (legacy query-param parity).
    #[test]
    fn path_item_helpers_decode_percent_escapes() {
        assert_eq!(
            device_item("/api/v1/devices/mac%20book%20pro").as_deref(),
            Some("mac book pro")
        );
        assert_eq!(
            profile_item("/api/v1/profiles/%E6%9C%BA%E6%88%BF").as_deref(),
            Some("机房")
        );
        assert_eq!(
            device_item("/api/v1/devices/a/b"),
            None,
            "multi-segment stays unrouted"
        );
        assert_eq!(
            inbox_open_action("/api/v1/inbox/re%20port.pdf/open").as_deref(),
            Some("re port.pdf")
        );
        assert_eq!(
            inbox_item("/api/v1/inbox/"),
            None,
            "empty name is not an item"
        );
    }
}
