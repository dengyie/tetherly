// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Peer-to-peer text messages: send, query, clear.

use super::*;

pub(super) async fn post_message_send_multi(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let req: MessageSendBody = if body.is_empty() {
        MessageSendBody {
            peer: qmap.get("peer").cloned(),
            tag: qmap.get("tag").cloned(),
            group: qmap.get("group").cloned(),
            device_type: qmap
                .get("type")
                .or_else(|| qmap.get("device_type"))
                .cloned(),
            r#type: None,
            all: qmap.get("all").map(|s| s == "true" || s == "1"),
            text: qmap.get("text").cloned().unwrap_or_default(),
        }
    } else {
        match serde_json::from_slice::<MessageSendBody>(body) {
            Ok(b) => b,
            Err(_) => return err(400, "bad json"),
        }
    };
    let text = if !req.text.trim().is_empty() {
        req.text.trim()
    } else if let Some(t) = qmap.get("text") {
        t.trim()
    } else {
        return err(400, "message text cannot be empty");
    };

    let is_all = req.all.unwrap_or(false)
        || qmap
            .get("all")
            .map(|s| s == "true" || s == "1")
            .unwrap_or(false)
        || qmap.contains_key("all");
    let tag = req
        .tag
        .as_deref()
        .or_else(|| qmap.get("tag").map(String::as_str));
    let group = req
        .group
        .as_deref()
        .or_else(|| qmap.get("group").map(String::as_str));
    let dev_type = req
        .device_type
        .as_deref()
        .or(req.r#type.as_deref())
        .or_else(|| {
            qmap.get("type")
                .or_else(|| qmap.get("device_type"))
                .map(String::as_str)
        });
    let peer = req
        .peer
        .as_deref()
        .or_else(|| qmap.get("peer").map(String::as_str));

    let selector =
        match crate::store::TargetSelector::from_query(peer, tag, group, dev_type, is_all) {
            Ok(s) => s,
            Err(e) => return err(400, &e),
        };

    match node.send_message_multi(&selector, text).await {
        Ok(report) => json_ok(&report),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_message_send(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let req: MessageSendBody = match serde_json::from_slice(body) {
        Ok(b) => b,
        Err(_) => return err(400, "bad json"),
    };
    let is_all = req.all.unwrap_or(false)
        || qmap
            .get("all")
            .map(|s| s == "true" || s == "1")
            .unwrap_or(false)
        || qmap.contains_key("all");
    let tag = req
        .tag
        .as_deref()
        .or_else(|| qmap.get("tag").map(String::as_str));
    let group = req
        .group
        .as_deref()
        .or_else(|| qmap.get("group").map(String::as_str));
    let dev_type = req
        .device_type
        .as_deref()
        .or(req.r#type.as_deref())
        .or_else(|| {
            qmap.get("type")
                .or_else(|| qmap.get("device_type"))
                .map(String::as_str)
        });

    if is_all || tag.is_some() || group.is_some() || dev_type.is_some() {
        let peer = req
            .peer
            .as_deref()
            .or_else(|| qmap.get("peer").map(String::as_str));
        let selector =
            match crate::store::TargetSelector::from_query(peer, tag, group, dev_type, is_all) {
                Ok(s) => s,
                Err(e) => return err(400, &e),
            };
        match node.send_message_multi(&selector, &req.text).await {
            Ok(report) => return json_ok(&report),
            Err(e) => return err(400, &e.to_string()),
        }
    }

    match node
        .send_message_to_peer(req.peer.as_deref().unwrap_or(""), &req.text)
        .await
    {
        Ok(report) => json_ok(&report),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_message_history(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let mut q_peer: Option<String> = None;
    let mut q_limit: Option<usize> = None;
    let mut q_unread_only = false;
    let mut q_mark_read = false;

    if let Some(q) = query_str {
        let map = parse_query_map(q);
        if let Some(p) = map
            .get("peer")
            .or_else(|| map.get("to"))
            .or_else(|| map.get("target"))
        {
            q_peer = Some(p.clone());
        }
        if let Some(l) = map.get("limit").and_then(|s| s.parse::<usize>().ok()) {
            q_limit = Some(l);
        }
        if let Some(u) = map.get("unread_only").or_else(|| map.get("unread")) {
            q_unread_only = u == "true" || u == "1";
        }
        if let Some(m) = map.get("mark_read").or_else(|| map.get("read")) {
            q_mark_read = m == "true" || m == "1";
        }
    }

    if !body.is_empty() {
        match serde_json::from_slice::<MessageQueryBody>(body) {
            Ok(b) => {
                if b.peer.is_some() {
                    q_peer = b.peer;
                }
                if b.limit.is_some() {
                    q_limit = b.limit;
                }
                if let Some(u) = b.unread_only {
                    q_unread_only = u;
                }
                if let Some(m) = b.mark_read {
                    q_mark_read = m;
                }
            }
            Err(_) => return err(400, "bad json"),
        }
    }

    match node.message_history(q_peer.as_deref(), q_limit, q_unread_only, q_mark_read) {
        Ok(report) => json_ok(&report),
        Err(e) => err(400, &e.to_string()),
    }
}

// Cross-device Resource & URL Handoff endpoints (v1.72 / ADR-065)
pub(super) async fn post_message_clear(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let mut q_peer: Option<String> = None;
    if let Some(q) = query_str {
        let map = parse_query_map(q);
        if let Some(p) = map
            .get("peer")
            .or_else(|| map.get("to"))
            .or_else(|| map.get("target"))
        {
            q_peer = Some(p.clone());
        }
    }
    if !body.is_empty() {
        if let Ok(b) = serde_json::from_slice::<MessageClearBody>(body) {
            if b.peer.is_some() {
                q_peer = b.peer;
            }
        }
    }
    match node.clear_messages(q_peer.as_deref()) {
        Ok(report) => json_ok(&report),
        Err(e) => err(400, &e.to_string()),
    }
}
