// SPDX-License-Identifier: Apache-2.0 OR MIT
//! File offers, transfers, auto-accept, inbox, outbox and storage housekeeping.

use super::*;

pub(super) async fn get_file_transfers(
    method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let mut q_id: Option<String> = None;
    let mut q_active: Option<bool> = None;
    let mut q_terminal: Option<bool> = None;
    let mut q_direction: Option<String> = None;

    if let Some(q) = query_str {
        let map = parse_query_map(q);
        if let Some(id) = map.get("id").or_else(|| map.get("transfer_id")) {
            q_id = Some(id.clone());
        }
        if let Some(a) = map.get("active") {
            q_active = Some(a == "true" || a == "1");
        }
        if let Some(t) = map.get("terminal") {
            q_terminal = Some(t == "true" || t == "1");
        }
        if let Some(d) = map.get("direction") {
            q_direction = Some(d.clone());
        }
    }

    if method == "POST" && !body.is_empty() {
        match serde_json::from_slice::<TransfersQueryBody>(body) {
            Ok(b) => {
                if b.id.is_some() {
                    q_id = b.id;
                }
                if b.active.is_some() {
                    q_active = b.active;
                }
                if b.terminal.is_some() {
                    q_terminal = b.terminal;
                }
                if b.direction.is_some() {
                    q_direction = b.direction;
                }
            }
            Err(_) => return err(400, "bad json"),
        }
    }

    if let Some(id) = q_id.as_deref().filter(|s| !s.is_empty()) {
        match node.transfer_by_id(id) {
            Some(t) => json_ok(&t),
            None => err(404, "transfer not found"),
        }
    } else {
        let mut rep = node.transfers_report();
        if let Some(true) = q_active {
            rep.transfers.retain(|t| {
                t.phase == crate::runtime::TransferPhase::Transferring
                    || t.phase == crate::runtime::TransferPhase::Verifying
            });
        } else if let Some(true) = q_terminal {
            rep.transfers.retain(|t| {
                t.phase == crate::runtime::TransferPhase::Completed
                    || t.phase == crate::runtime::TransferPhase::Failed
            });
        }
        if let Some(dir) = q_direction.as_deref() {
            let dir_lower = dir.to_ascii_lowercase();
            if dir_lower == "in" || dir_lower == "inbound" {
                rep.transfers
                    .retain(|t| t.direction == crate::runtime::TransferDirection::Inbound);
            } else if dir_lower == "out" || dir_lower == "outbound" {
                rep.transfers
                    .retain(|t| t.direction == crate::runtime::TransferDirection::Outbound);
            }
        }
        rep.total_count = rep.transfers.len();
        rep.active_count = rep
            .transfers
            .iter()
            .filter(|t| {
                t.phase == crate::runtime::TransferPhase::Transferring
                    || t.phase == crate::runtime::TransferPhase::Verifying
            })
            .count();
        rep.terminal_count = rep.total_count.saturating_sub(rep.active_count);
        json_ok(&rep)
    }
}

pub(super) async fn get_file_offers(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let q_id = qmap
        .get("id")
        .or_else(|| qmap.get("transfer_id"))
        .map(String::as_str);
    let q_peer = qmap
        .get("peer")
        .or_else(|| qmap.get("device_id"))
        .map(String::as_str);
    match node.file_offers(q_id, q_peer) {
        Ok(rep) => {
            if q_id.is_some() {
                if let Some(first) = rep.offers.first() {
                    json_ok(first)
                } else {
                    err(404, "transfer offer not found")
                }
            } else {
                json_ok(&rep)
            }
        }
        Err(tetherly_core::CoreError::UnknownTransfer) => err(404, "transfer offer not found"),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_file_accept(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<FileDecisionBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<FileDecisionBody>(body) {
            Ok(b) => Some(b),
            Err(_) => {
                return err(
                    400,
                    "bad json, expected { id?: string, all?: bool, peer?: string }",
                )
            }
        }
    };
    let is_all = parsed_body.as_ref().and_then(|b| b.all).unwrap_or(false)
        || qmap
            .get("all")
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);
    let target_peer = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone())
        .or_else(|| qmap.get("peer").or_else(|| qmap.get("device_id")).cloned());
    let target_id = parsed_body
        .as_ref()
        .and_then(|b| b.id.clone())
        .or_else(|| qmap.get("id").or_else(|| qmap.get("transfer_id")).cloned());

    if is_all || (target_id.is_none() && target_peer.is_some()) {
        match node.batch_accept_files(target_peer.as_deref()).await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        }
    } else if let Some(id) = target_id {
        match node.user_accept_file_reported(&id).await {
            Ok(rep) => json_ok(&rep),
            Err(tetherly_core::CoreError::UnknownTransfer) => err(404, "transfer offer not found"),
            Err(e) => err(400, &e.to_string()),
        }
    } else {
        err(400, "missing required 'id', 'all', or 'peer' parameter")
    }
}

