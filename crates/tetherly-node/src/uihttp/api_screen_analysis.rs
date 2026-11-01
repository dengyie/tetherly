// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen analysis: metrics, QoE, probes, trendline, cursor, motion, ROI, focus.

use super::*;

pub(super) async fn get_screen_cursor(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_cursor_report();
    json_ok(&rep)
}

// Dirty-rect detection and static frame adaptive squelch (v2.03 / ADR-089).
pub(super) async fn post_screen_cursor(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct CursorBody {
        mode: Option<String>,
        cursor_mode: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<CursorBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.cursor_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("cursor_mode").cloned());

    if let Some(m_str) = mode_str {
        match tetherly_core::ScreenCursorMode::parse(&m_str) {
            Some(mode) => {
                let rep = node.screen_set_cursor_mode(mode);
                json_ok(&rep)
            }
            None => err(
                400,
                "invalid cursor mode: expected decoupled, embedded, or hidden",
            ),
        }
    } else {
        let rep = node.screen_cursor_report();
        json_ok(&rep)
    }
}

pub(super) async fn get_screen_trendline(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_trendline_report();
    json_ok(&rep)
}

// Screen selective NACK & Hybrid ARQ (HARQ) pipeline (v2.07 / ADR-093).
pub(super) async fn post_screen_trendline(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct TrendlineBody {
        mode: Option<String>,
        trendline_mode: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TrendlineBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.trendline_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("trendline_mode").cloned());

    if let Some(m_str) = mode_str {
        match m_str.parse::<tetherly_core::ScreenTrendlineMode>() {
            Ok(mode) => {
                let rep = node.screen_set_trendline_mode(mode);
                json_ok(&rep)
            }
            Err(_) => err(
                400,
                "invalid trendline mode: expected auto, aggressive, conservative, or disabled",
            ),
        }
    } else {
        let rep = node.screen_trendline_report();
        json_ok(&rep)
    }
}

pub(super) async fn get_screen_probe(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_probe_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_probe(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct ProbeBody {
        mode: Option<String>,
        probe_mode: Option<String>,
        target_bps: Option<u32>,
        trigger: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ProbeBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.probe_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("probe_mode").cloned());

    let trigger_flag = parsed_body
        .as_ref()
        .and_then(|b| b.trigger)
        .or_else(|| qmap.get("trigger").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(false);

    let target_bps = parsed_body
        .as_ref()
        .and_then(|b| b.target_bps)
        .or_else(|| qmap.get("target_bps").and_then(|v| v.parse::<u32>().ok()));

    if let Some(m_str) = mode_str {
        match m_str.parse::<tetherly_core::ScreenProbeMode>() {
            Ok(mode) => {
                let _ = node.screen_set_probe_mode(mode);
            }
            Err(_) => {
                return err(
                    400,
                    "invalid probe mode: expected auto, aggressive, conservative, or disabled",
                );
            }
        }
    }

    if trigger_flag || target_bps.is_some() {
        match node.trigger_screen_probe(target_bps).await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(500, &format!("failed to trigger screen probe: {e}")),
        }
    } else {
        let rep = node.screen_probe_report();
        json_ok(&rep)
    }
}

// Screen audio-video playout lip-sync & clock synchronization pipeline (v2.10 / ADR-096).
pub(super) async fn post_screen_probe_trigger(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct TriggerBody {
        target_bps: Option<u32>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TriggerBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let target_bps = parsed_body
        .as_ref()
        .and_then(|b| b.target_bps)
        .or_else(|| qmap.get("target_bps").and_then(|v| v.parse::<u32>().ok()));
    match node.trigger_screen_probe(target_bps).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to trigger screen probe: {e}")),
    }
}

pub(super) async fn get_screen_privacy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_privacy_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_privacy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct PrivacyBody {
        enabled: Option<bool>,
        lock_input: Option<bool>,
        lock: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PrivacyBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled_opt = parsed_body.as_ref().and_then(|b| b.enabled).or_else(|| {
        qmap.get("enabled")
            .and_then(|v| match v.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            })
    });

    let Some(enabled) = enabled_opt else {
        return err(
            400,
            "missing required field: enabled (expected boolean true/false)",
        );
    };

    let lock_input = parsed_body
        .as_ref()
        .and_then(|b| b.lock_input.or(b.lock))
        .or_else(|| {
            qmap.get("lock_input")
                .or_else(|| qmap.get("lock"))
                .and_then(|v| match v.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" | "on" => Some(true),
                    "false" | "0" | "no" | "off" => Some(false),
                    _ => None,
                })
        })
        .unwrap_or(false);

    match node.screen_set_privacy_mode(enabled, lock_input).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(
            500,
            &format!("failed to configure screen privacy mode: {e}"),
        ),
    }
}

