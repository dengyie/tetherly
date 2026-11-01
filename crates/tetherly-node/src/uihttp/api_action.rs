// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Local action definitions, triggers, history and action policy.

use super::*;

pub(super) async fn get_action_local(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.local_action_report())
}

pub(super) async fn post_action_local(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ActionLocalBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<ActionLocalBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected ActionLocalBody"),
        }
    };

    let id = parsed_body
        .as_ref()
        .and_then(|b| b.id.clone())
        .or_else(|| qmap.get("id").cloned());
    let Some(action_id) = id.filter(|s| !s.trim().is_empty()) else {
        return err(400, "missing 'id' for action");
    };

    let name = parsed_body
        .as_ref()
        .and_then(|b| b.name.clone())
        .or_else(|| qmap.get("name").cloned())
        .unwrap_or_else(|| action_id.clone());

    let description = parsed_body
        .as_ref()
        .and_then(|b| b.description.clone())
        .or_else(|| {
            qmap.get("description")
                .or_else(|| qmap.get("desc"))
                .cloned()
        });

    let icon = parsed_body
        .as_ref()
        .and_then(|b| b.icon.clone())
        .or_else(|| qmap.get("icon").cloned());

    let command = parsed_body
        .as_ref()
        .and_then(|b| b.command.clone())
        .or_else(|| qmap.get("command").or_else(|| qmap.get("cmd")).cloned());
    let Some(cmd) = command.filter(|s| !s.trim().is_empty()) else {
        return err(400, "missing 'command' for action");
    };

    let args = parsed_body
        .as_ref()
        .and_then(|b| b.args.clone())
        .or_else(|| {
            qmap.get("args").map(|s| {
                s.split(',')
                    .map(|arg| arg.trim().to_string())
                    .filter(|arg| !arg.is_empty())
                    .collect()
            })
        })
        .unwrap_or_default();

    let timeout_secs = parsed_body
        .as_ref()
        .and_then(|b| b.timeout_secs)
        .or_else(|| {
            qmap.get("timeout_secs")
                .or_else(|| qmap.get("timeout"))
                .and_then(|s| s.parse::<u32>().ok())
        })
        .unwrap_or(10);

    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|s| s.parse::<bool>().ok()))
        .unwrap_or(true);

    let def = LocalActionDefinition {
        id: action_id,
        name,
        description,
        icon,
        command: cmd,
        args,
        timeout_secs,
        enabled,
    };

    match node.add_local_action(def) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn delete_action_local(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ActionLocalBody> = if body.is_empty() {
        None
    } else {
        serde_json::from_slice::<ActionLocalBody>(body).ok()
    };

    let id = parsed_body
        .as_ref()
        .and_then(|b| b.id.clone())
        .or_else(|| qmap.get("id").cloned());
    let Some(action_id) = id.filter(|s| !s.trim().is_empty()) else {
        return err(400, "missing 'id' for action to delete");
    };

    match node.remove_local_action(&action_id) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_action_remote(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let peer = qmap.get("peer").or_else(|| qmap.get("target"));
    let Some(peer_target) = peer.filter(|p| !p.trim().is_empty()) else {
        return err(400, "missing target peer (specify 'peer' or 'target')");
    };
    match node.remote_actions(peer_target) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_action_remote_refresh(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ActionTriggerBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<ActionTriggerBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected ActionTriggerBody"),
        }
    };
    let peer = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone())
        .or_else(|| qmap.get("peer").cloned())
        .or_else(|| qmap.get("target").cloned());
    let Some(peer_target) = peer.filter(|p| !p.trim().is_empty()) else {
        return err(400, "missing target peer (specify 'peer' or 'target')");
    };
    match node.query_remote_actions(&peer_target).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_action_trigger(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ActionTriggerBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<ActionTriggerBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected ActionTriggerBody"),
        }
    };

    let peer = parsed_body
        .as_ref()
        .and_then(|b| b.peer.clone())
        .or_else(|| qmap.get("peer").cloned())
        .or_else(|| qmap.get("target").cloned());
    let Some(peer_target) = peer.filter(|p| !p.trim().is_empty()) else {
        return err(400, "missing target peer (specify 'peer' or 'target')");
    };

    let action_id = parsed_body
        .as_ref()
        .and_then(|b| b.action_id.clone())
        .or_else(|| qmap.get("action_id").cloned())
        .or_else(|| qmap.get("id").cloned());
    let Some(act_id) = action_id.filter(|s| !s.trim().is_empty()) else {
        return err(400, "missing 'action_id' (or 'id')");
    };

    match node.trigger_remote_action(&peer_target, &act_id).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_action_history(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.action_history())
}

pub(super) async fn get_action_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.action_policy_report())
}

pub(super) async fn post_action_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ActionPolicyBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<ActionPolicyBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected ActionPolicyBody"),
        }
    };

    let mut policy = node.action_policy();
    if let Some(ref b) = parsed_body {
        if let Some(ref m_str) = b.mode {
            match m_str.parse::<ActionAccessMode>() {
                Ok(m) => policy.mode = m,
                Err(e) => return err(400, &format!("invalid action mode: {e}")),
            }
        }
        if let Some(timeout) = b.max_timeout_secs {
            policy.max_timeout_secs = ActionValidator::clamp_timeout(timeout);
        }
        if let Some(exp) = b.expose_action_list {
            policy.expose_action_list = exp;
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
        if b.clear_filters == Some(true) {
            policy.allowed_groups.clear();
            policy.allowed_tags.clear();
            policy.allowed_types.clear();
        }
    }

    if let Some(m_str) = qmap.get("mode") {
        match m_str.parse::<ActionAccessMode>() {
            Ok(m) => policy.mode = m,
            Err(e) => return err(400, &format!("invalid action mode: {e}")),
        }
    }
    if let Some(t_str) = qmap.get("timeout").or_else(|| qmap.get("max_timeout_secs")) {
        if let Ok(t) = t_str.parse::<u32>() {
            policy.max_timeout_secs = ActionValidator::clamp_timeout(t);
        }
    }
    if let Some(exp_str) = qmap
        .get("expose")
        .or_else(|| qmap.get("expose_action_list"))
    {
        if let Ok(exp) = exp_str.parse::<bool>() {
            policy.expose_action_list = exp;
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
    if qmap
        .get("clear_filters")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false)
    {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }

    match node.set_action_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