pub(super) async fn post_file_reject(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<FileDecisionBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<FileDecisionBody>(body) {
            Ok(b) => Some(b),
            Err(_) => {
                return err(
                    400,
                    "bad json, expected { id?: string, all?: bool, peer?: string }",
                )
            }
        }
    };
    let is_all = parsed_body.as_ref().and_then(|b| b.all).unwrap_or(false)
        || qmap
            .get("all")
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);
    let target_peer = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone())
        .or_else(|| qmap.get("peer").or_else(|| qmap.get("device_id")).cloned());
    let target_id = parsed_body
        .as_ref()
        .and_then(|b| b.id.clone())
        .or_else(|| qmap.get("id").or_else(|| qmap.get("transfer_id")).cloned());

    if is_all || (target_id.is_none() && target_peer.is_some()) {
        match node.batch_reject_files(target_peer.as_deref()).await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        }
    } else if let Some(id) = target_id {
        match node.user_reject_file_reported(&id).await {
            Ok(rep) => json_ok(&rep),
            Err(tetherly_core::CoreError::UnknownTransfer) => err(404, "transfer offer not found"),
            Err(e) => err(400, &e.to_string()),
        }
    } else {
        err(400, "missing required 'id', 'all', or 'peer' parameter")
    }
}

pub(super) async fn get_file_auto_accept_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.auto_accept_policy_report())
}

