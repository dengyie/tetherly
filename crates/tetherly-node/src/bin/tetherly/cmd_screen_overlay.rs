// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen overlay: watermark/annotate/whiteboard/roi/color/privacy.

use super::util::urlencode_path;
use super::*;

/// `tetherly screen privacy` -- Privacy arm of the screen dispatcher.
pub(crate) async fn screen_privacy(
    action: Option<ScreenPrivacySubCmd>,
    enable: bool,
    disable: bool,
    lock_input: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let (target_enabled, target_lock_input, is_json, port) = match action {
        Some(ScreenPrivacySubCmd::Enable {
            lock_input: sub_lock,
            json: j,
            ui_port: p,
        }) => (Some(true), sub_lock || lock_input, j || json, p),
        Some(ScreenPrivacySubCmd::Disable {
            json: j,
            ui_port: p,
        }) => (Some(false), false, j || json, p),
        Some(ScreenPrivacySubCmd::Status {
            json: j,
            ui_port: p,
        }) => (None, false, j || json, p),
        None => {
            if enable {
                (Some(true), lock_input, json, ui_port)
            } else if disable {
                (Some(false), false, json, ui_port)
            } else {
                (None, false, json, ui_port)
            }
        }
    };

    if let Some(en) = target_enabled {
        let req = serde_json::json!({
            "enabled": en,
            "lock_input": target_lock_input,
        });
        let body = serde_json::to_vec(&req)?;
        let (code, resp) = tetherly_node::loopback_post(port, "/api/screen/privacy", &body)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "screen privacy configuration failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if is_json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let rep: tetherly_core::ScreenPrivacyReport =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            println!("Screen Privacy Mode & Host Input Lockout Pipeline (ADR-098):");
            println!(
                "  Privacy Mode:         {}",
                if rep.enabled {
                    "ENABLED [Active Blanking]"
                } else {
                    "DISABLED"
                }
            );
            println!(
                "  Host Input Lockout:   {}",
                if rep.lock_input {
                    "LOCKED [Keyboard/Mouse Blocked]"
                } else {
                    "UNLOCKED"
                }
            );
            println!("  Blanking Method:      {}", rep.blanking_method);
            if rep.last_change_us > 0 {
                println!("  Last Change (us):     {}", rep.last_change_us);
            }
        }
    } else {
        let (code, resp) = tetherly_node::loopback_get(port, "/api/screen/privacy")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "screen privacy query failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if is_json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let rep: tetherly_core::ScreenPrivacyReport =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            println!("Screen Privacy Mode & Host Input Lockout Pipeline (ADR-098):");
            println!(
                "  Privacy Mode:         {}",
                if rep.enabled {
                    "ENABLED [Active Blanking]"
                } else {
                    "DISABLED"
                }
            );
            println!(
                "  Host Input Lockout:   {}",
                if rep.lock_input {
                    "LOCKED [Keyboard/Mouse Blocked]"
                } else {
                    "UNLOCKED"
                }
            );
            println!("  Blanking Method:      {}", rep.blanking_method);
            if rep.last_change_us > 0 {
                println!("  Last Change (us):     {}", rep.last_change_us);
            }
        }
    }
    Ok(())
}

