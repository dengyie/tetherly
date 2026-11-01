// SPDX-License-Identifier: Apache-2.0 OR MIT
//! URL handoff inbox/outbox and handoff policy.

use super::*;

pub(super) async fn post_handoff_send_multi(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let req: HandoffSendBody = if body.is_empty() {
        HandoffSendBody {
            peer: qmap.get("peer").cloned(),
            url: qmap.get("url").cloned().unwrap_or_default(),
            title: qmap.get("title").cloned(),
            auto_open: qmap.get("auto_open").map(|s| s == "true" || s == "1"),
            tag: qmap.get("tag").cloned(),
            group: qmap.get("group").cloned(),
            device_type: qmap
                .get("type")
                .or_else(|| qmap.get("device_type"))
                .cloned(),
            r#type: None,
            all: qmap.get("all").map(|s| s == "true" || s == "1"),
        }
    } else {
        match serde_json::from_slice::<HandoffSendBody>(body) {
            Ok(b) => b,
            Err(_) => return err(400, "bad json, expected HandoffSendBody"),
        }
    };
    let url = if !req.url.trim().is_empty() {
        req.url.trim()
    } else if let Some(u) = qmap.get("url") {
        u.trim()
    } else {
        return err(400, "url cannot be empty");
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
    let auto_open = req.auto_open.unwrap_or(false)
        || qmap
            .get("auto_open")
            .map(|s| s == "true" || s == "1")
            .unwrap_or(false);
    let title = req
        .title
        .as_deref()
        .or_else(|| qmap.get("title").map(String::as_str));

    let selector =
        match crate::store::TargetSelector::from_query(peer, tag, group, dev_type, is_all) {
            Ok(s) => s,
            Err(e) => return err(400, &e),
        };

    match node
        .handoff_send_multi(&selector, url, title, auto_open)
        .await
    {
        Ok(report) => json_ok(&report),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_handoff_send(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let req: HandoffSendBody = if body.is_empty() {
        HandoffSendBody {
            peer: qmap.get("peer").cloned(),
            url: qmap.get("url").cloned().unwrap_or_default(),
            title: qmap.get("title").cloned(),
            auto_open: qmap.get("auto_open").map(|s| s == "true" || s == "1"),
            tag: qmap.get("tag").cloned(),
            group: qmap.get("group").cloned(),
            device_type: qmap
                .get("type")
                .or_else(|| qmap.get("device_type"))
                .cloned(),
            r#type: None,
            all: qmap.get("all").map(|s| s == "true" || s == "1"),
        }
    } else {
        match serde_json::from_slice::<HandoffSendBody>(body) {
            Ok(b) => b,
            Err(_) => return err(400, "bad json, expected HandoffSendBody"),
        }
    };
    let url = if !req.url.trim().is_empty() {
        req.url.trim()
    } else if let Some(u) = qmap.get("url") {
        u.trim()
    } else {
        return err(400, "url cannot be empty");
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
    let auto_open = req.auto_open.unwrap_or(false)
        || qmap
            .get("auto_open")
            .map(|s| s == "true" || s == "1")
            .unwrap_or(false);
    let title = req
        .title
        .as_deref()
        .or_else(|| qmap.get("title").map(String::as_str));

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
        match node
            .handoff_send_multi(&selector, url, title, auto_open)
            .await
        {
            Ok(report) => return json_ok(&report),
            Err(e) => return err(400, &e.to_string()),
        }
    }

    let target_peer = req
        .peer
        .as_deref()
        .or_else(|| qmap.get("peer").map(String::as_str))
        .unwrap_or("");
    match node.handoff_send(target_peer, url, title, auto_open).await {
        Ok(res) => json_ok(&res),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_handoff_inbox(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let status_filter = qmap
        .get("status")
        .and_then(|s| s.parse::<tetherly_core::HandoffStatus>().ok());
    let list = node.handoff_inbox_list(status_filter);
    json_ok(&serde_json::json!({
        "ok": true,
        "total": list.len(),
        "inbox": list,
        "entries": list,
    }))
}

pub(super) async fn post_handoff_inbox_open(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let target_id = qmap.get("id").cloned().or_else(|| {
        if !body.is_empty() {
            serde_json::from_slice::<HandoffInboxActionBody>(body)
                .ok()
                .and_then(|b| b.id)
        } else {
            None
        }
    });
    let Some(id) = target_id else {
        return err(400, "missing required 'id' parameter");
    };
    match node.handoff_inbox_open(&id) {
        Ok(entry) => json_ok(&serde_json::json!({
            "ok": true,
            "id": entry.id,
            "url": entry.url,
            "status": entry.status.as_str(),
            "opened_ms": entry.opened_ms,
            "entry": entry,
        })),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_handoff_inbox_dismiss(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let target_id = qmap.get("id").cloned().or_else(|| {
        if !body.is_empty() {
            serde_json::from_slice::<HandoffInboxActionBody>(body)
                .ok()
                .and_then(|b| b.id)
        } else {
            None
        }
    });
    let Some(id) = target_id else {
        return err(400, "missing required 'id' parameter");
    };
    match node.handoff_inbox_dismiss(&id) {
        Ok(entry) => json_ok(&serde_json::json!({
            "ok": true,
            "id": entry.id,
            "status": entry.status.as_str(),
            "entry": entry,
        })),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_handoff_inbox_clear(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.handoff_inbox_clear() {
        Ok(cleared) => json_ok(&serde_json::json!({
            "ok": true,
            "cleared": cleared,
        })),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_handoff_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.handoff_policy_report())
}

pub(super) async fn post_handoff_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<HandoffPolicyBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<HandoffPolicyBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected handoff policy fields"),
        }
    };
    let mut policy = node.handoff_policy();

    if let Some(ref b) = parsed_body {
        if let Some(ref a) = b.action {
            match a.parse::<tetherly_core::HandoffAction>() {
                Ok(act) => policy.action = act,
                Err(e) => return err(400, &format!("invalid handoff action '{a}': {e}")),
            }
        }
        if let Some(ref grps) = b.allowed_groups {
            policy.allowed_groups = grps.clone();
        }
        if let Some(ref tags) = b.allowed_tags {
            policy.allowed_tags = tags.clone();
        }
        if let Some(ref types) = b.allowed_types {
            let mut dtypes = Vec::new();
            for t in types {
                match t.parse::<crate::store::DeviceType>() {
                    Ok(dt) => dtypes.push(dt),
                    Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                }
            }
            policy.allowed_types = dtypes;
        }
        if let Some(max_entries) = b.max_inbox_entries {
            policy.max_inbox_entries = max_entries;
        }
        if b.clear_filters == Some(true) {
            policy.allowed_groups.clear();
            policy.allowed_tags.clear();
            policy.allowed_types.clear();
        }
    }

    if let Some(act_str) = qmap.get("action") {
        match act_str.parse::<tetherly_core::HandoffAction>() {
            Ok(act) => policy.action = act,
            Err(e) => return err(400, &format!("invalid handoff action '{act_str}': {e}")),
        }
    }
    if let Some(g) = qmap.get("group") {
        if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
            policy.allowed_groups.push(g.clone());
        }
    }
    if let Some(t) = qmap.get("tag") {
        if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
            policy.allowed_tags.push(t.clone());
        }
    }
    if let Some(dt_str) = qmap.get("type").or_else(|| qmap.get("device_type")) {
        match dt_str.parse::<crate::store::DeviceType>() {
            Ok(dt) => {
                if !policy.allowed_types.contains(&dt) {
                    policy.allowed_types.push(dt);
                }
            }
            Err(e) => return err(400, &format!("invalid device type '{dt_str}': {e}")),
        }
    }
    if let Some(max_str) = qmap
        .get("max_inbox_entries")
        .or_else(|| qmap.get("max_entries"))
    {
        if let Ok(m) = max_str.parse::<usize>() {
            policy.max_inbox_entries = m;
        }
    }
    if qmap
        .get("clear_filters")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false)
    {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }

    match node.set_handoff_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