pub(super) async fn post_file_auto_accept_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<AutoAcceptBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<AutoAcceptBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected auto-accept policy fields"),
        }
    };
    let mut policy = node.auto_accept_policy();
    if let Some(ref b) = parsed_body {
        if let Some(en) = b.enabled.or(b.auto_accept_trusted) {
            policy.enabled = en;
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
        if let Some(sz) = b.max_file_size {
            policy.max_file_size = Some(sz);
        }
    }
    if let Some(en_str) = qmap
        .get("enabled")
        .or_else(|| qmap.get("auto_accept_trusted"))
    {
        if let Ok(en) = en_str.parse::<bool>() {
            policy.enabled = en;
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
    if let Some(sz_str) = qmap.get("max_size").or_else(|| qmap.get("max_file_size")) {
        if let Ok(sz) = sz_str.parse::<u64>() {
            policy.max_file_size = Some(sz);
        }
    }
    match node.set_auto_accept_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_file_auto_accept(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    if qmap
        .get("policy")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false)
    {
        json_ok(&node.auto_accept_policy_report())
    } else {
        json_ok(&node.auto_accept_status())
    }
}

pub(super) async fn post_file_auto_accept(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<AutoAcceptBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<AutoAcceptBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected { enabled?: bool }"),
        }
    };
    let has_policy_fields = parsed_body
        .as_ref()
        .map(|b| {
            b.allowed_groups.is_some()
                || b.allowed_tags.is_some()
                || b.allowed_types.is_some()
                || b.max_file_size.is_some()
        })
        .unwrap_or(false)
        || qmap.contains_key("group")
        || qmap.contains_key("tag")
        || qmap.contains_key("max_size");

    if has_policy_fields {
        let mut policy = node.auto_accept_policy();
        if let Some(ref b) = parsed_body {
            if let Some(en) = b.enabled.or(b.auto_accept_trusted) {
                policy.enabled = en;
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
            if let Some(sz) = b.max_file_size {
                policy.max_file_size = Some(sz);
            }
        }
        if let Some(en_str) = qmap
            .get("enabled")
            .or_else(|| qmap.get("auto_accept_trusted"))
        {
            if let Ok(en) = en_str.parse::<bool>() {
                policy.enabled = en;
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
        if let Some(sz_str) = qmap.get("max_size").or_else(|| qmap.get("max_file_size")) {
            if let Ok(sz) = sz_str.parse::<u64>() {
                policy.max_file_size = Some(sz);
            }
        }
        match node.set_auto_accept_policy(policy) {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        }
    } else {
        let enabled = parsed_body
            .and_then(|b| b.enabled.or(b.auto_accept_trusted))
            .or_else(|| {
                qmap.get("enabled")
                    .or_else(|| qmap.get("auto_accept_trusted"))
                    .and_then(|v| v.parse::<bool>().ok())
            })
            .unwrap_or(true);
        json_ok(&node.set_auto_accept(enabled))
    }
}

pub(super) async fn post_file_cancel(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<IdBody>(body) {
        Ok(d) => match node.cancel_transfer(&d.id).await {
            Ok(()) => json_ok(&serde_json::json!({ "ok": true, "transfer_id": d.id })),
            Err(e) => err(400, &e.to_string()),
        },
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn post_file_clear(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let cleared = node.clear_terminal_transfers();
    json_ok(&serde_json::json!({ "ok": true, "cleared": cleared }))
}

pub(super) async fn post_file_send_multi(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let d: SendFileBody = if body.is_empty() {
        SendFileBody {
            path: qmap.get("path").cloned(),
            data_base64: None,
            peer: qmap.get("peer").cloned(),
            name: qmap.get("name").cloned(),
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
        match serde_json::from_slice::<SendFileBody>(body) {
            Ok(b) => b,
            Err(_) => return err(400, "bad json, expected SendFileBody"),
        }
    };
    let is_all = d.all.unwrap_or(false)
        || qmap
            .get("all")
            .map(|s| s == "true" || s == "1")
            .unwrap_or(false)
        || qmap.contains_key("all");
    let tag = d
        .tag
        .as_deref()
        .or_else(|| qmap.get("tag").map(String::as_str));
    let group = d
        .group
        .as_deref()
        .or_else(|| qmap.get("group").map(String::as_str));
    let dev_type = d
        .device_type
        .as_deref()
        .or(d.r#type.as_deref())
        .or_else(|| {
            qmap.get("type")
                .or_else(|| qmap.get("device_type"))
                .map(String::as_str)
        });
    let peer = d
        .peer
        .as_deref()
        .or_else(|| qmap.get("peer").map(String::as_str));

    let selector =
        match crate::store::TargetSelector::from_query(peer, tag, group, dev_type, is_all) {
            Ok(s) => s,
            Err(e) => return err(400, &e),
        };

    if let Some(b64) = d.data_base64 {
        use base64::Engine;
        let bytes = match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
            Ok(b) => b,
            Err(_) => return err(400, "bad base64 in data_base64"),
        };
        let filename = d.name.as_deref().unwrap_or("uploaded_file");
        match node
            .offer_and_send_bytes_multi(&selector, filename, &bytes)
            .await
        {
            Ok(report) => return json_ok(&report),
            Err(e) => return err(400, &e.to_string()),
        }
    }

    let path_opt = d
        .path
        .as_deref()
        .or_else(|| qmap.get("path").map(String::as_str));
    let Some(path_str) = path_opt else {
        return err(400, "missing required 'path' or 'data_base64' parameter");
    };
    let p = std::path::Path::new(path_str);
    if !p.is_file() {
        return err(400, "file not found or not a regular file");
    }

    let name_opt = d
        .name
        .as_deref()
        .or_else(|| qmap.get("name").map(String::as_str));
    match node.offer_and_send_file_multi(&selector, p, name_opt).await {
        Ok(report) => json_ok(&report),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_file_send(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<SendFileBody>(body) {
        Ok(d) => {
            let qmap = query_str.map(parse_query_map).unwrap_or_default();
            let is_all = d.all.unwrap_or(false)
                || qmap
                    .get("all")
                    .map(|s| s == "true" || s == "1")
                    .unwrap_or(false)
                || qmap.contains_key("all");
            let tag = d
                .tag
                .as_deref()
                .or_else(|| qmap.get("tag").map(String::as_str));
            let group = d
                .group
                .as_deref()
                .or_else(|| qmap.get("group").map(String::as_str));
            let dev_type = d
                .device_type
                .as_deref()
                .or(d.r#type.as_deref())
                .or_else(|| {
                    qmap.get("type")
                        .or_else(|| qmap.get("device_type"))
                        .map(String::as_str)
                });

            if is_all || tag.is_some() || group.is_some() || dev_type.is_some() {
                let peer = d
                    .peer
                    .as_deref()
                    .or_else(|| qmap.get("peer").map(String::as_str));
                let selector = match crate::store::TargetSelector::from_query(
                    peer, tag, group, dev_type, is_all,
                ) {
                    Ok(s) => s,
                    Err(e) => return err(400, &e),
                };
                if let Some(b64) = d.data_base64 {
                    use base64::Engine;
                    let bytes =
                        match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes()) {
                            Ok(b) => b,
                            Err(_) => return err(400, "bad base64 in data_base64"),
                        };
                    let filename = d.name.as_deref().unwrap_or("uploaded_file");
                    match node
                        .offer_and_send_bytes_multi(&selector, filename, &bytes)
                        .await
                    {
                        Ok(report) => return json_ok(&report),
                        Err(e) => return err(400, &e.to_string()),
                    }
                }
                let path_opt = d
                    .path
                    .as_deref()
                    .or_else(|| qmap.get("path").map(String::as_str));
                let Some(path_str) = path_opt else {
                    return err(
                        400,
                        "missing required 'path' or 'data_base64' parameter for targeted dispatch",
                    );
                };
                let p = std::path::Path::new(path_str);
                if !p.is_file() {
                    return err(400, "file not found or not a regular file");
                }
                let name_opt = d
                    .name
                    .as_deref()
                    .or_else(|| qmap.get("name").map(String::as_str));
                match node.offer_and_send_file_multi(&selector, p, name_opt).await {
                    Ok(report) => return json_ok(&report),
                    Err(e) => return err(400, &e.to_string()),
                }
            }

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

            if let Some(ref path_str) = d.path {
                let p = std::path::Path::new(path_str);
                if !p.is_file() {
                    return err(400, "file not found or not a regular file");
                }
                let meta = match std::fs::metadata(p) {
                    Ok(m) => m,
                    Err(e) => return err(400, &format!("cannot read file metadata: {e}")),
                };
                if meta.len() > tetherly_core::FILE_MAX {
                    return err(400, "file exceeds FILE_MAX (64 GiB)");
                }
                let size = meta.len();
                match node
                    .offer_and_send_file(&target, p, d.name.as_deref())
                    .await
                {
                    Ok(transfer_id) => {
                        let fname = d.name.clone().unwrap_or_else(|| {
                            p.file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("file.bin")
                                .to_string()
                        });
                        let safe_name = match tetherly_core::sanitize_file_name(&fname) {
                            Ok(n) => n,
                            Err(e) => return err(400, &format!("invalid file name: {e}")),
                        };
                        json_ok(&serde_json::json!({
                            "ok": true,
                            "transfer_id": transfer_id,
                            "name": safe_name,
                            "size": size,
                            "peer": target.as_str(),
                        }))
                    }
                    Err(e) => err(400, &e.to_string()),
                }
            } else if let Some(ref b64) = d.data_base64 {
                use base64::Engine;
                let decoded = match base64::engine::general_purpose::STANDARD.decode(b64.as_bytes())
                {
                    Ok(b) => b,
                    Err(e) => return err(400, &format!("invalid base64: {e}")),
                };
                if decoded.len() as u64 > tetherly_core::FILE_MAX {
                    return err(400, "file exceeds FILE_MAX (64 GiB)");
                }
                let fname = d.name.clone().unwrap_or_else(|| "upload.bin".to_string());
                let safe_name = match tetherly_core::sanitize_file_name(&fname) {
                    Ok(n) => n,
                    Err(e) => return err(400, &format!("invalid file name: {e}")),
                };
                let digest = tetherly_core::hex_sha256(&decoded);
                let size = decoded.len() as u64;
                match node
                    .offer_and_send_bytes(&target, &safe_name, &decoded, false)
                    .await
                {
                    Ok(transfer_id) => json_ok(&serde_json::json!({
                        "ok": true,
                        "transfer_id": transfer_id,
                        "name": safe_name,
                        "size": size,
                        "sha256": digest,
                        "peer": target.as_str(),
                    })),
                    Err(e) => err(400, &e.to_string()),
                }
            } else {
                err(400, "either path or data_base64 must be provided")
            }
        }
        Err(_) => err(400, "bad json"),
    }
}

pub(super) async fn post_file_send_batch(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<crate::runtime::BatchSendRequest>(body) {
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

            let mut all_items = d.items;
            for p in d.paths {
                all_items.push(crate::runtime::BatchSendItem {
                    path: Some(p),
                    name: None,
                    data_base64: None,
                });
            }
            if all_items.is_empty() {
                return err(400, "no files specified in paths or items");
            }
            match node.send_batch_items(&target, all_items).await {
                Ok(rep) => json_ok(&rep),
                Err(e) => err(400, &e.to_string()),
            }
        }
        Err(_) => err(
            400,
            "bad json, expected { peer?: string, paths?: string[], items?: [] }",
        ),
    }
}

pub(super) async fn get_file_resume(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.resume_markers_report();
    json_ok(&rep)
}

pub(super) async fn get_storage(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.storage_report();
    json_ok(&rep)
}

pub(super) async fn post_storage_clean(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let orphaned_only = if body.is_empty() {
        true
    } else {
        match serde_json::from_slice::<StorageCleanBody>(body) {
            Ok(d) => d.orphaned_only.unwrap_or(true),
            Err(_) => return err(400, "bad json"),
        }
    };
    let rep = node.clean_storage(orphaned_only);
    json_ok(&rep)
}

pub(super) async fn get_file_inbox(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let mut search: Option<String> = None;
    let mut limit: Option<usize> = None;
    if let Some(q) = query_str {
        let map = parse_query_map(q);
        if let Some(s) = map.get("search").or_else(|| map.get("q")) {
            search = Some(s.clone());
        }
        if let Some(l) = map.get("limit").and_then(|s| s.parse::<usize>().ok()) {
            limit = Some(l);
        }
    }
    let rep = node.inbox_report(search.as_deref(), limit);
    json_ok(&rep)
}

pub(super) async fn post_file_inbox_delete(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let name = qmap.get("name").cloned().or_else(|| {
        if !body.is_empty() {
            serde_json::from_slice::<InboxDeleteBody>(body)
                .ok()
                .and_then(|b| b.name)
        } else {
            None
        }
    });
    if let Some(name) = name.filter(|s| !s.trim().is_empty()) {
        match node.delete_inbox_file(&name) {
            Ok(rep) => json_ok(&rep),
            Err(tetherly_core::CoreError::File(ref msg)) if msg.contains("not found") => {
                err(404, "file not found in inbox")
            }
            Err(e) => err(400, &e.to_string()),
        }
    } else {
        err(400, "missing required 'name' in query or body")
    }
}

pub(super) async fn post_file_inbox_clear(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let before_secs = qmap
        .get("before_secs")
        .and_then(|s| s.parse::<u64>().ok())
        .or_else(|| {
            if !body.is_empty() {
                serde_json::from_slice::<InboxClearBody>(body)
                    .ok()
                    .and_then(|b| b.before_secs)
            } else {
                None
            }
        });
    match node.clear_inbox(before_secs) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_file_inbox_download(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let name = qmap
        .get("name")
        .or_else(|| qmap.get("file"))
        .map(|s| s.trim());
    match name {
        Some(n) if !n.is_empty() => match node.read_inbox_file(n) {
            Ok((clean_name, mime, bytes)) => UiResponse::new(200, mime, bytes).with_header(
                "Content-Disposition",
                format!("attachment; filename=\"{clean_name}\""),
            ),
            Err(tetherly_core::CoreError::UnsafeFileName) => err(400, "unsafe file name"),
            Err(tetherly_core::CoreError::File(ref msg)) if msg.contains("not found") => {
                err(404, "file not found in inbox")
            }
            Err(tetherly_core::CoreError::FileTooLarge(sz)) => {
                err(413, &format!("file too large ({sz} bytes)"))
            }
            Err(e) => err(400, &e.to_string()),
        },
        _ => err(400, "missing required 'name' query parameter"),
    }
}

pub(super) async fn get_file_inbox_raw(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let name = qmap
        .get("name")
        .or_else(|| qmap.get("file"))
        .map(|s| s.trim());
    match name {
        Some(n) if !n.is_empty() => match node.read_inbox_file(n) {
            Ok((clean_name, mime, bytes)) => UiResponse::new(200, mime, bytes).with_header(
                "Content-Disposition",
                format!("inline; filename=\"{clean_name}\""),
            ),
            Err(tetherly_core::CoreError::UnsafeFileName) => err(400, "unsafe file name"),
            Err(tetherly_core::CoreError::File(ref msg)) if msg.contains("not found") => {
                err(404, "file not found in inbox")
            }
            Err(tetherly_core::CoreError::FileTooLarge(sz)) => {
                err(413, &format!("file too large ({sz} bytes)"))
            }
            Err(e) => err(400, &e.to_string()),
        },
        _ => err(400, "missing required 'name' query parameter"),
    }
}

pub(super) async fn post_file_inbox_open(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let name = qmap.get("name").cloned().or_else(|| {
        if !body.is_empty() {
            serde_json::from_slice::<InboxOpenBody>(body)
                .ok()
                .and_then(|b| b.name)
        } else {
            None
        }
    });
    let op = node.opener();
    match node.open_inbox_file(name.as_deref(), op.as_ref()) {
        Ok(rep) => json_ok(&rep),
        Err(tetherly_core::CoreError::UnsafeFileName) => err(400, "unsafe file name"),
        Err(tetherly_core::CoreError::File(ref msg)) if msg.contains("not found") => {
            err(404, "file not found in inbox")
        }
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_file_outbox(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let mut q_peer: Option<String> = None;
    let mut q_status: Option<OutboxStatus> = None;
    let mut q_limit: Option<usize> = None;

    if let Some(q) = query_str {
        let map = parse_query_map(q);
        if let Some(p) = map
            .get("peer")
            .or_else(|| map.get("to"))
            .or_else(|| map.get("target"))
        {
            q_peer = Some(p.clone());
        }
        if let Some(s) = map.get("status") {
            match s.parse::<OutboxStatus>() {
                Ok(st) => q_status = Some(st),
                Err(e) => return err(400, &e),
            }
        }
        if let Some(l) = map.get("limit").and_then(|s| s.parse::<usize>().ok()) {
            q_limit = Some(l);
        }
    }

    if !body.is_empty() {
        match serde_json::from_slice::<OutboxQueryBody>(body) {
            Ok(b) => {
                if b.peer.is_some() {
                    q_peer = b.peer;
                }
                if let Some(ref st_str) = b.status {
                    match st_str.parse::<OutboxStatus>() {
                        Ok(st) => q_status = Some(st),
                        Err(e) => return err(400, &e),
                    }
                }
                if b.limit.is_some() {
                    q_limit = b.limit;
                }
            }
            Err(_) => return err(400, "bad json"),
        }
    }

    match node.outbox_history(q_peer.as_deref(), q_status, q_limit) {
        Ok(report) => json_ok(&report),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_file_outbox_clear(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let mut q_peer: Option<String> = None;
    let mut q_status: Option<OutboxStatus> = None;

    if let Some(q) = query_str {
        let map = parse_query_map(q);
        if let Some(p) = map
            .get("peer")
            .or_else(|| map.get("to"))
            .or_else(|| map.get("target"))
        {
            q_peer = Some(p.clone());
        }
        if let Some(s) = map.get("status") {
            match s.parse::<OutboxStatus>() {
                Ok(st) => q_status = Some(st),
                Err(e) => return err(400, &e),
            }
        }
    }

    if !body.is_empty() {
        match serde_json::from_slice::<OutboxClearBody>(body) {
            Ok(b) => {
                if b.peer.is_some() {
                    q_peer = b.peer;
                }
                if let Some(ref st_str) = b.status {
                    match st_str.parse::<OutboxStatus>() {
                        Ok(st) => q_status = Some(st),
                        Err(e) => return err(400, &e),
                    }
                }
            }
            Err(_) => return err(400, "bad json"),
        }
    }

    match node.clear_outbox(q_peer.as_deref(), q_status) {
        Ok(report) => json_ok(&report),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_file_outbox_resend(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let mut target_id: Option<String> = None;
    if let Some(q) = query_str {
        let map = parse_query_map(q);
        if let Some(id) = map.get("id").or_else(|| map.get("transfer_id")) {
            target_id = Some(id.clone());
        }
    }

    if !body.is_empty() {
        match serde_json::from_slice::<OutboxResendBody>(body) {
            Ok(b) => {
                if b.id.is_some() {
                    target_id = b.id;
                } else if b.transfer_id.is_some() {
                    target_id = b.transfer_id;
                }
            }
            Err(_) => return err(400, "bad json"),
        }
    }

    let Some(id) = target_id else {
        return err(400, "missing required transfer or outbox id");
    };

    match node.resend_outbox_file(&id).await {
        Ok(report) => json_ok(&report),
        Err(tetherly_core::CoreError::UnknownTransfer) => err(404, "outbox entry not found"),
        Err(e) => err(400, &e.to_string()),
    }
}