pub(super) async fn post_screen_privacy_enable(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct EnableBody {
        lock_input: Option<bool>,
        lock: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<EnableBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let lock_input = parsed_body
        .as_ref()
        .and_then(|b| b.lock_input.or(b.lock))
        .or_else(|| {
            qmap.get("lock_input")
                .or_else(|| qmap.get("lock"))
                .and_then(|v| match v.to_ascii_lowercase().as_str() {
                    "true" | "1" | "yes" | "on" => Some(true),
                    "false" | "0" | "no" | "off" => Some(false),
                    _ => None,
                })
        })
        .unwrap_or(false);

    match node.screen_set_privacy_mode(true, lock_input).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to enable screen privacy mode: {e}")),
    }
}

// Microsecond end-to-end latency profiler & performance telemetry (ADR-099).
pub(super) async fn post_screen_privacy_disable(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_set_privacy_mode(false, false).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to disable screen privacy mode: {e}")),
    }
}

pub(super) async fn get_screen_roi(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_roi_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_roi(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct RoiBody {
        x: Option<u16>,
        y: Option<u16>,
        w: Option<u16>,
        h: Option<u16>,
        zoom_percent: Option<u16>,
        zoom: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<RoiBody> = if !body.is_empty() {
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
    let w = parsed_body
        .as_ref()
        .and_then(|b| b.w)
        .or_else(|| qmap.get("w").and_then(|v| v.parse::<u16>().ok()))
        .unwrap_or(0);
    let h = parsed_body
        .as_ref()
        .and_then(|b| b.h)
        .or_else(|| qmap.get("h").and_then(|v| v.parse::<u16>().ok()))
        .unwrap_or(0);
    let zoom = parsed_body
        .as_ref()
        .and_then(|b| b.zoom_percent.or(b.zoom))
        .or_else(|| {
            qmap.get("zoom_percent")
                .or_else(|| qmap.get("zoom"))
                .and_then(|v| v.parse::<u16>().ok())
        });

    match node.screen_set_roi(x, y, w, h, zoom).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen roi: {e}")),
    }
}

pub(super) async fn post_screen_roi_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_roi().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen roi: {e}")),
    }
}

// Screen adaptive color depth, grayscale economy & monochrome pipeline (v2.22 / ADR-108)
pub(super) async fn post_screen_roi_zoom(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct ZoomBody {
        zoom_percent: Option<u16>,
        zoom: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ZoomBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let zoom = parsed_body
        .as_ref()
        .and_then(|b| b.zoom_percent.or(b.zoom))
        .or_else(|| {
            qmap.get("zoom_percent")
                .or_else(|| qmap.get("zoom"))
                .and_then(|v| v.parse::<u16>().ok())
        })
        .unwrap_or(100);

    match node.screen_set_roi_zoom(zoom).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen roi zoom: {e}")),
    }
}

pub(super) async fn get_screen_metric(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_quality_metric_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_metric(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct MetricBody {
        enabled: Option<bool>,
        sample_interval: Option<u8>,
        interval: Option<u8>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<MetricBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let interval = parsed_body
        .as_ref()
        .and_then(|b| b.sample_interval.or(b.interval))
        .or_else(|| {
            qmap.get("sample_interval")
                .or_else(|| qmap.get("interval"))
                .and_then(|v| v.parse::<u8>().ok())
        });

    match node.screen_set_quality_metric_mode(enabled, interval).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(
            500,
            &format!("failed to set screen quality metric mode: {e}"),
        ),
    }
}

// Screen streaming autonomous closed-loop QoE adaptation pipeline (v2.25 / ADR-111)
pub(super) async fn post_screen_metric_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_quality_metric_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(
            500,
            &format!("failed to reset screen quality metric mode: {e}"),
        ),
    }
}

pub(super) async fn get_screen_qoe(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_qoe_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_qoe(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct QoeBody {
        mode: Option<String>,
        target_psnr_db: Option<u8>,
        target_psnr: Option<u8>,
        target_ssim_pct: Option<u8>,
        target_ssim: Option<u8>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<QoeBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone())
        .or_else(|| qmap.get("mode").cloned())
        .unwrap_or_else(|| "balanced".to_string());
    let mode = match mode_str.parse::<tetherly_core::ScreenQoeMode>() {
        Ok(m) => m,
        Err(_) => {
            return err(
                400,
                "invalid qoe mode (expected: off, conservative, balanced, economy)",
            )
        }
    };
    let target_psnr_db = parsed_body
        .as_ref()
        .and_then(|b| b.target_psnr_db.or(b.target_psnr))
        .or_else(|| {
            qmap.get("target_psnr_db")
                .or_else(|| qmap.get("target_psnr"))
                .and_then(|v| v.parse::<u8>().ok())
        });
    let target_ssim_pct = parsed_body
        .as_ref()
        .and_then(|b| b.target_ssim_pct.or(b.target_ssim))
        .or_else(|| {
            qmap.get("target_ssim_pct")
                .or_else(|| qmap.get("target_ssim"))
                .and_then(|v| v.parse::<u8>().ok())
        });

    match node
        .screen_set_qoe_mode(mode, target_psnr_db, target_ssim_pct)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen qoe mode: {e}")),
    }
}