/// `tetherly screen watermark` -- Watermark arm of the screen dispatcher.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn screen_watermark(
    action: Option<ScreenWatermarkSubCmd>,
    text: Option<String>,
    host_enforced: bool,
    opacity: Option<u8>,
    color: Option<String>,
    scale: Option<u8>,
    pattern: Option<String>,
    spacing_x: Option<u16>,
    spacing_y: Option<u16>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        Some(ScreenWatermarkSubCmd::Status { json, ui_port }) => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/screen/watermark")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen watermark query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_watermark_report(&rep);
            }
        }
        Some(ScreenWatermarkSubCmd::Enable {
            host_enforced,
            text,
            opacity,
            pattern,
            json,
            ui_port,
        }) => {
            let mut payload = serde_json::Map::new();
            payload.insert("enabled".to_string(), serde_json::json!(true));
            if host_enforced {
                payload.insert("host_enforced".to_string(), serde_json::json!(true));
            }
            if let Some(t) = text {
                payload.insert("text".to_string(), serde_json::json!(t));
            }
            if let Some(o) = opacity {
                payload.insert("opacity".to_string(), serde_json::json!(o));
            }
            if let Some(p) = pattern {
                payload.insert("pattern".to_string(), serde_json::json!(p));
            }
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/screen/watermark", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen watermark enable failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen security watermark enabled.");
                print_screen_watermark_report(&rep);
            }
        }
        Some(ScreenWatermarkSubCmd::Disable { json, ui_port }) => {
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/screen/watermark/disable", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen watermark disable failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen security watermark disabled.");
                print_screen_watermark_report(&rep);
            }
        }
        Some(ScreenWatermarkSubCmd::Set {
            text,
            opacity,
            color,
            scale,
            pattern,
            spacing_x,
            spacing_y,
            json,
            ui_port,
        }) => {
            let mut payload = serde_json::Map::new();
            if let Some(t) = text {
                payload.insert("text".to_string(), serde_json::json!(t));
            }
            if let Some(o) = opacity {
                payload.insert("opacity".to_string(), serde_json::json!(o));
            }
            if let Some(c) = color {
                payload.insert("color".to_string(), serde_json::json!(c));
            }
            if let Some(s) = scale {
                payload.insert("scale".to_string(), serde_json::json!(s));
            }
            if let Some(p) = pattern {
                payload.insert("pattern".to_string(), serde_json::json!(p));
            }
            if let Some(sx) = spacing_x {
                payload.insert("spacing_x".to_string(), serde_json::json!(sx));
            }
            if let Some(sy) = spacing_y {
                payload.insert("spacing_y".to_string(), serde_json::json!(sy));
            }
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(ui_port, "/api/screen/watermark", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen watermark configuration failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen security watermark configuration updated.");
                print_screen_watermark_report(&rep);
            }
        }
        None => {
            let has_flags = text.is_some()
                || host_enforced
                || opacity.is_some()
                || color.is_some()
                || scale.is_some()
                || pattern.is_some()
                || spacing_x.is_some()
                || spacing_y.is_some();
            if has_flags {
                let mut payload = serde_json::Map::new();
                if host_enforced {
                    payload.insert("host_enforced".to_string(), serde_json::json!(true));
                    payload.insert("enabled".to_string(), serde_json::json!(true));
                }
                if let Some(t) = text {
                    payload.insert("text".to_string(), serde_json::json!(t));
                }
                if let Some(o) = opacity {
                    payload.insert("opacity".to_string(), serde_json::json!(o));
                }
                if let Some(c) = color {
                    payload.insert("color".to_string(), serde_json::json!(c));
                }
                if let Some(s) = scale {
                    payload.insert("scale".to_string(), serde_json::json!(s));
                }
                if let Some(p) = pattern {
                    payload.insert("pattern".to_string(), serde_json::json!(p));
                }
                if let Some(sx) = spacing_x {
                    payload.insert("spacing_x".to_string(), serde_json::json!(sx));
                }
                if let Some(sy) = spacing_y {
                    payload.insert("spacing_y".to_string(), serde_json::json!(sy));
                }
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) =
                    tetherly_node::loopback_post(ui_port, "/api/screen/watermark", &bytes)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "screen watermark configuration failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    print_screen_watermark_report(&rep);
                }
            } else {
                let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/screen/watermark")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
                if code != 200 {
                    anyhow::bail!(
                        "screen watermark query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    print_screen_watermark_report(&rep);
                }
            }
        }
    }
    Ok(())
}

