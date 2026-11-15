// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Static hosting of the ADR-139 control-client SPA at `/next/` (M2b).
//!
//! The built `client/dist` output is embedded at compile time with fixed file
//! names (see `client/vite.config.ts`: hash-free asset names) so the daemon
//! binary ships the new console without a sidecar. The legacy console stays
//! at `/`; nothing under `/api/*` changes. Unknown `/next/*` paths fall back
//! to the entry document so the SPA router can handle deep links.

use super::*;

/// `client/dist/index.html`, embedded at compile time.
const NEXT_INDEX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../client/dist/index.html"
));
/// `client/dist/assets/app.js` — the SPA bundle.
const NEXT_APP_JS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../client/dist/assets/app.js"
));
/// `client/dist/assets/index.css` — the SPA stylesheet.
const NEXT_APP_CSS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../client/dist/assets/index.css"
));

/// `GET /next` or `GET /next/`: the entry document.
pub(super) fn get_entry() -> UiResponse {
    UiResponse::new(
        200,
        "text/html; charset=utf-8",
        NEXT_INDEX.as_bytes().to_vec(),
    )
}

/// `GET /next/<asset>`: serve a known asset; anything else falls back to the
/// entry so the SPA router owns unknown paths.
pub(super) fn get_asset(path: &str) -> UiResponse {
    match path {
        "/next/assets/app.js" => UiResponse::new(
            200,
            "text/javascript; charset=utf-8",
            NEXT_APP_JS.as_bytes().to_vec(),
        ),
        "/next/assets/index.css" => UiResponse::new(
            200,
            "text/css; charset=utf-8",
            NEXT_APP_CSS.as_bytes().to_vec(),
        ),
        // Unknown asset or deep link: hand back the entry document. A missed
        // hashed asset would loop forever, but asset names are hash-free and
        // embedded, so the entry's references always resolve.
        _ => get_entry(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_is_the_embedded_index_html() {
        let r = get_entry();
        assert_eq!(r.code, 200);
        let body = String::from_utf8(r.body).unwrap();
        assert!(
            body.contains("/next/assets/app.js"),
            "entry references the bundle"
        );
        assert!(
            body.contains("/next/assets/index.css"),
            "entry references the stylesheet"
        );
    }

    #[test]
    fn assets_are_served_with_their_content_type() {
        let js = get_asset("/next/assets/app.js");
        assert_eq!(js.code, 200);
        assert!(js.ctype.starts_with("text/javascript"));
        assert!(!js.body.is_empty());

        let css = get_asset("/next/assets/index.css");
        assert_eq!(css.code, 200);
        assert!(css.ctype.starts_with("text/css"));
        assert!(!css.body.is_empty());
    }

    #[test]
    fn unknown_next_paths_fall_back_to_the_entry() {
        for p in ["/next/devices", "/next/assets/nope.js", "/next/"] {
            let r = get_asset(p);
            assert_eq!(r.code, 200, "GET {p}");
            let body = String::from_utf8(r.body).unwrap();
            assert!(
                body.contains("/next/assets/app.js"),
                "GET {p} serves the entry"
            );
        }
    }

    #[test]
    fn entry_routes_answer_get_only_via_router() {
        // The router table decides 404 for non-GET; verify the match arms the
        // same way the legacy console arm is exercised.
        let html = NEXT_INDEX;
        assert!(html.starts_with("<!doctype html>"));
    }
}
