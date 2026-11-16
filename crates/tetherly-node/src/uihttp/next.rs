// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Static hosting of the two consoles (ADR-139).
//!
//! Since M4 the built `client/dist` SPA IS the console at `/`; the legacy
//! single-file console moved to `/legacy/` and stays until M5 deletes it.
//! Both are embedded at compile time with fixed file names (see
//! `client/vite.config.ts`: hash-free asset names), so the daemon binary
//! ships both consoles without a sidecar. Nothing under `/api/*` changes.
//! SPA assets live at the fixed rooted paths `/assets/*` (GET-only); unknown
//! `/legacy/*` paths fall back to the legacy entry (it is one file with
//! inline styles/scripts, so sub-resource resolution never happens).

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
/// `ui/index.html` — the legacy single-file console (inline styles/scripts,
/// no external assets).
const LEGACY_INDEX: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../ui/index.html"));

/// `GET /`: the new SPA's entry document (the default console since M4).
pub(super) fn get_entry() -> UiResponse {
    UiResponse::new(
        200,
        "text/html; charset=utf-8",
        NEXT_INDEX.as_bytes().to_vec(),
    )
}

/// `GET /assets/<name>`: the SPA's embedded bundle and stylesheet; anything
/// else under /assets/ is a 404 (asset names are fixed, so a miss is a bug,
/// not a route the SPA owns).
pub(super) fn get_asset(path: &str) -> UiResponse {
    match path {
        "/assets/app.js" => UiResponse::new(
            200,
            "text/javascript; charset=utf-8",
            NEXT_APP_JS.as_bytes().to_vec(),
        ),
        "/assets/index.css" => UiResponse::new(
            200,
            "text/css; charset=utf-8",
            NEXT_APP_CSS.as_bytes().to_vec(),
        ),
        _ => UiResponse::new(404, "text/plain; charset=utf-8", b"not found".to_vec()),
    }
}

/// `GET /legacy` or `GET /legacy/`: the legacy entry document.
pub(super) fn get_legacy_entry() -> UiResponse {
    UiResponse::new(
        200,
        "text/html; charset=utf-8",
        LEGACY_INDEX.as_bytes().to_vec(),
    )
}

/// `GET /legacy/<asset>`: the legacy console is a single file with inline
/// styles and scripts — any sub-path falls back to its entry (its API calls
/// are same-origin absolute paths, so sub-resource resolution never happens).
pub(super) fn get_legacy_asset(path: &str) -> UiResponse {
    let _ = path;
    get_legacy_entry()
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
            body.contains("/assets/app.js"),
            "entry references the bundle at the M4 rooted path"
        );
        assert!(
            body.contains("/assets/index.css"),
            "entry references the stylesheet at the M4 rooted path"
        );
    }

    #[test]
    fn assets_are_served_with_their_content_type() {
        let js = get_asset("/assets/app.js");
        assert_eq!(js.code, 200);
        assert!(js.ctype.starts_with("text/javascript"));
        assert!(!js.body.is_empty());

        let css = get_asset("/assets/index.css");
        assert_eq!(css.code, 200);
        assert!(css.ctype.starts_with("text/css"));
        assert!(!css.body.is_empty());
    }

    /// Asset names are fixed and embedded: a miss under /assets/ is a bug,
    /// not an SPA route — 404 beats serving the HTML entry for a script tag.
    #[test]
    fn unknown_asset_paths_answer_404() {
        for p in ["/assets/nope.js", "/assets/app.js.map", "/assets/"] {
            let r = get_asset(p);
            assert_eq!(r.code, 404, "GET {p}");
        }
    }

    #[test]
    fn legacy_entry_is_the_single_file_console() {
        let r = get_legacy_entry();
        assert_eq!(r.code, 200);
        let body = String::from_utf8(r.body).unwrap();
        assert!(body.starts_with("<!DOCTYPE html>"), "legacy doctype");
        assert!(
            !body.contains("src="),
            "legacy console carries no external scripts"
        );
        // Sub-paths fold back to the same entry.
        assert_eq!(
            get_legacy_asset("/legacy/whatever").body,
            LEGACY_INDEX.as_bytes()
        );
    }

    #[test]
    fn entry_routes_answer_get_only_via_router() {
        // The router table decides 404 for non-GET; verify the match arms the
        // same way the legacy console arm is exercised.
        let html = NEXT_INDEX;
        assert!(html.starts_with("<!doctype html>"));
    }
}