/// `tetherly screen annotate` -- Annotate arm of the screen dispatcher.
pub(crate) async fn screen_annotate(
    action: Option<ScreenAnnotateSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        Some(ScreenAnnotateSubCmd::List {
            json: j,
            ui_port: p,
        }) => {
            let is_json = j || json;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = tetherly_node::loopback_get(port, "/api/screen/annotation")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen annotation query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_annotation_report(&rep);
            }
        }
        Some(ScreenAnnotateSubCmd::Clear {
            json: j,
            ui_port: p,
        }) => {
            let is_json = j || json;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) =
                tetherly_node::loopback_post(port, "/api/screen/annotation/clear", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen annotation clear failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("All collaborative whiteboard annotations cleared.");
                print_screen_annotation_report(&rep);
            }
        }
        Some(ScreenAnnotateSubCmd::Add { shape }) => {
            let mut payload = serde_json::Map::new();
            let (is_json, port) = match shape {
                ScreenAnnotateShapeSubCmd::Stroke {
                    points,
                    author,
                    color,
                    thickness,
                    alpha,
                    ttl,
                    json: j,
                    ui_port: p,
                } => {
                    payload.insert("kind".to_string(), serde_json::json!("stroke"));
                    payload.insert("points_str".to_string(), serde_json::json!(points));
                    if let Some(a) = author {
                        payload.insert("author".to_string(), serde_json::json!(a));
                    }
                    if let Some(c) = color {
                        payload.insert("color".to_string(), serde_json::json!(c));
                    }
                    if let Some(t) = thickness {
                        payload.insert("thickness".to_string(), serde_json::json!(t));
                    }
                    if let Some(a) = alpha {
                        payload.insert("alpha".to_string(), serde_json::json!(a));
                    }
                    if let Some(ms) = ttl {
                        payload.insert("ttl_ms".to_string(), serde_json::json!(ms));
                    }
                    (j || json, if p != 45716 { p } else { ui_port })
                }
                ScreenAnnotateShapeSubCmd::Rect {
                    x,
                    y,
                    w,
                    h,
                    author,
                    color,
                    thickness,
                    alpha,
                    ttl,
                    json: j,
                    ui_port: p,
                } => {
                    payload.insert("kind".to_string(), serde_json::json!("rect"));
                    payload.insert("x".to_string(), serde_json::json!(x));
                    payload.insert("y".to_string(), serde_json::json!(y));
                    payload.insert("w".to_string(), serde_json::json!(w));
                    payload.insert("h".to_string(), serde_json::json!(h));
                    if let Some(a) = author {
                        payload.insert("author".to_string(), serde_json::json!(a));
                    }
                    if let Some(c) = color {
                        payload.insert("color".to_string(), serde_json::json!(c));
                    }
                    if let Some(t) = thickness {
                        payload.insert("thickness".to_string(), serde_json::json!(t));
                    }
                    if let Some(a) = alpha {
                        payload.insert("alpha".to_string(), serde_json::json!(a));
                    }
                    if let Some(ms) = ttl {
                        payload.insert("ttl_ms".to_string(), serde_json::json!(ms));
                    }
                    (j || json, if p != 45716 { p } else { ui_port })
                }
                ScreenAnnotateShapeSubCmd::Arrow {
                    from,
                    to,
                    author,
                    color,
                    thickness,
                    alpha,
                    ttl,
                    json: j,
                    ui_port: p,
                } => {
                    payload.insert("kind".to_string(), serde_json::json!("arrow"));
                    payload.insert("from_str".to_string(), serde_json::json!(from));
                    payload.insert("to_str".to_string(), serde_json::json!(to));
                    if let Some(a) = author {
                        payload.insert("author".to_string(), serde_json::json!(a));
                    }
                    if let Some(c) = color {
                        payload.insert("color".to_string(), serde_json::json!(c));
                    }
                    if let Some(t) = thickness {
                        payload.insert("thickness".to_string(), serde_json::json!(t));
                    }
                    if let Some(a) = alpha {
                        payload.insert("alpha".to_string(), serde_json::json!(a));
                    }
                    if let Some(ms) = ttl {
                        payload.insert("ttl_ms".to_string(), serde_json::json!(ms));
                    }
                    (j || json, if p != 45716 { p } else { ui_port })
                }
                ScreenAnnotateShapeSubCmd::Text {
                    text,
                    x,
                    y,
                    author,
                    scale,
                    color,
                    alpha,
                    ttl,
                    json: j,
                    ui_port: p,
                } => {
                    payload.insert("kind".to_string(), serde_json::json!("text"));
                    payload.insert("text".to_string(), serde_json::json!(text));
                    payload.insert("x".to_string(), serde_json::json!(x));
                    payload.insert("y".to_string(), serde_json::json!(y));
                    if let Some(a) = author {
                        payload.insert("author".to_string(), serde_json::json!(a));
                    }
                    if let Some(s) = scale {
                        payload.insert("scale".to_string(), serde_json::json!(s));
                    }
                    if let Some(c) = color {
                        payload.insert("color".to_string(), serde_json::json!(c));
                    }
                    if let Some(a) = alpha {
                        payload.insert("alpha".to_string(), serde_json::json!(a));
                    }
                    if let Some(ms) = ttl {
                        payload.insert("ttl_ms".to_string(), serde_json::json!(ms));
                    }
                    (j || json, if p != 45716 { p } else { ui_port })
                }
            };
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(port, "/api/screen/annotation", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen annotation add failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let id = rep.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                println!("Collaborative annotation added (#{id}).");
                if let Some(r) = rep.get("report") {
                    print_screen_annotation_report(r);
                }
            }
        }
        None => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/screen/annotation")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen annotation query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_annotation_report(&rep);
            }
        }
    }
    Ok(())
}

