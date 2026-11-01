// SPDX-License-Identifier: Apache-2.0 OR MIT
//! ANCS (iPhone notification) bridge configuration and status.

use super::*;

// Step 1 of the phone wizard: the user connected the iPhone in the OS
// Bluetooth settings, so we only have to subscribe to ANCS here.
pub(super) async fn get_ancs(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.ancs_report();
    json_ok(&rep)
}

pub(super) async fn post_ancs_connect(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.ancs_connect_reported() {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_ancs_disconnect(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.ancs_disconnect_reported();
    json_ok(&rep)
}

pub(super) async fn post_ancs_allow(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    match serde_json::from_slice::<OpenRuleBody>(body) {
        Ok(d) => match node.add_open_rule_reported(tetherly_core::OpenRule {
            app_id: d.app_id,
            url: d.url,
        }) {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(400, &e.to_string()),
        },
        Err(_) => err(400, "bad json, expected { app_id: string, url?: string }"),
    }
}

pub(super) async fn delete_ancs_allow(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let app_id = qmap.get("app_id").cloned().or_else(|| {
        if !body.is_empty() {
            serde_json::from_slice::<OpenRuleRemoveBody>(body)
                .ok()
                .and_then(|b| b.app_id)
        } else {
            None
        }
    });
    if let Some(app_id) = app_id.filter(|s| !s.trim().is_empty()) {
        let rep = node.remove_open_rule_reported(&app_id);
        json_ok(&rep)
    } else {
        err(400, "missing required 'app_id' in query or body")
    }
}

pub(super) async fn post_ancs_rules_clear(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.clear_open_rules_reported();
    json_ok(&rep)
}
