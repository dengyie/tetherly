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
    BlockRequest, ConnectRequest, DevicePatch, Diagnostics, ErrorBody, ErrorCode, Event, Health,
    LogLevel, LogLevelRequest, LogTail, PinGrant, SettingsPatch, SysHost, SysProcess, SysTelemetry,
    Topic, API_VERSION,
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

/// `/api/v1/devices/{id}` -> `id` (one non-empty segment).
fn device_item(path: &str) -> Option<&str> {
    path.strip_prefix("/api/v1/devices/")
        .filter(|id| !id.is_empty() && !id.contains('/'))
}

/// `/api/v1/devices/{id}/{action}` -> (`id`, `action`).
fn device_action(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/v1/devices/")?;
    let (id, action) = rest.split_once('/')?;
    (!id.is_empty() && !action.contains('/')).then_some((id, action))
}

/// `/api/v1/transfers/{id}/{action}` -> (`id`, `action`).
fn transfer_action(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/v1/transfers/")?;
    let (id, action) = rest.split_once('/')?;
    (!id.is_empty() && !action.contains('/')).then_some((id, action))
}

/// `/api/v1/inbox/{name}` -> name (single segment, may contain dots).
fn inbox_item(path: &str) -> Option<&str> {
    path.strip_prefix("/api/v1/inbox/")
        .filter(|n| !n.is_empty() && !n.contains('/'))
}

/// `/api/v1/inbox/{name}/open` -> name.
fn inbox_open_action(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/api/v1/inbox/")?;
    let (name, action) = rest.split_once('/')?;
    (action == "open" && !name.is_empty()).then_some(name)
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
        "/api/v1/transfers" => Some(&["GET", "DELETE"]),
        "/api/v1/inbox" => Some(&["GET", "DELETE"]),
        "/api/v1/settings" => Some(&["GET", "PATCH"]),
        "/api/v1/pairing/pin" => Some(&["GET", "DELETE"]),
        "/api/v1/pairing/pin/rotate" | "/api/v1/input/release" => Some(&["POST"]),
        "/api/v1/diagnostics/log-level" => Some(&["PATCH"]),
        p if device_item(p).is_some() => Some(&["GET", "PATCH", "DELETE"]),
        p => match device_action(p) {
            Some((_, "connect")) | Some((_, "disconnect")) => Some(&["POST"]),
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

/// Map a node error onto the v1 envelope.
fn core_error(e: CoreError) -> UiResponse {
    let code = match &e {
        CoreError::PeerNotFound(_) | CoreError::UnknownTransfer => ErrorCode::NotFound,
        // Inbox lookups surface missing files as File("file not found…");
        // legacy /api/file/inbox maps the same shape to 404.
        CoreError::File(ref msg) if msg.contains("not found") => ErrorCode::NotFound,
        CoreError::InvalidAlias(_)
        | CoreError::InvalidProfile(_)
        | CoreError::InvalidRoute(_)
        | CoreError::InvalidPayload(_)
        | CoreError::InvalidConfig(_)
        | CoreError::InvalidDeviceId => ErrorCode::BadRequest,
        CoreError::PeerBlocked(_) => ErrorCode::Conflict,
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
        ("DELETE", "/api/v1/pairing/pin") => {
            node.clear_pin();
            Ok(no_content())
        }
        ("GET", "/api/v1/input") => Ok(json(200, &crate::apiv1::model::input(node).await)),
        ("GET", "/api/v1/transfers") => Ok(json(200, &crate::apiv1::model::transfers(node))),
        ("GET", "/api/v1/inbox") => Ok(json(200, &crate::apiv1::model::inbox(node))),
        ("GET", "/api/v1/layout") => Ok(json(200, &crate::apiv1::model::layout(node))),
        ("GET", "/api/v1/settings") => Ok(json(200, &crate::apiv1::model::settings(node))),
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
        ("PATCH", "/api/v1/settings") => patch_settings(node, body),
        ("DELETE", "/api/v1/transfers") => {
            node.clear_terminal_transfers();
            Ok(json(200, &crate::apiv1::model::transfers(node)))
        }
        ("DELETE", "/api/v1/inbox") => match node.clear_inbox(None) {
            Ok(_) => Ok(json(200, &crate::apiv1::model::inbox(node))),
            Err(e) => Err(core_error(e)),
        },
        ("GET", p) if device_item(p).is_some() => {
            Ok(device_json(node, device_item(p).unwrap_or_default()))
        }
        ("PATCH", p) if device_item(p).is_some() => {
            patch_device(node, device_item(p).unwrap_or_default(), body)
        }
        ("DELETE", p) if device_item(p).is_some() => {
            forget_device(node, device_item(p).unwrap_or_default()).await
        }
        (m, p) => match (m, device_action(p)) {
            ("POST", Some((id, "connect"))) => connect_device(node, id, body).await,
            ("POST", Some((id, "disconnect"))) => disconnect_device(node, id).await,
            ("POST", Some((id, "block"))) => block_device(node, id, body).await,
            ("DELETE", Some((id, "block"))) => unblock_device(node, id).await,
            _ => match (m, transfer_action(p)) {
                ("POST", Some((id, "cancel"))) => cancel_transfer(node, id).await,
                _ => match (m, inbox_open_action(p)) {
                    ("POST", Some(name)) => open_inbox_file(node, name, body).await,
                    _ => match (m, inbox_item(p)) {
                        ("DELETE", Some(name)) => delete_inbox_file(node, name),
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
                stream.write_all(f.sse().as_bytes()).await?;
            }
            sent = frames.last().map_or(sent, |f| f.seq.max(sent));
        }
        None => {
            let snapshot = crate::apiv1::model::snapshot(node).await;
            let frame = Frame::new(sub.current, &Event::Snapshot { snapshot });
            stream.write_all(frame.sse().as_bytes()).await?;
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
                        stream.write_all(f.sse().as_bytes()).await?;
                        stream.flush().await?;
                    }
                    // Any forwarded frame proves the subscriber is alive.
                    node.tray_keepalive();
                }
                Err(RecvError::Lagged(_)) | Err(RecvError::Closed) => break,
            },
            _ = ping.tick() => {
                stream.write_all(b": ping\n\n").await?;
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