/// `tetherly screen whiteboard` -- Whiteboard arm of the screen dispatcher.
pub(crate) async fn screen_whiteboard(
    action: Option<ScreenWhiteboardSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        Some(ScreenWhiteboardSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = tetherly_node::loopback_get(port, "/api/screen/annotation")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "whiteboard status query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_whiteboard_report(&rep);
            }
        }
        Some(ScreenWhiteboardSubCmd::Undo {
            author,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let uri = if let Some(ref a) = author {
                format!("/api/screen/whiteboard/undo?author={}", urlencode_path(a))
            } else {
                "/api/screen/whiteboard/undo".to_string()
            };
            let (code, resp) = tetherly_node::loopback_post(port, &uri, b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "whiteboard undo failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                if rep.get("undone").map(|v| v.is_null()).unwrap_or(true) {
                    println!("No active whiteboard annotations available to undo.");
                } else {
                    let undone = &rep["undone"];
                    let id = undone.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                    let auth = undone
                        .get("author")
                        .and_then(|v| v.as_str())
                        .unwrap_or("anon");
                    println!("Whiteboard annotation #{id} (author: {auth}) undone.");
                }
                if let Some(r) = rep.get("report") {
                    print_screen_whiteboard_report(r);
                }
            }
        }
        Some(ScreenWhiteboardSubCmd::Redo {
            author,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let uri = if let Some(ref a) = author {
                format!("/api/screen/whiteboard/redo?author={}", urlencode_path(a))
            } else {
                "/api/screen/whiteboard/redo".to_string()
            };
            let (code, resp) = tetherly_node::loopback_post(port, &uri, b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "whiteboard redo failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                if rep.get("redone").map(|v| v.is_null()).unwrap_or(true) {
                    println!("No undone whiteboard annotations available to redo.");
                } else {
                    let redone = &rep["redone"];
                    let id = redone.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                    let auth = redone
                        .get("author")
                        .and_then(|v| v.as_str())
                        .unwrap_or("anon");
                    println!("Whiteboard annotation #{id} (author: {auth}) redone.");
                }
                if let Some(r) = rep.get("report") {
                    print_screen_whiteboard_report(r);
                }
            }
        }
        Some(ScreenWhiteboardSubCmd::Export {
            out,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let uri = if let Some(ref o) = out {
                format!("/api/screen/whiteboard/export?out={}", urlencode_path(o))
            } else {
                "/api/screen/whiteboard/export".to_string()
            };
            let (code, resp) = tetherly_node::loopback_get(port, &uri).await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "whiteboard export failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let items = rep["export"]["active_items"]
                    .as_array()
                    .map(|a| a.len())
                    .unwrap_or(0);
                if let Some(ref dest) = out {
                    println!(
                                    "Whiteboard vector snapshot successfully exported ({items} items) to: {dest}"
                                );
                } else {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&rep["export"]).unwrap_or_default()
                    );
                }
            }
        }
        Some(ScreenWhiteboardSubCmd::Import {
            path,
            replace,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let p_obj = std::path::Path::new(&path);
            if p_obj
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                anyhow::bail!("path traversal rejected");
            }
            let raw_content = std::fs::read(&path)
                .map_err(|e| anyhow::anyhow!("failed to read snapshot file '{path}': {e}"))?;
            let snapshot: serde_json::Value = serde_json::from_slice(&raw_content)
                .map_err(|e| anyhow::anyhow!("invalid JSON snapshot '{path}': {e}"))?;
            let payload = serde_json::json!({
                "snapshot": snapshot,
                "replace": replace
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(port, "/api/screen/whiteboard/import", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "whiteboard import failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                let count = rep
                    .get("imported_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                println!(
                    "Imported {count} whiteboard annotations from '{path}' (replace: {replace})."
                );
                if let Some(r) = rep.get("report") {
                    print_screen_whiteboard_report(r);
                }
            }
        }
        Some(ScreenWhiteboardSubCmd::Policy {
            read_only,
            allow_clear,
            max_items,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            if read_only.is_none() && allow_clear.is_none() && max_items.is_none() {
                let (code, resp) =
                    tetherly_node::loopback_get(port, "/api/screen/whiteboard/policy")
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "whiteboard policy query failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if is_json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    println!("Screen Whiteboard Policy (ADR-106):");
                    if let Some(pol) = rep.get("policy") {
                        println!(
                            "  Read Only:          {}",
                            pol.get("read_only")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                        );
                        println!(
                            "  Allow Peer Clear:   {}",
                            pol.get("allow_peer_clear")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true)
                        );
                        println!(
                            "  Max Items:          {}",
                            pol.get("max_items")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(1000)
                        );
                    }
                }
            } else {
                let mut payload = serde_json::Map::new();
                if let Some(ro) = read_only {
                    payload.insert("read_only".to_string(), serde_json::json!(ro));
                }
                if let Some(ac) = allow_clear {
                    payload.insert("allow_peer_clear".to_string(), serde_json::json!(ac));
                }
                if let Some(mi) = max_items {
                    payload.insert("max_items".to_string(), serde_json::json!(mi));
                }
                let bytes = serde_json::to_vec(&payload)?;
                let (code, resp) =
                    tetherly_node::loopback_post(port, "/api/screen/whiteboard/policy", &bytes)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                            )
                        })?;
                if code != 200 {
                    anyhow::bail!(
                        "whiteboard policy update failed (HTTP {code}): {}",
                        String::from_utf8_lossy(&resp)
                    );
                }
                if is_json {
                    println!("{}", String::from_utf8_lossy(&resp).trim());
                } else {
                    let rep: serde_json::Value = serde_json::from_slice(&resp)
                        .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                    println!("Whiteboard policy updated successfully.");
                    if let Some(pol) = rep.get("policy") {
                        println!(
                            "  Read Only:          {}",
                            pol.get("read_only")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                        );
                        println!(
                            "  Allow Peer Clear:   {}",
                            pol.get("allow_peer_clear")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true)
                        );
                        println!(
                            "  Max Items:          {}",
                            pol.get("max_items")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(1000)
                        );
                    }
                }
            }
        }
        Some(ScreenWhiteboardSubCmd::Clear {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) =
                tetherly_node::loopback_post(port, "/api/screen/whiteboard/clear", b"{}")
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "whiteboard clear failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Whiteboard canvas cleared.");
                print_screen_whiteboard_report(&rep);
            }
        }
        None => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/screen/annotation")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "whiteboard query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_whiteboard_report(&rep);
            }
        }
    }
    Ok(())
}

