// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Hit counters for the frozen `/api/*` routes (ADR-139 §9): which legacy
//! routes are still called, and by what kind of client, so they can be
//! retired once their callers have moved to `/api/v1`.

use std::collections::BTreeMap;
use std::sync::Mutex;
use tetherly_api::LegacyRouteHits;

/// Distinct (method, route, caller) keys kept; further keys fold into
/// [`OVERFLOW_ROUTE`] so a scanner cannot grow the map without bound.
pub const MAX_KEYS: usize = 256;
pub const OVERFLOW_ROUTE: &str = "(other)";

type Key = (String, String, &'static str);

#[derive(Default)]
pub struct LegacyRoutes {
    hits: Mutex<BTreeMap<Key, u64>>,
}

impl LegacyRoutes {
    /// Count one request if `path` is a legacy `/api/*` route.
    pub fn record(&self, method: &str, path: &str, user_agent: Option<&str>) {
        let path = path.split_once('?').map_or(path, |(p, _)| p);
        if !path.starts_with("/api/") || path.starts_with("/api/v1/") {
            return;
        }
        let caller = caller_class(user_agent);
        let mut key: Key = (method.to_string(), route_key(path), caller);
        let mut hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        if !hits.contains_key(&key) && hits.len() >= MAX_KEYS {
            key = ("*".to_string(), OVERFLOW_ROUTE.to_string(), "other");
        }
        *hits.entry(key).or_default() += 1;
    }

    /// All counters, ordered by route, then method, then caller.
    pub fn snapshot(&self) -> Vec<LegacyRouteHits> {
        let hits = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<LegacyRouteHits> = hits
            .iter()
            .map(|((method, route, caller), count)| LegacyRouteHits {
                method: method.clone(),
                route: route.clone(),
                caller: (*caller).to_string(),
                count: *count,
            })
            .collect();
        out.sort_by(|a, b| (&a.route, &a.method, &a.caller).cmp(&(&b.route, &b.method, &b.caller)));
        out
    }
}

/// Route template: the query is dropped and the first segment that is not a
/// plain `[a-z-]` route word (a file name, an id) and everything after it
/// collapse to `*`. Every legacy route path is made of such words.
pub fn route_key(path: &str) -> String {
    let path = path.split_once('?').map_or(path, |(p, _)| p);
    let mut out = String::new();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        out.push('/');
        if !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
            out.push_str(seg);
        } else {
            out.push('*');
            break;
        }
    }
    out
}

/// Coarse client kind from the `User-Agent` header. The bundled CLI sends
/// none (or a `tetherly` one).
pub fn caller_class(user_agent: Option<&str>) -> &'static str {
    let Some(ua) = user_agent.map(str::to_ascii_lowercase) else {
        return "cli";
    };
    if ua.starts_with("tetherly") {
        "cli"
    } else if ua.contains("okhttp") || ua.contains("dalvik") || ua.contains("android") {
        "android"
    } else if ua.starts_with("mozilla") {
        "browser"
    } else if ua.starts_with("curl") {
        "curl"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callers_are_classified_by_user_agent() {
        assert_eq!(caller_class(None), "cli");
        assert_eq!(caller_class(Some("Mozilla/5.0 (Macintosh)")), "browser");
        assert_eq!(caller_class(Some("okhttp/4.12.0")), "android");
        assert_eq!(
            caller_class(Some("Dalvik/2.1.0 (Linux; U; Android 14)")),
            "android"
        );
        assert_eq!(caller_class(Some("curl/8.7.1")), "curl");
        assert_eq!(caller_class(Some("tetherly-cli/0.2")), "cli");
        assert_eq!(caller_class(Some("python-requests")), "other");
    }

    #[test]
    fn only_legacy_api_paths_count() {
        let l = LegacyRoutes::default();
        l.record("GET", "/api/peers", None);
        l.record("GET", "/api/peers", None);
        l.record("GET", "/api/v1/devices", None);
        l.record("GET", "/", Some("Mozilla/5.0"));
        l.record("GET", "/api/events", Some("Mozilla/5.0"));
        let hits = l.snapshot();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].route, "/api/events");
        assert_eq!(hits[0].caller, "browser");
        assert_eq!(hits[1].route, "/api/peers");
        assert_eq!(hits[1].count, 2);
    }

    #[test]
    fn route_keys_drop_query_and_deep_segments() {
        assert_eq!(route_key("/api/peers?x=1"), "/api/peers");
        assert_eq!(route_key("/api/inbox/some%20file.pdf"), "/api/inbox/*");
        assert_eq!(
            route_key("/api/input/tunnel/release"),
            "/api/input/tunnel/release"
        );
        assert_eq!(
            route_key("/api/screen/session/s_1a2b/frame"),
            "/api/screen/session/*"
        );
    }

    #[test]
    fn distinct_keys_are_capped() {
        let l = LegacyRoutes::default();
        for i in 0..(MAX_KEYS + 10) {
            let name: String = format!("{i:03}")
                .bytes()
                .map(|b| (b - b'0' + b'a') as char)
                .collect();
            l.record("GET", &format!("/api/{name}"), None);
        }
        let hits = l.snapshot();
        assert_eq!(hits.len(), MAX_KEYS + 1);
        let overflow = hits.iter().find(|h| h.route == OVERFLOW_ROUTE).unwrap();
        assert_eq!(overflow.count, 10);
    }
}
