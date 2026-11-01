// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen annotations, whiteboard, watermark, laser pointer and privacy overlays.

use super::*;

pub(super) async fn get_screen_watermark(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_watermark_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_watermark(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct WatermarkBody {
        enabled: Option<bool>,
        #[serde(default, alias = "enforce", alias = "enforced")]
        host_enforced: Option<bool>,
        #[serde(default, alias = "text_template", alias = "template")]
        text: Option<String>,
        opacity: Option<u8>,
        color: Option<String>,
        color_rgb: Option<[u8; 3]>,
        scale: Option<u8>,
        pattern: Option<String>,
        spacing_x: Option<u16>,
        spacing_y: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<WatermarkBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };

    let enabled = parsed_body.as_ref().and_then(|b| b.enabled).or_else(|| {
        qmap.get("enabled")
            .and_then(|v| match v.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            })
    });

    let host_enforced = parsed_body
        .as_ref()
        .and_then(|b| b.host_enforced)
        .or_else(|| {
            qmap.get("host_enforced")
                .or_else(|| qmap.get("enforce"))
                .or_else(|| qmap.get("enforced"))
                .and_then(|v| match v.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" | "on" => Some(true),
                    "false" | "0" | "no" | "off" => Some(false),
                    _ => None,
                })
        });

    let text_template = parsed_body
        .as_ref()
        .and_then(|b| b.text.clone())
        .or_else(|| qmap.get("text").cloned())
        .or_else(|| qmap.get("text_template").cloned());

    let opacity = parsed_body
        .as_ref()
        .and_then(|b| b.opacity)
        .or_else(|| qmap.get("opacity").and_then(|v| v.parse::<u8>().ok()));

    let color_rgb = parsed_body.as_ref().and_then(|b| b.color_rgb).or_else(|| {
        let col_str = parsed_body
            .as_ref()
            .and_then(|b| b.color.as_deref())
            .or_else(|| qmap.get("color").map(String::as_str))?;
        let s = col_str.trim().trim_start_matches('#');
        if s.len() == 6 {
            let r = u8::from_str_radix(&s[0..2], 16).ok()?;
            let g = u8::from_str_radix(&s[2..4], 16).ok()?;
            let b = u8::from_str_radix(&s[4..6], 16).ok()?;
            Some([r, g, b])
        } else {
            None
        }
    });

    let scale = parsed_body
        .as_ref()
        .and_then(|b| b.scale)
        .or_else(|| qmap.get("scale").and_then(|v| v.parse::<u8>().ok()));

    let pattern = parsed_body
        .as_ref()
        .and_then(|b| b.pattern.clone())
        .or_else(|| qmap.get("pattern").cloned());

    let spacing_x = parsed_body
        .as_ref()
        .and_then(|b| b.spacing_x)
        .or_else(|| qmap.get("spacing_x").and_then(|v| v.parse::<u16>().ok()));

    let spacing_y = parsed_body
        .as_ref()
        .and_then(|b| b.spacing_y)
        .or_else(|| qmap.get("spacing_y").and_then(|v| v.parse::<u16>().ok()));

    let update = crate::runtime::ScreenWatermarkUpdate {
        enabled,
        host_enforced,
        text_template,
        opacity,
        color_rgb,
        scale,
        pattern,
        spacing_x,
        spacing_y,
    };

    match node.screen_set_watermark(update).await {
        Ok(rep) => json_ok(&rep),
        Err(tetherly_core::CoreError::InvalidConfig(msg)) if msg.contains("enforced by host") => {
            err(403, &format!("watermark configuration forbidden: {msg}"))
        }
        Err(e) => err(400, &format!("invalid watermark configuration: {e}")),
    }
}

pub(super) async fn post_screen_watermark_enable(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct EnableWatermarkBody {
        #[serde(default, alias = "enforce", alias = "enforced")]
        host_enforced: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<EnableWatermarkBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let host_enforced = parsed_body
        .as_ref()
        .and_then(|b| b.host_enforced)
        .or_else(|| {
            qmap.get("host_enforced")
                .or_else(|| qmap.get("enforce"))
                .or_else(|| qmap.get("enforced"))
                .and_then(|v| match v.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" | "on" => Some(true),
                    "false" | "0" | "no" | "off" => Some(false),
                    _ => None,
                })
        })
        .unwrap_or(false);

    match node.screen_enable_watermark(host_enforced).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &format!("failed to enable watermark: {e}")),
    }
}

