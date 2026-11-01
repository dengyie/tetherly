// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Desktop notifications, OTP rules, open rules, replies and dismissals.

use super::*;

pub(super) async fn post_notify(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<NotifyBody>(body) {
        Ok(d) => {
            let mut peers = node.live_peers();
            peers.sort_by_key(|p| p.device_id.to_string());
            let target = match d.peer.as_deref().filter(|s| !s.is_empty()) {
                Some(raw) => match node.resolve_peer(raw) {
                    Ok(id) => {
                        if peers.iter().any(|p| p.device_id == id) {
                            id
                        } else {
                            return err(400, "peer is not connected");
                        }
                    }
                    Err(e) => return err(400, &format!("bad peer id: {e}")),
                },
                None => match peers.len() {
                    0 => return err(400, "no connected peer"),
                    1 => peers[0].device_id.clone(),
                    _ => return err(400, "multiple peers connected; pass peer (tdev_…)"),
                },
            };
            let push = tetherly_core::NotifyPush {
                uid: format!("local-{}", node_clock_ms()),
                app_id: d.app_id,
                app_name: "tetherly".into(),
                title: d.title,
                body: d.body,
                ts: node_clock_ms(),
                actions: vec!["copy".into()],
            }
            .truncate();
            match node.send_notify(&target, push).await {
                Ok(()) => json_ok(&serde_json::json!({ "ok": true, "peer": target.as_str() })),
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn get_notify_candidates(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let peer = qmap.get("peer").map(String::as_str);
    let app_id = qmap.get("app_id").map(String::as_str);
    let has_otp = qmap.get("has_otp").and_then(|v| v.parse::<bool>().ok());

    match node.candidates_filtered(peer, app_id, has_otp) {
        Ok(list) => json_ok(&serde_json::json!({
            "ok": true,
            "candidates": list,
        })),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_notify_replies(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let replies: Vec<crate::events::ReplyViewDto> = node
        .reply_views()
        .into_iter()
        .map(crate::events::ReplyViewDto::from)
        .collect();
    json_ok(&serde_json::json!({
        "ok": true,
        "replies": replies,
    }))
}

pub(super) async fn post_otp_get(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<IdBody>(body) {
        Ok(d) => match node.get_candidate_otp(&d.id) {
            Ok(otp) => json_ok(&serde_json::json!({
                "ok": true,
                "id": d.id,
                "otp": otp,
            })),
            Err(e) => err(400, &e.to_string()),
        },
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn post_copy(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<IdBody>(body) {
        Ok(d) => match node.copy_candidate(&d.id).await {
            Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
            Err(e) => err(400, &e.to_string()),
        },
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn post_insert(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<IdBody>(body) {
        Ok(d) => match node.insert_candidate(&d.id) {
            Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
            Err(e) => err(400, &e.to_string()),
        },
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn post_dismiss(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<IdBody>(body) {
        Ok(d) => match node.dismiss_candidate_sync(&d.id).await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        },
        Err(_) => err(400, "bad json, expected { id: string }"),
    }
}

pub(super) async fn post_notify_dismiss(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<NotifyDismissBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<NotifyDismissBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(
                400,
                "bad json, expected { id?: string, peer?: string, app_id?: string, all?: bool }",
            ),
        }
    };

    let id = parsed_body
        .as_ref()
        .and_then(|b| b.id.clone())
        .or_else(|| qmap.get("id").cloned());
    let peer = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone())
        .or_else(|| qmap.get("peer").cloned());
    let app_id = parsed_body
        .as_ref()
        .and_then(|b| b.app_id.clone())
        .or_else(|| qmap.get("app_id").cloned());
    let all = parsed_body
        .as_ref()
        .and_then(|b| b.all)
        .or_else(|| qmap.get("all").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(false);

    if all || peer.is_some() || app_id.is_some() {
        let filter = crate::DismissFilter {
            id,
            peer,
            app_id,
            all: Some(all),
        };
        match node.dismiss_candidates(&filter).await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        }
    } else if let Some(target_id) = id.filter(|s| !s.trim().is_empty()) {
        match node.dismiss_candidate_sync(&target_id).await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        }
    } else {
        err(
            400,
            "dismiss target required: provide id, peer, app_id, or all: true",
        )
    }
}

// Quick reply typed on the desktop, sent back to the phone that owns
// the notification. The text is the user's own; it is delivered, never
// executed, and never logged.
pub(super) async fn post_open(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<IdBody>(body) {
        Ok(d) => {
            let op = node.opener();
            match node.open_candidate(&d.id, op.as_ref()) {
                Ok(url) => json_ok(&serde_json::json!({ "ok": true, "url": url })),
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(400, "bad json"),
    }
}

// Hand a previously received reply to the local OS path exactly once.
pub(super) async fn post_reply(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<ReplyBody>(body) {
        Ok(d) => match node.reply_to_candidate(&d.id, &d.text).await {
            Ok(()) => json_ok(&serde_json::json!({ "ok": true })),
            Err(e) => err(400, &e.to_string()),
        },
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn post_reply_take(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<IdBody>(body) {
        Ok(d) => match node.take_reply(&d.id) {
            Some((view, text)) => json_ok(&serde_json::json!({
                "ok": true,
                "id": d.id,
                "uid": view.uid,
                "app_id": view.app_id,
                "text": text,
            })),
            None => err(400, "unknown reply"),
        },
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn get_notify_config(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.notify_config_report())
}

pub(super) async fn post_notify_config(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let mut cfg = node.notify_config();
    if let Some(q) = query_str {
        let map = parse_query_map(q);
        if let Some(b) = map.get("banners").or_else(|| map.get("banners_enabled")) {
            cfg.banners_enabled = b == "true" || b == "1";
        }
        if let Some(d) = map.get("dnd").or_else(|| map.get("dnd_mode")) {
            match d.as_str() {
                "all" => cfg.dnd_mode = crate::runtime::DndMode::All,
                "silent" => cfg.dnd_mode = crate::runtime::DndMode::Silent,
                "off" => cfg.dnd_mode = crate::runtime::DndMode::Off,
                _ => {}
            }
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

    if !body.is_empty() {
        match serde_json::from_slice::<NotifyConfigUpdateBody>(body) {
            Ok(up) => {
                if let Some(b) = up.banners_enabled {
                    cfg.banners_enabled = b;
                }
                if let Some(d) = up.dnd_mode {
                    cfg.dnd_mode = d;
                }
                if let Some(s) = up.sound_enabled {
                    cfg.sound_enabled = s;
                }
                if let Some(m) = up.mask_otp {
                    cfg.mask_otp = m;
                }
                if let Some(a) = up.allow_otp_in_dnd {
                    cfg.allow_otp_in_dnd = a;
                }
            }
            Err(_) => return err(400, "bad json for notify config"),
        }
    }

    let rep = node.update_notify_config(cfg);
    json_ok(&rep)
}

pub(super) async fn post_notify_test(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let mut q_title: Option<String> = None;
    let mut q_body: Option<String> = None;
    if let Some(q) = query_str {
        let map = parse_query_map(q);
        q_title = map.get("title").cloned();
        q_body = map.get("body").or_else(|| map.get("message")).cloned();
    }

    let (title, body_text) = if !body.is_empty() {
        match serde_json::from_slice::<NotifyTestBody>(body) {
            Ok(b) => (b.title.or(q_title), b.body.or(q_body)),
            Err(_) => (q_title, q_body),
        }
    } else {
        (q_title, q_body)
    };

    match node.test_notification_banner(title.as_deref(), body_text.as_deref()) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