/// `tetherly screen roi` -- Roi arm of the screen dispatcher.
pub(crate) async fn screen_roi(
    action: Option<ScreenRoiSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        Some(ScreenRoiSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = tetherly_node::loopback_get(port, "/api/screen/roi")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen roi status failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_roi_report(&rep);
            }
        }
        Some(ScreenRoiSubCmd::Set {
            x,
            y,
            w,
            h,
            zoom,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let payload = serde_json::json!({
                "x": x,
                "y": y,
                "w": w,
                "h": h,
                "zoom_percent": zoom,
            });
            let (code, resp) = tetherly_node::loopback_post(
                port,
                "/api/screen/roi",
                &serde_json::to_vec(&payload)?,
            )
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "screen roi set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen ROI viewport updated successfully.");
                print_screen_roi_report(&rep);
            }
        }
        Some(ScreenRoiSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = tetherly_node::loopback_post(port, "/api/screen/roi/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen roi reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen ROI viewport reset to full frame.");
                print_screen_roi_report(&rep);
            }
        }
        Some(ScreenRoiSubCmd::Zoom {
            percent,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let payload = serde_json::json!({
                "zoom_percent": percent,
            });
            let (code, resp) = tetherly_node::loopback_post(
                port,
                "/api/screen/roi/zoom",
                &serde_json::to_vec(&payload)?,
            )
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "screen roi zoom failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen ROI zoom scale updated successfully.");
                print_screen_roi_report(&rep);
            }
        }
        None => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/screen/roi")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen roi query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_roi_report(&rep);
            }
        }
    }
    Ok(())
}