// Collaborative whiteboard annotations & transient virtual laser pointer (ADR-105)
pub(super) async fn post_screen_watermark_disable(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_disable_watermark().await {
        Ok(rep) => json_ok(&rep),
        Err(tetherly_core::CoreError::InvalidConfig(msg)) if msg.contains("enforced by host") => {
            err(403, &format!("watermark disable forbidden: {msg}"))
        }
        Err(e) => err(400, &format!("failed to disable watermark: {e}")),
    }
}

pub(super) async fn get_screen_annotation(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_annotation_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_annotation(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct AddAnnotationBody {
        #[serde(default)]
        id: Option<u32>,
        author: Option<String>,
        kind: Option<String>,
        points: Option<Vec<(u16, u16)>>,
        points_str: Option<String>,
        x: Option<u16>,
        y: Option<u16>,
        w: Option<u16>,
        h: Option<u16>,
        from: Option<(u16, u16)>,
        to: Option<(u16, u16)>,
        from_str: Option<String>,
        to_str: Option<String>,
        text: Option<String>,
        scale: Option<u8>,
        color: Option<String>,
        color_rgb: Option<[u8; 3]>,
        thickness: Option<u8>,
        alpha: Option<u8>,
        ttl_ms: Option<u32>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<AddAnnotationBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };

    let kind_str = parsed_body
        .as_ref()
        .and_then(|b| b.kind.clone())
        .or_else(|| qmap.get("kind").cloned())
        .unwrap_or_else(|| "stroke".to_string())
        .to_ascii_lowercase();

    let color_rgb = parsed_body
        .as_ref()
        .and_then(|b| b.color_rgb)
        .or_else(|| {
            let col_str = parsed_body
                .as_ref()
                .and_then(|b| b.color.as_deref())
                .or_else(|| qmap.get("color").map(String::as_str))?;
            let s = col_str.trim().trim_start_matches('#');
            if s.len() == 6 {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                Some([r, g, b])
            } else {
                None
            }
        })
        .unwrap_or([255, 0, 0]);

    let thickness = parsed_body
        .as_ref()
        .and_then(|b| b.thickness)
        .or_else(|| qmap.get("thickness").and_then(|v| v.parse::<u8>().ok()))
        .unwrap_or(3);

    let alpha = parsed_body
        .as_ref()
        .and_then(|b| b.alpha)
        .or_else(|| qmap.get("alpha").and_then(|v| v.parse::<u8>().ok()))
        .unwrap_or(255);

    let ttl_ms = parsed_body
        .as_ref()
        .and_then(|b| b.ttl_ms)
        .or_else(|| {
            qmap.get("ttl_ms")
                .or_else(|| qmap.get("ttl"))
                .and_then(|v| v.parse::<u32>().ok())
        })
        .unwrap_or(0);

    let author = parsed_body
        .as_ref()
        .and_then(|b| b.author.clone())
        .or_else(|| qmap.get("author").cloned())
        .unwrap_or_default();

    let id = parsed_body
        .as_ref()
        .and_then(|b| b.id)
        .or_else(|| qmap.get("id").and_then(|v| v.parse::<u32>().ok()))
        .unwrap_or(0);

    let kind = match kind_str.as_str() {
        "stroke" | "polyline" | "line" => {
            let pts = parsed_body
                .as_ref()
                .and_then(|b| b.points.clone())
                .or_else(|| {
                    let s = parsed_body
                        .as_ref()
                        .and_then(|b| b.points_str.as_deref())
                        .or_else(|| qmap.get("points").map(String::as_str))?;
                    Some(
                        s.split(';')
                            .filter_map(|p| {
                                let mut it = p.split(',');
                                let x = it.next()?.trim().parse::<u16>().ok()?;
                                let y = it.next()?.trim().parse::<u16>().ok()?;
                                Some((x, y))
                            })
                            .collect(),
                    )
                })
                .unwrap_or_default();
            if pts.is_empty() {
                return err(400, "stroke annotation requires non-empty points");
            }
            tetherly_core::AnnotationKind::Stroke { points: pts }
        }
        "rect" | "box" => {
            let x = parsed_body
                .as_ref()
                .and_then(|b| b.x)
                .or_else(|| qmap.get("x").and_then(|v| v.parse::<u16>().ok()))
                .unwrap_or(0);
            let y = parsed_body
                .as_ref()
                .and_then(|b| b.y)
                .or_else(|| qmap.get("y").and_then(|v| v.parse::<u16>().ok()))
                .unwrap_or(0);
            let w = parsed_body
                .as_ref()
                .and_then(|b| b.w)
                .or_else(|| {
                    qmap.get("w")
                        .or_else(|| qmap.get("width"))
                        .and_then(|v| v.parse::<u16>().ok())
                })
                .unwrap_or(0);
            let h = parsed_body
                .as_ref()
                .and_then(|b| b.h)
                .or_else(|| {
                    qmap.get("h")
                        .or_else(|| qmap.get("height"))
                        .and_then(|v| v.parse::<u16>().ok())
                })
                .unwrap_or(0);
            if w == 0 || h == 0 {
                return err(400, "rect annotation requires non-zero width and height");
            }
            tetherly_core::AnnotationKind::Rect { x, y, w, h }
        }
        "arrow" => {
            let parse_pt = |s: &str| -> Option<(u16, u16)> {
                let mut it = s.split(',');
                let x = it.next()?.trim().parse::<u16>().ok()?;
                let y = it.next()?.trim().parse::<u16>().ok()?;
                Some((x, y))
            };
            let from = parsed_body
                .as_ref()
                .and_then(|b| b.from)
                .or_else(|| {
                    let s = parsed_body
                        .as_ref()
                        .and_then(|b| b.from_str.as_deref())
                        .or_else(|| qmap.get("from").map(String::as_str))?;
                    parse_pt(s)
                })
                .unwrap_or((0, 0));
            let to = parsed_body
                .as_ref()
                .and_then(|b| b.to)
                .or_else(|| {
                    let s = parsed_body
                        .as_ref()
                        .and_then(|b| b.to_str.as_deref())
                        .or_else(|| qmap.get("to").map(String::as_str))?;
                    parse_pt(s)
                })
                .unwrap_or((0, 0));
            if from == to {
                return err(400, "arrow requires distinct from and to points");
            }
            tetherly_core::AnnotationKind::Arrow { from, to }
        }
        "text" | "note" => {
            let text = parsed_body
                .as_ref()
                .and_then(|b| b.text.clone())
                .or_else(|| qmap.get("text").cloned())
                .unwrap_or_default();
            if text.is_empty() {
                return err(400, "text annotation requires non-empty text");
            }
            let x = parsed_body
                .as_ref()
                .and_then(|b| b.x)
                .or_else(|| qmap.get("x").and_then(|v| v.parse::<u16>().ok()))
                .unwrap_or(0);
            let y = parsed_body
                .as_ref()
                .and_then(|b| b.y)
                .or_else(|| qmap.get("y").and_then(|v| v.parse::<u16>().ok()))
                .unwrap_or(0);
            let scale = parsed_body
                .as_ref()
                .and_then(|b| b.scale)
                .or_else(|| qmap.get("scale").and_then(|v| v.parse::<u8>().ok()))
                .unwrap_or(1);
            tetherly_core::AnnotationKind::Text { x, y, text, scale }
        }
        other => return err(400, &format!("unsupported annotation kind: {other}")),
    };

    let item = tetherly_core::AnnotationItem {
        id,
        author,
        kind,
        color_rgb,
        thickness,
        alpha,
        ttl_ms,
        created_at_ms: 0,
    };

    match node.screen_add_annotation(item).await {
        Ok(item_id) => {
            let rep = node.screen_annotation_report();
            json_ok(&serde_json::json!({
                "ok": true,
                "id": item_id,
                "report": rep,
            }))
        }
        Err(e) if e.to_string().contains("read-only") => err(403, &format!("forbidden: {e}")),
        Err(e) => err(500, &format!("failed to add annotation: {e}")),
    }
}

pub(super) async fn post_screen_annotation_clear(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_clear_annotations().await {
        Ok(()) => {
            let rep = node.screen_annotation_report();
            json_ok(&rep)
        }
        Err(e) => err(500, &format!("failed to clear annotations: {e}")),
    }
}

pub(super) async fn get_screen_laser(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_annotation_report();
    json_ok(&serde_json::json!({
        "ok": true,
        "laser_active": rep.laser_active,
        "laser": rep.laser,
        "laser_updates_total": rep.laser_updates_total,
        "timestamp_ms": rep.timestamp_ms,
    }))
}

pub(super) async fn post_screen_laser(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct LaserBody {
        x: Option<u16>,
        y: Option<u16>,
        active: Option<bool>,
        color: Option<String>,
        color_rgb: Option<[u8; 3]>,
        radius: Option<u8>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<LaserBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };

    let x = parsed_body
        .as_ref()
        .and_then(|b| b.x)
        .or_else(|| qmap.get("x").and_then(|v| v.parse::<u16>().ok()))
        .unwrap_or(0);
    let y = parsed_body
        .as_ref()
        .and_then(|b| b.y)
        .or_else(|| qmap.get("y").and_then(|v| v.parse::<u16>().ok()))
        .unwrap_or(0);

    let active = parsed_body
        .as_ref()
        .and_then(|b| b.active)
        .or_else(|| {
            qmap.get("active")
                .and_then(|v| match v.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" | "on" => Some(true),
                    "false" | "0" | "no" | "off" => Some(false),
                    _ => None,
                })
        })
        .unwrap_or(true);

    let color_rgb = parsed_body
        .as_ref()
        .and_then(|b| b.color_rgb)
        .or_else(|| {
            let col_str = parsed_body
                .as_ref()
                .and_then(|b| b.color.as_deref())
                .or_else(|| qmap.get("color").map(String::as_str))?;
            let s = col_str.trim().trim_start_matches('#');
            if s.len() == 6 {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                Some([r, g, b])
            } else {
                None
            }
        })
        .unwrap_or([255, 30, 30]);

    let radius = parsed_body
        .as_ref()
        .and_then(|b| b.radius)
        .or_else(|| qmap.get("radius").and_then(|v| v.parse::<u8>().ok()))
        .unwrap_or(12);

    let laser_state = tetherly_core::LaserPointerState {
        x,
        y,
        active,
        color_rgb,
        radius,
    };

    match node.screen_set_laser_pointer(laser_state).await {
        Ok(()) => {
            let rep = node.screen_annotation_report();
            json_ok(&serde_json::json!({
                "ok": true,
                "laser_active": rep.laser_active,
                "laser": rep.laser,
                "laser_updates_total": rep.laser_updates_total,
                "timestamp_ms": rep.timestamp_ms,
            }))
        }
        Err(e) => err(500, &format!("failed to set laser pointer: {e}")),
    }
}

