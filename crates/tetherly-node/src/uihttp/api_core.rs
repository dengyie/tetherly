// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Node state, config, PIN, dialling, capabilities, routes, net, metrics and process reports.

use super::*;

pub(super) async fn get_index(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    _node: &Node,
) -> UiResponse {
    UiResponse::new(200, "text/html; charset=utf-8", INDEX.as_bytes().to_vec())
}

pub(super) async fn get_state(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&snapshot(node).await)
}

pub(super) async fn get_config(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.config_report())
}

pub(super) async fn post_config(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<ConfigUpdateBody>(body) {
        Ok(d) => {
            if d.name.is_none() && d.advertise.is_none() && d.reconnect.is_none() {
                return err(400, "no configuration fields to update");
            }
            match node.update_config(d.name.as_deref(), d.advertise, d.reconnect) {
                Ok(report) => json_ok(&report),
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn get_pin(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.pin_report())
}

pub(super) async fn post_pin(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.generate_pin() {
        Ok(pin) => {
            let rep = node.pin_report();
            json_ok(&serde_json::json!({
                "ok": true,
                "pin": pin,
                "expires_in_secs": rep.expires_in_secs,
                "ttl_secs": rep.ttl_secs
            }))
        }
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_pin_clear(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let cleared = node.clear_pin();
    json_ok(&serde_json::json!({ "ok": true, "cleared": cleared }))
}

pub(super) async fn post_dial(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<DialBody>(body) {
        Ok(d) => {
            if let Some(p) = d.pin {
                match tetherly_core::pin::parse_pin(&p) {
                    Ok(digits) => node.set_pin(digits),
                    Err(e) => return err(400, &e.to_string()),
                }
            }
            let target = d
                .addr
                .as_deref()
                .or(d.peer.as_deref())
                .filter(|s| !s.trim().is_empty());
            let target = match target {
                Some(t) => t,
                None => return err(400, "missing addr or peer"),
            };
            match node.dial_peer(target).await {
                Ok(resolved_addr) => json_ok(&serde_json::json!({
                    "ok": true,
                    "addr": resolved_addr.to_string(),
                })),
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn get_caps(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.local_caps_report())
}

pub(super) async fn post_caps(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct CapsUpdateBody {
        #[serde(default)]
        enable: Vec<String>,
        #[serde(default)]
        disable: Vec<String>,
    }
    match serde_json::from_slice::<CapsUpdateBody>(body) {
        Ok(b) => {
            if b.enable.is_empty() && b.disable.is_empty() {
                return err(400, "no capability changes specified");
            }
            match node.update_local_caps(&b.enable, &b.disable).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(
            400,
            "bad json, expected { enable?: string[], disable?: string[] }",
        ),
    }
}

pub(super) async fn get_routes(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.routes_report())
}

pub(super) async fn post_routes(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<RouteAddBody>(body) {
        Ok(d) => {
            let peer = d.peer.or(d.device_id).unwrap_or_default();
            if peer.trim().is_empty() {
                return err(400, "peer or device_id must be specified");
            }
            if d.addr.trim().is_empty() {
                return err(400, "addr must be specified");
            }
            match node.add_route(&peer, &d.addr) {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(400, "bad json, expected { peer: string, addr: string }"),
    }
}

pub(super) async fn delete_routes(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let peer_from_query = qmap.get("peer").or_else(|| qmap.get("device_id")).cloned();
    let peer = if let Some(p) = peer_from_query {
        p
    } else if !body.is_empty() {
        #[derive(serde::Deserialize, Default)]
        struct RouteDelBody {
            #[serde(default)]
            peer: Option<String>,
            #[serde(default)]
            device_id: Option<String>,
        }
        match serde_json::from_slice::<RouteDelBody>(body) {
            Ok(d) => d.peer.or(d.device_id).unwrap_or_default(),
            Err(_) => return err(400, "bad json, expected { peer: string }"),
        }
    } else {
        String::new()
    };
    if peer.trim().is_empty() {
        return err(400, "peer or device_id must be specified via query or body");
    }
    match node.remove_route(&peer) {
        Ok(rep) => json_ok(&rep),
        Err(tetherly_core::CoreError::PeerNotFound(msg)) => err(404, &msg),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_net(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.network_report_dto())
}

pub(super) async fn post_net_refresh(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.refresh_network().await;
    json_ok(&rep)
}

pub(super) async fn get_metrics(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.traffic_report())
}

pub(super) async fn post_metrics_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    node.reset_traffic_metrics();
    json_ok(&serde_json::json!({
        "ok": true,
        "reset": true,
        "timestamp_ms": node_clock_ms()
    }))
}

pub(super) async fn get_health(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.health_report();
    json_ok(&rep)
}

pub(super) async fn get_sys(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.system_report() {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &e.to_string()),
    }
}

// Remote screen: consent is local and explicit. `allow` is the only
// gate into streaming; nothing a peer sends can flip it.
pub(super) async fn post_daemon_shutdown(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let n = node.clone_handle();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        n.shutdown();
    });
    json_ok(&serde_json::json!({
        "ok": true,
        "status": "shutting_down",
        "message": "node shutdown initiated"
    }))
}

pub(super) async fn post_daemon_restart(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let req: DaemonRestartBody = if body.is_empty() {
        DaemonRestartBody::default()
    } else {
        // Malformed JSON must not silently become "restart now with
        // defaults" — the caller gets a 400 instead.
        match serde_json::from_slice::<DaemonRestartBody>(body) {
            Ok(r) => r,
            Err(e) => return err(400, &format!("bad request body: {e}")),
        }
    };
    match node.restart_daemon(req.replacement_path.as_deref(), req.delay_ms) {
        Ok(rep) => json_ok(&rep),
        Err(e) => {
            if e.contains("not found") || e.contains("must be placed inside") {
                err(400, &e)
            } else if e.contains("already in progress") {
                err(409, &e)
            } else {
                err(500, &e)
            }
        }
    }
}

pub(super) async fn any_events(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    _node: &Node,
) -> UiResponse {
    err(405, "method not allowed")
}
