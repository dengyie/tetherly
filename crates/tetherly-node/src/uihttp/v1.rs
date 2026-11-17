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
    BlockRequest, CapsUpdate, ConnectRequest, DevicePatch, Diagnostics, DialReport, DialRequest,
    ErrorBody, ErrorCode, Event, Health, HeartbeatPatch, LinkProbeRequest, LogLevel,
    LogLevelRequest, LogTail, Patch, PeerProfileDto, PeerProfilePatch, PeerProfileUpdated,
    PinGrant, SettingsPatch, SysHost, SysProcess, SysTelemetry, Topic, API_VERSION,
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

/// Methods each known v1 path answers; drives 404 vs 405.
fn allowed(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "/api/v1/health"
        | "/api/v1/system"
        | "/api/v1/events"
        | "/api/v1/devices"
        | "/api/v1/pairing"
        | "/api/v1/input"
        | "/api/v1/layout"
        | "/api/v1/activity"
        | "/api/v1/diagnostics"
        | "/api/v1/diagnostics/sys"
        | "/api/v1/diagnostics/logs" => Some(&["GET"]),
        "/api/v1/link" => Some(&["GET"]),
        "/api/v1/link/heartbeat" => Some(&["GET", "PATCH"]),
        "/api/v1/transfers" => Some(&["GET", "DELETE"]),
        "/api/v1/inbox" => Some(&["GET", "DELETE"]),
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
        ("GET", "/api/v1/input") => Ok(json(200, &crate::apiv1::model::input(node).await)),
        ("GET", "/api/v1/transfers") => Ok(json(200, &crate::apiv1::model::transfers(node))),
        ("GET", "/api/v1/inbox") => Ok(json(200, &crate::apiv1::model::inbox(node))),
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
        ("DELETE", "/api/v1/inbox") => match node.clear_inbox(None) {
            Ok(_) => Ok(json(200, &crate::apiv1::model::inbox(node))),
            Err(e) => Err(core_error(e)),
        },
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
                _ => match (m, inbox_open_action(p)) {
                    ("POST", Some(name)) => open_inbox_file(node, &name, body).await,
                    _ => match (m, inbox_item(p)) {
                        ("DELETE", Some(name)) => delete_inbox_file(node, &name),
                        _ => Ok(match allowed(path) {
                            Some(methods) => error(
                                ErrorCode::MethodNotAllowed,
                                format!("{method} not allowed; use {}", methods.join(", ")),
                            )
                            .with_header("Allow", methods.join(", ")),
                            None => error(ErrorCode::NotFound, format!("no such resource: {path}")),
                        }),
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
    Ok(json(200, &crate::apiv1::model::inbox(node)))
}

/// `POST /inbox/{name}/open`: reveals the file with the OS opener.
async fn open_inbox_file(node: &Node, name: &str, body: &[u8]) -> Handled {
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
    tokio::task::spawn_blocking(move || node.open_inbox_file(Some(&name), opener.as_ref()))
        .await
        .map_err(|_| error(ErrorCode::Internal, "open task failed"))?
        .map_err(core_error)?;
    Ok(no_content())
}

/// `PATCH /link/heartbeat`: dynamic keepalive interval/deadline (§152).
/// Partial patch; omitted fields stay untouched.
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
                    ))
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