pub(super) async fn post_screen_whiteboard_undo(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct UndoBody {
        author: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<UndoBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let author = parsed_body
        .and_then(|b| b.author)
        .or_else(|| qmap.get("author").cloned());
    match node.screen_whiteboard_undo(author.as_deref()).await {
        Ok(undone) => {
            let rep = node.screen_annotation_report();
            json_ok(&serde_json::json!({
                "ok": true,
                "undone": undone,
                "report": rep,
            }))
        }
        Err(e) => err(500, &format!("failed to undo whiteboard annotation: {e}")),
    }
}

pub(super) async fn post_screen_whiteboard_redo(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct RedoBody {
        author: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<RedoBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let author = parsed_body
        .and_then(|b| b.author)
        .or_else(|| qmap.get("author").cloned());
    match node.screen_whiteboard_redo(author.as_deref()).await {
        Ok(redone) => {
            let rep = node.screen_annotation_report();
            json_ok(&serde_json::json!({
                "ok": true,
                "redone": redone,
                "report": rep,
            }))
        }
        Err(e) => err(500, &format!("failed to redo whiteboard annotation: {e}")),
    }
}

pub(super) async fn get_screen_whiteboard_export(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let out_path = qmap.get("out").map(String::as_str);
    match node.screen_whiteboard_export(out_path) {
        Ok(snapshot) => {
            let rep = node.screen_annotation_report();
            json_ok(&serde_json::json!({
                "ok": true,
                "export": snapshot,
                "report": rep,
            }))
        }
        Err(e) => err(400, &format!("failed to export whiteboard: {e}")),
    }
}

pub(super) async fn post_screen_whiteboard_import(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct ImportBody {
        snapshot: Option<tetherly_core::WhiteboardSnapshot>,
        items: Option<Vec<tetherly_core::AnnotationItem>>,
        file: Option<String>,
        replace: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ImportBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let replace = parsed_body
        .as_ref()
        .and_then(|b| b.replace)
        .or_else(|| qmap.get("replace").map(|v| v == "true" || v == "1"))
        .unwrap_or(false);

    let snapshot_res = if let Some(file_path) = parsed_body
        .as_ref()
        .and_then(|b| b.file.as_deref())
        .or_else(|| qmap.get("file").map(String::as_str))
    {
        let path = std::path::Path::new(file_path);
        if path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return err(400, "path traversal rejected");
        }
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice::<tetherly_core::WhiteboardSnapshot>(&bytes)
                .map_err(|e| format!("invalid snapshot file: {e}")),
            Err(e) => return err(400, &format!("failed to read snapshot file: {e}")),
        }
    } else if let Some(snap) = parsed_body.as_ref().and_then(|b| b.snapshot.clone()) {
        Ok(snap)
    } else if let Some(items) = parsed_body.as_ref().and_then(|b| b.items.clone()) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Ok(tetherly_core::WhiteboardSnapshot {
            version: 1,
            created_at_ms: now_ms,
            session_id: "import".into(),
            active_items: items,
            total_added_counter: 0,
        })
    } else {
        Err("missing snapshot, items or file".into())
    };

    match snapshot_res {
        Ok(snap) => match node.screen_whiteboard_import(snap, replace).await {
            Ok(count) => {
                let rep = node.screen_annotation_report();
                json_ok(&serde_json::json!({
                    "ok": true,
                    "imported_count": count,
                    "report": rep,
                }))
            }
            Err(e) => err(400, &format!("failed to import whiteboard: {e}")),
        },
        Err(msg) => err(400, &msg),
    }
}