/// `tetherly screen color` -- Color arm of the screen dispatcher.
pub(crate) async fn screen_color(
    action: Option<ScreenColorSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        Some(ScreenColorSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = tetherly_node::loopback_get(port, "/api/screen/color")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen color query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_color_report(&rep);
            }
        }
        Some(ScreenColorSubCmd::Set {
            mode,
            threshold,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let payload = serde_json::json!({
                "mode": mode,
                "threshold": threshold,
            });
            let (code, resp) = tetherly_node::loopback_post(
                port,
                "/api/screen/color",
                &serde_json::to_vec(&payload)?,
            )
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "screen color set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen color mode set successfully.");
                print_screen_color_report(&rep);
            }
        }
        Some(ScreenColorSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = tetherly_node::loopback_post(port, "/api/screen/color/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen color reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen color mode reset to full color (32-bit true color).");
                print_screen_color_report(&rep);
            }
        }
        None => {
            let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/screen/color")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen color query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_color_report(&rep);
            }
        }
    }
    Ok(())
}

pub(crate) fn print_screen_watermark_report(rep: &serde_json::Value) {
    println!("Screen Dynamic Security Watermark (Zero-Dependency Font Pipeline, ADR-104):");
    let ok = rep.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let host_enforced = rep
        .get("host_enforced")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let text_template = rep
        .get("text_template")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let rendered_text = rep
        .get("rendered_text")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let opacity = rep.get("opacity").and_then(|v| v.as_u64()).unwrap_or(64);
    let opacity_pct = rep
        .get("opacity_pct")
        .and_then(|v| v.as_u64())
        .unwrap_or(25);
    let color_hex = rep
        .get("color_hex")
        .and_then(|v| v.as_str())
        .unwrap_or("#DCDCDC");
    let scale = rep.get("scale").and_then(|v| v.as_u64()).unwrap_or(1);
    let pattern = rep
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("grid");
    let spacing_x = rep.get("spacing_x").and_then(|v| v.as_u64()).unwrap_or(240);
    let spacing_y = rep.get("spacing_y").and_then(|v| v.as_u64()).unwrap_or(160);
    let frames_watermarked = rep
        .get("frames_watermarked")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    println!(
        "  Status:               {}",
        if ok { "ok" } else { "failed" }
    );
    println!(
        "  Enabled:              {}",
        if enabled { "yes" } else { "no" }
    );
    println!(
        "  Host Enforced:        {}",
        if host_enforced {
            "yes (visitor lockdown)"
        } else {
            "no"
        }
    );
    println!("  Text Template:        {text_template}");
    if !rendered_text.is_empty() {
        println!("  Rendered Sample:      {rendered_text}");
    }
    println!("  Opacity:              {opacity} ({opacity_pct}%)");
    println!("  Color:                {color_hex}");
    println!(
        "  Font Scale:           {scale}x (glyph: {}x{})",
        8 * scale,
        8 * scale
    );
    println!("  Layout Pattern:       {pattern}");
    println!("  Tile Spacing:         {spacing_x}x{spacing_y} px");
    println!("  Frames Watermarked:   {frames_watermarked}");
}

