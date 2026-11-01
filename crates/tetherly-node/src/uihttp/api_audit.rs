// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Security audit trail and audit queries.

use super::*;

pub(super) async fn get_audit(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let mut q_cat: Option<String> = None;
    let mut q_sev: Option<String> = None;
    let mut q_lim: Option<usize> = None;
    let mut q_since: Option<u64> = None;

    if let Some(q) = query_str {
        let map = parse_query_map(q);
        if let Some(c) = map.get("category").or_else(|| map.get("cat")) {
            q_cat = Some(c.clone());
        }
        if let Some(s) = map.get("severity").or_else(|| map.get("sev")) {
            q_sev = Some(s.clone());
        }
        if let Some(l) = map.get("limit").and_then(|s| s.parse::<usize>().ok()) {
            q_lim = Some(l);
        }
        if let Some(since) = map
            .get("since")
            .or_else(|| map.get("since_ms"))
            .and_then(|s| s.parse::<u64>().ok())
        {
            q_since = Some(since);
        }
    }

    if !body.is_empty() {
        match serde_json::from_slice::<AuditQueryBody>(body) {
            Ok(b) => {
                if b.category.is_some() {
                    q_cat = b.category;
                }
                if b.severity.is_some() {
                    q_sev = b.severity;
                }
                if b.limit.is_some() {
                    q_lim = b.limit;
                }
                if b.since.is_some() {
                    q_since = b.since;
                }
            }
            Err(_) => return err(400, "bad json"),
        }
    }

    let cat = match q_cat.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(c) => match c.parse::<crate::runtime::AuditCategory>() {
            Ok(cat) => Some(cat),
            Err(e) => return err(400, &e),
        },
        None => None,
    };

    let sev = match q_sev.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(s) => match s.parse::<crate::runtime::AuditSeverity>() {
            Ok(sev) => Some(sev),
            Err(e) => return err(400, &e),
        },
        None => None,
    };

    let report = node.audit_report(cat, sev, q_lim, q_since);
    json_ok(&report)
}

pub(super) async fn post_audit_clear(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let report = node.clear_audit_log();
    json_ok(&report)
}