pub(super) async fn get_screen_whiteboard_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let policy = node.screen_whiteboard_get_policy();
    json_ok(&serde_json::json!({
        "ok": true,
        "policy": policy,
    }))
}

pub(super) async fn post_screen_whiteboard_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct PolicyBody {
        read_only: Option<bool>,
        allow_peer_clear: Option<bool>,
        max_items: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PolicyBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let current = node.screen_whiteboard_get_policy();
    let read_only = parsed_body
        .as_ref()
        .and_then(|b| b.read_only)
        .or_else(|| qmap.get("read_only").map(|v| v == "true" || v == "1"))
        .unwrap_or(current.read_only);
    let allow_peer_clear = parsed_body
        .as_ref()
        .and_then(|b| b.allow_peer_clear)
        .or_else(|| {
            qmap.get("allow_peer_clear")
                .map(|v| v == "true" || v == "1")
        })
        .unwrap_or(current.allow_peer_clear);
    let max_items = parsed_body
        .as_ref()
        .and_then(|b| b.max_items)
        .or_else(|| qmap.get("max_items").and_then(|v| v.parse::<u16>().ok()))
        .unwrap_or(current.max_items);

    let updated = tetherly_core::WhiteboardPolicy {
        read_only,
        allow_peer_clear,
        max_items,
    };
    match node.screen_whiteboard_set_policy(updated).await {
        Ok(()) => json_ok(&serde_json::json!({
            "ok": true,
            "policy": updated,
        })),
        Err(e) => err(500, &format!("failed to update whiteboard policy: {e}")),
    }
}

// Screen dynamic Region-of-Interest (ROI) viewport cropping & zoom (v2.21 / ADR-107)
pub(super) async fn post_screen_whiteboard_clear(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_clear_annotations().await {
        Ok(()) => {
            let rep = node.screen_annotation_report();
            json_ok(&rep)
        }
        Err(e) => err(500, &format!("failed to clear whiteboard: {e}")),
    }
}