pub(crate) fn print_screen_annotation_report(rep: &serde_json::Value) {
    println!("Screen Collaborative Annotations & Whiteboard Overlay (ADR-105):");
    let ok = rep.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let count = rep
        .get("annotations_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let laser_active = rep
        .get("laser_active")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let total_added = rep
        .get("total_annotations_added")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let laser_updates = rep
        .get("laser_updates_total")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    println!(
        "  Status:               {}",
        if ok { "ok" } else { "failed" }
    );
    println!("  Active Annotations:   {count}");
    println!("  Total Added:          {total_added}");
    println!(
        "  Laser Pointer:        {}",
        if laser_active { "ACTIVE" } else { "idle/off" }
    );
    println!("  Laser Updates:        {laser_updates}");
    if let Some(laser) = rep.get("laser").filter(|v| !v.is_null()) {
        if let (Some(x), Some(y)) = (
            laser.get("x").and_then(|v| v.as_u64()),
            laser.get("y").and_then(|v| v.as_u64()),
        ) {
            let r = laser.get("radius").and_then(|v| v.as_u64()).unwrap_or(12);
            println!("  Laser Position:       ({x}, {y}) [radius: {r}px]");
        }
    }
    if let Some(arr) = rep.get("annotations").and_then(|v| v.as_array()) {
        if !arr.is_empty() {
            println!("  Active Items:");
            for (i, item) in arr.iter().enumerate() {
                let id = item.get("id").and_then(|v| v.as_u64()).unwrap_or(i as u64);
                let kind_desc = if let Some(k) = item.get("kind") {
                    if let Some(s) = k.get("stroke") {
                        let pts = s
                            .get("points")
                            .and_then(|p| p.as_array())
                            .map(|p| p.len())
                            .unwrap_or(0);
                        format!("stroke ({pts} points)")
                    } else if let Some(r) = k.get("rect") {
                        format!(
                            "rect (at {},{}, {}x{})",
                            r.get("x").unwrap_or(&serde_json::json!(0)),
                            r.get("y").unwrap_or(&serde_json::json!(0)),
                            r.get("w").unwrap_or(&serde_json::json!(0)),
                            r.get("h").unwrap_or(&serde_json::json!(0))
                        )
                    } else if let Some(a) = k.get("arrow") {
                        format!(
                            "arrow ({:?} -> {:?})",
                            a.get("from").unwrap_or(&serde_json::json!([0, 0])),
                            a.get("to").unwrap_or(&serde_json::json!([0, 0]))
                        )
                    } else if let Some(t) = k.get("text") {
                        format!(
                            "text: \"{}\"",
                            t.get("text").and_then(|x| x.as_str()).unwrap_or("")
                        )
                    } else {
                        "custom".to_string()
                    }
                } else {
                    "unknown".to_string()
                };
                let ttl = item.get("ttl_ms").and_then(|v| v.as_u64()).unwrap_or(0);
                let ttl_str = if ttl == 0 {
                    "persistent".to_string()
                } else {
                    format!("{ttl}ms TTL")
                };
                println!("    #{id}: {kind_desc} [{ttl_str}]");
            }
        }
    }
}

pub(crate) fn print_screen_whiteboard_report(rep: &serde_json::Value) {
    println!("Screen Collaborative Whiteboard Status (v2.20 / ADR-106):");
    let count = rep
        .get("annotations_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let redo_count = rep.get("redo_count").and_then(|v| v.as_u64()).unwrap_or(0);
    let total_added = rep
        .get("total_annotations_added")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    println!("  Active Items:         {count}");
    println!("  Redo Stack:           {redo_count}");
    println!("  Total Added:          {total_added}");
    if let Some(pol) = rep.get("whiteboard_policy") {
        let ro = pol
            .get("read_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let pc = pol
            .get("allow_peer_clear")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let max_items = pol
            .get("max_items")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000);
        println!("  Whiteboard Policy:");
        println!("    Read Only:          {ro}");
        println!("    Allow Peer Clear:   {pc}");
        println!("    Max Items:          {max_items}");
    }
    if let Some(arr) = rep.get("annotations").and_then(|v| v.as_array()) {
        if !arr.is_empty() {
            println!("  Active Annotations:");
            for (i, item) in arr.iter().enumerate() {
                let id = item.get("id").and_then(|v| v.as_u64()).unwrap_or(i as u64);
                let author = item
                    .get("author")
                    .and_then(|v| v.as_str())
                    .unwrap_or("anon");
                let kind_desc = if let Some(k) = item.get("kind") {
                    if let Some(s) = k.get("stroke") {
                        let pts = s
                            .get("points")
                            .and_then(|p| p.as_array())
                            .map(|p| p.len())
                            .unwrap_or(0);
                        format!("stroke ({pts} points)")
                    } else if let Some(r) = k.get("rect") {
                        format!(
                            "rect (at {},{}, {}x{})",
                            r.get("x").unwrap_or(&serde_json::json!(0)),
                            r.get("y").unwrap_or(&serde_json::json!(0)),
                            r.get("w").unwrap_or(&serde_json::json!(0)),
                            r.get("h").unwrap_or(&serde_json::json!(0))
                        )
                    } else if let Some(a) = k.get("arrow") {
                        format!(
                            "arrow ({:?} -> {:?})",
                            a.get("from").unwrap_or(&serde_json::json!([0, 0])),
                            a.get("to").unwrap_or(&serde_json::json!([0, 0]))
                        )
                    } else if let Some(t) = k.get("text") {
                        format!(
                            "text: \"{}\"",
                            t.get("text").and_then(|x| x.as_str()).unwrap_or("")
                        )
                    } else {
                        "custom".to_string()
                    }
                } else {
                    "item".to_string()
                };
                println!("    #{id} [{author}]: {kind_desc}");
            }
        }
    }
}

pub(crate) fn print_screen_roi_report(rep: &serde_json::Value) {
    println!("Screen Dynamic Region-of-Interest (ROI) Status (v2.21 / ADR-107):");
    let active = rep.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
    let state_str = if active {
        "active (sub-viewport cropped)"
    } else {
        "inactive (full-frame streaming)"
    };
    println!("  Status:        {state_str}");
    if active {
        let x = rep.get("x").and_then(|v| v.as_u64()).unwrap_or(0);
        let y = rep.get("y").and_then(|v| v.as_u64()).unwrap_or(0);
        let w = rep.get("w").and_then(|v| v.as_u64()).unwrap_or(0);
        let h = rep.get("h").and_then(|v| v.as_u64()).unwrap_or(0);
        println!("  Viewport:      x: {x}, y: {y}, {w}x{h}");
    }
    let zoom = rep
        .get("zoom_percent")
        .and_then(|v| v.as_u64())
        .unwrap_or(100);
    println!("  Zoom Scale:    {zoom}% ({:.1}x)", zoom as f64 / 100.0);
    let eff_w = rep.get("effective_w").and_then(|v| v.as_u64()).unwrap_or(0);
    let eff_h = rep.get("effective_h").and_then(|v| v.as_u64()).unwrap_or(0);
    let src_w = rep.get("source_w").and_then(|v| v.as_u64()).unwrap_or(0);
    let src_h = rep.get("source_h").and_then(|v| v.as_u64()).unwrap_or(0);
    println!("  Resolution:    effective: {eff_w}x{eff_h} · source: {src_w}x{src_h}");
    let updates = rep
        .get("update_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    println!("  Total Updates: {updates}");
}

pub(crate) fn print_screen_color_report(rep: &serde_json::Value) {
    println!("Screen Adaptive Color Depth Pipeline (v2.22 / ADR-108):");
    let mode = rep
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("full_color32");
    let active = rep
        .get("is_active")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mode_desc = match mode {
        "full_color32" => "full_color32 (32-bit uncompressed true color BGRA8/RGBA8)",
        "reduced_color16" => "reduced_color16 (16-bit RGB565 high-bit replenishment)",
        "grayscale8" => "grayscale8 (8-bit Rec.601 luminance economy mode)",
        "monochrome1" => "monochrome1 (1-bit high-contrast black & white binarization)",
        other => other,
    };
    println!("  Mode:          {mode_desc}");
    let active_str = if active {
        "active (color transformation applied)"
    } else {
        "inactive (native full color)"
    };
    println!("  Status:        {active_str}");
    let threshold = rep.get("threshold").and_then(|v| v.as_u64()).unwrap_or(128);
    println!("  Threshold:     {threshold} (active in monochrome1 mode)");
    let updates = rep
        .get("update_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    println!("  Total Updates: {updates}");
}