// Screen streaming Progressive Intra-Refresh (PIR) anti-burst slice pipeline (v2.26 / ADR-112)
pub(super) async fn post_screen_qoe_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_qoe_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen qoe mode: {e}")),
    }
}

pub(super) async fn get_screen_pir(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_pir_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_pir(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct PirBody {
        mode: Option<String>,
        bands: Option<u8>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PirBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone())
        .or_else(|| qmap.get("mode").cloned())
        .unwrap_or_else(|| "continuous".to_string());
    let mode = match mode_str.parse::<tetherly_core::ScreenPirMode>() {
        Ok(m) => m,
        Err(_) => {
            return err(
                400,
                "invalid pir mode (expected: disabled, continuous, on-demand)",
            )
        }
    };
    let bands = parsed_body
        .as_ref()
        .and_then(|b| b.bands)
        .or_else(|| qmap.get("bands").and_then(|v| v.parse::<u8>().ok()));

    match node.screen_set_pir_mode(mode, bands).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen pir mode: {e}")),
    }
}

// Screen streaming Long-Term Reference (LTR) frame anchoring & recovery pipeline (v2.27 / ADR-113)
pub(super) async fn post_screen_pir_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_pir_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen pir mode: {e}")),
    }
}

pub(super) async fn get_screen_motion(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_motion_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_motion(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct MotionBody {
        enabled: Option<bool>,
        max_displacement: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<MotionBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let max_displacement = parsed_body
        .as_ref()
        .and_then(|b| b.max_displacement)
        .or_else(|| {
            qmap.get("max_displacement")
                .and_then(|v| v.parse::<u16>().ok())
        })
        .unwrap_or(tetherly_core::SCREEN_MOTION_DEFAULT_MAX_DISPLACEMENT);

    match node.screen_set_motion_mode(enabled, max_displacement).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen motion mode: {e}")),
    }
}

// Screen streaming spatial tile-grid decomposition (v2.30 / ADR-116)
pub(super) async fn post_screen_motion_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_motion_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen motion mode: {e}")),
    }
}

pub(super) async fn get_screen_focus(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_focus_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_focus(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct FocusBody {
        enabled: Option<bool>,
        mode: Option<String>,
        radius: Option<u16>,
        peripheral_skip_ratio: Option<u8>,
        box_x: Option<u16>,
        box_y: Option<u16>,
        box_w: Option<u16>,
        box_h: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<FocusBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let enabled = parsed_body
        .as_ref()
        .and_then(|b| b.enabled)
        .or_else(|| qmap.get("enabled").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(true);
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone())
        .or_else(|| qmap.get("mode").cloned());
    let mode = mode_str.and_then(|s| tetherly_core::ScreenFocusMode::from_str_loose(&s));
    let radius = parsed_body
        .as_ref()
        .and_then(|b| b.radius)
        .or_else(|| qmap.get("radius").and_then(|v| v.parse::<u16>().ok()));
    let peripheral_skip_ratio = parsed_body
        .as_ref()
        .and_then(|b| b.peripheral_skip_ratio)
        .or_else(|| {
            qmap.get("peripheral_skip_ratio")
                .and_then(|v| v.parse::<u8>().ok())
        });

    let bx = parsed_body
        .as_ref()
        .and_then(|b| b.box_x)
        .or_else(|| qmap.get("box_x").and_then(|v| v.parse::<u16>().ok()));
    let by = parsed_body
        .as_ref()
        .and_then(|b| b.box_y)
        .or_else(|| qmap.get("box_y").and_then(|v| v.parse::<u16>().ok()));
    let bw = parsed_body
        .as_ref()
        .and_then(|b| b.box_w)
        .or_else(|| qmap.get("box_w").and_then(|v| v.parse::<u16>().ok()));
    let bh = parsed_body
        .as_ref()
        .and_then(|b| b.box_h)
        .or_else(|| qmap.get("box_h").and_then(|v| v.parse::<u16>().ok()));
    let box_coords = match (bx, by, bw, bh) {
        (Some(x), Some(y), Some(w), Some(h)) => Some((x, y, w, h)),
        _ => None,
    };

    match node
        .screen_set_focus_mode(enabled, mode, radius, peripheral_skip_ratio, box_coords)
        .await
    {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen focus mode: {e}")),
    }
}

pub(super) async fn post_screen_focus_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_focus_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen focus mode: {e}")),
    }
}
