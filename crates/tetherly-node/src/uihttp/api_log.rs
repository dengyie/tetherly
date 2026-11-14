// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Daemon log status, tailing and live filter control (v0.2.49).
//!
//! - `GET  /api/log`        — path, sizes, active / baseline filter.
//! - `GET  /api/log/tail`   — `lines` (≤2000, default 200), `grep`
//!   (case-insensitive substring), `level` (minimum: error|warn|info|…).
//! - `POST /api/log/level`  — `{"filter":"tetherly=debug","revert_after_secs":600}`
//!   or `{"reset":true}`. Mutating, so it sits behind the router's
//!   loopback Origin check like every other POST.

use super::*;

pub(super) async fn get_log(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    _node: &Node,
) -> UiResponse {
    json_ok(&crate::logging::status())
}

pub(super) async fn get_log_tail(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    _node: &Node,
) -> UiResponse {
    let Some(path) = crate::logging::log_path() else {
        return err(503, "file logging not active");
    };
    let map = query_str.map(parse_query_map).unwrap_or_default();
    let lines = map
        .get("lines")
        .or_else(|| map.get("n"))
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(200);
    let min_level = match map.get("level") {
        Some(l) => match crate::logging::parse_level(l) {
            Some(lv) => Some(lv),
            None => return err(400, "level must be error|warn|info|debug|trace"),
        },
        None => None,
    };
    let q = crate::logging::TailQuery {
        lines,
        grep: map.get("grep").filter(|g| !g.is_empty()).cloned(),
        min_level,
    };
    let path = path.to_path_buf();
    // File I/O off the async workers.
    match tokio::task::spawn_blocking(move || crate::logging::tail(&path, &q)).await {
        Ok(Ok(lines)) => json_ok(&serde_json::json!({ "count": lines.len(), "lines": lines })),
        Ok(Err(e)) => err(500, &format!("read log: {e}")),
        Err(_) => err(500, "tail task failed"),
    }
}

#[derive(serde::Deserialize, Default)]
struct LevelBody {
    filter: Option<String>,
    #[serde(default)]
    reset: bool,
    revert_after_secs: Option<u64>,
}

pub(super) async fn post_log_level(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    _node: &Node,
) -> UiResponse {
    let Some(ctl) = crate::logging::control() else {
        return err(503, "logging control not active");
    };
    let req: LevelBody = match serde_json::from_slice(body) {
        Ok(r) => r,
        Err(_) => return err(400, "body must be {\"filter\":\"...\"} or {\"reset\":true}"),
    };
    let result = if req.reset {
        ctl.reset().map(|_| ())
    } else if let Some(spec) = req.filter.as_deref() {
        crate::logging::set_filter_with_revert(
            ctl,
            spec,
            req.revert_after_secs
                .filter(|s| *s > 0)
                .map(std::time::Duration::from_secs),
            node_clock_ms(),
        )
    } else {
        Err("missing filter".to_string())
    };
    match result {
        Ok(()) => json_ok(&crate::logging::status()),
        Err(e) => err(400, &e),
    }
}
