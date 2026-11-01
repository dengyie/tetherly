// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen quality tuning: bitrate, fps, scale, FEC, pacer, HARQ, TWCC, playout.

use super::*;

pub(super) async fn get_screen_bitrate(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_bitrate_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_bitrate(
    _method: &str,
    _query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct BitrateReq {
        #[serde(default)]
        kbps: Option<u32>,
        #[serde(default)]
        bps: Option<u32>,
    }
    let req: BitrateReq = if !body.is_empty() {
        match serde_json::from_slice(body) {
            Ok(b) => b,
            Err(_) => return err(400, "bad json"),
        }
    } else {
        return err(400, "missing bitrate parameter (kbps or bps)");
    };
    let kbps = req.kbps.or_else(|| req.bps.map(|b| b / 1000)).unwrap_or(0);
    if kbps == 0 {
        return err(400, "bitrate must be greater than 0");
    }
    let rep = node.screen_set_bitrate_reported(kbps);
    json_ok(&rep)
}

pub(super) async fn get_screen_fps(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_fps_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_fps(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct FpsReq {
        #[serde(default)]
        fps: Option<u32>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<FpsReq> = if !body.is_empty() {
        match serde_json::from_slice(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected { fps: number }"),
        }
    } else {
        None
    };
    let fps_val = parsed_body
        .and_then(|b| b.fps)
        .or_else(|| qmap.get("fps").and_then(|v| v.parse::<u32>().ok()));
    let fps = match fps_val {
        Some(f) => f,
        None => return err(400, "missing fps parameter"),
    };
    let rep = node.screen_set_fps_reported(fps);
    json_ok(&rep)
}

pub(super) async fn get_screen_scale(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_scale_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_scale(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct ScaleReq {
        #[serde(default)]
        scale_pct: Option<u32>,
        #[serde(default)]
        scale: Option<u32>,
        #[serde(default)]
        pct: Option<u32>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ScaleReq> = if !body.is_empty() {
        match serde_json::from_slice(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected { scale_pct: number }"),
        }
    } else {
        None
    };
    let scale_val = parsed_body
        .and_then(|b| b.scale_pct.or(b.scale).or(b.pct))
        .or_else(|| {
            qmap.get("scale_pct")
                .or_else(|| qmap.get("scale"))
                .or_else(|| qmap.get("pct"))
                .and_then(|v| v.parse::<u32>().ok())
        });
    let scale = match scale_val {
        Some(s) => s,
        None => return err(400, "missing scale parameter (scale_pct or scale or pct)"),
    };
    let rep = node.screen_set_scale_reported(scale);
    json_ok(&rep)
}

pub(super) async fn get_screen_profile(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_profile_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_profile(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct ProfileReq {
        #[serde(default)]
        profile: Option<String>,
        #[serde(default)]
        min_qp: Option<u8>,
        #[serde(default)]
        max_qp: Option<u8>,
        #[serde(default)]
        qp: Option<u8>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ProfileReq> = if !body.is_empty() {
        match serde_json::from_slice(body) {
            Ok(b) => Some(b),
            Err(_) => {
                return err(
                    400,
                    "bad json, expected { profile?: string, min_qp?: number, max_qp?: number }",
                )
            }
        }
    } else {
        None
    };
    let profile_str = parsed_body
        .as_ref()
        .and_then(|b| b.profile.clone())
        .or_else(|| qmap.get("profile").cloned())
        .or_else(|| qmap.get("mode").cloned());
    let min_qp = parsed_body
        .as_ref()
        .and_then(|b| b.min_qp)
        .or_else(|| qmap.get("min_qp").and_then(|v| v.parse::<u8>().ok()));
    let max_qp = parsed_body
        .as_ref()
        .and_then(|b| b.max_qp)
        .or_else(|| qmap.get("max_qp").and_then(|v| v.parse::<u8>().ok()));
    let single_qp = parsed_body
        .as_ref()
        .and_then(|b| b.qp)
        .or_else(|| qmap.get("qp").and_then(|v| v.parse::<u8>().ok()));

    if let Some(prof_str) = profile_str {
        match prof_str.parse::<tetherly_core::ScreenContentProfile>() {
            Ok(p) => {
                node.screen_set_profile(p);
            }
            Err(_) => return err(400, "invalid profile: expected text, motion, or auto"),
        }
    }
    if let Some(qp) = single_qp {
        node.screen_set_qp_range(qp, qp);
    } else if min_qp.is_some() || max_qp.is_some() {
        let current_range = node.screen_qp_range();
        let new_min = min_qp.unwrap_or(current_range.0);
        let new_max = max_qp.unwrap_or(current_range.1);
        node.screen_set_qp_range(new_min, new_max);
    }
    let rep = node.screen_profile_report();
    json_ok(&rep)
}

pub(super) async fn get_screen_matrix(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_matrix_report();
    json_ok(&rep)
}

// Playout buffer & adaptive jitter dejitterizer (v2.01 / ADR-087).
pub(super) async fn post_screen_matrix(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct MatrixReq {
        #[serde(default)]
        mode: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<MatrixReq> = if !body.is_empty() {
        match serde_json::from_slice(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected { mode?: string }"),
        }
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone())
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("matrix_mode").cloned());

    if let Some(m_str) = mode_str {
        match tetherly_core::ScreenMatrixMode::from_str_loose(&m_str) {
            Some(mode) => {
                let rep = node.screen_set_matrix_mode(mode);
                json_ok(&rep)
            }
            None => err(
                400,
                "invalid matrix mode: expected auto, conservative, or manual",
            ),
        }
    } else {
        let rep = node.screen_matrix_report();
        json_ok(&rep)
    }
}

pub(super) async fn get_screen_playout(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_playout_report();
    json_ok(&rep)
}

// Hardware cursor decoupling (v2.02 / ADR-088).
pub(super) async fn post_screen_playout(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct PlayoutBody {
        mode: Option<String>,
        playout_mode: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PlayoutBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.playout_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("playout_mode").cloned());

    if let Some(m_str) = mode_str {
        match tetherly_core::ScreenPlayoutMode::parse(&m_str) {
            Some(mode) => {
                let rep = node.screen_set_playout_mode(mode);
                json_ok(&rep)
            }
            None => err(
                400,
                "invalid playout mode: expected adaptive, zerolatency, or smooth",
            ),
        }
    } else {
        let rep = node.screen_playout_report();
        json_ok(&rep)
    }
}

pub(super) async fn get_screen_squelch(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_squelch_report();
    json_ok(&rep)
}

// Forward error correction and adaptive redundancy protection (v2.04 / ADR-090).
pub(super) async fn post_screen_squelch(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct SquelchBody {
        mode: Option<String>,
        squelch_mode: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<SquelchBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.squelch_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("squelch_mode").cloned());

    if let Some(m_str) = mode_str {
        match tetherly_core::ScreenSquelchMode::from_str_loose(&m_str) {
            Some(mode) => {
                let rep = node.screen_set_squelch_mode(mode);
                json_ok(&rep)
            }
            None => err(
                400,
                "invalid squelch mode: expected auto, aggressive, or disabled",
            ),
        }
    } else {
        let rep = node.screen_squelch_report();
        json_ok(&rep)
    }
}

pub(super) async fn get_screen_fec(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_fec_report();
    json_ok(&rep)
}

// Microsecond packet pacing and burst shaper (v2.05 / ADR-091).
pub(super) async fn post_screen_fec(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct FecBody {
        mode: Option<String>,
        fec_mode: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<FecBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.fec_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("fec_mode").cloned());

    if let Some(m_str) = mode_str {
        match m_str.parse::<tetherly_core::ScreenFecMode>() {
            Ok(mode) => {
                let rep = node.screen_set_fec_mode(mode);
                json_ok(&rep)
            }
            Err(_) => err(
                400,
                "invalid fec mode: expected auto, conservative, aggressive, or disabled",
            ),
        }
    } else {
        let rep = node.screen_fec_report();
        json_ok(&rep)
    }
}

pub(super) async fn get_screen_pacer(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_pacer_report();
    json_ok(&rep)
}

// Delay-based congestion controller & trendline filter (v2.06 / ADR-092).
pub(super) async fn post_screen_pacer(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct PacerBody {
        mode: Option<String>,
        pacer_mode: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<PacerBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.pacer_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("pacer_mode").cloned());

    if let Some(m_str) = mode_str {
        match m_str.parse::<tetherly_core::ScreenPacerMode>() {
            Ok(mode) => {
                let rep = node.screen_set_pacer_mode(mode);
                json_ok(&rep)
            }
            Err(_) => err(
                400,
                "invalid pacer mode: expected auto, aggressive, or disabled",
            ),
        }
    } else {
        let rep = node.screen_pacer_report();
        json_ok(&rep)
    }
}

pub(super) async fn get_screen_harq(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_harq_report();
    json_ok(&rep)
}

// Screen transport-wide congestion control (TWCC) pipeline (v2.08 / ADR-094).
pub(super) async fn post_screen_harq(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct HarqBody {
        mode: Option<String>,
        harq_mode: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<HarqBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.harq_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("harq_mode").cloned());

    if let Some(m_str) = mode_str {
        match m_str.parse::<tetherly_core::ScreenHarqMode>() {
            Ok(mode) => {
                let rep = node.screen_set_harq_mode(mode);
                json_ok(&rep)
            }
            Err(_) => err(
                400,
                "invalid harq mode: expected auto, aggressive, conservative, or disabled",
            ),
        }
    } else {
        let rep = node.screen_harq_report();
        json_ok(&rep)
    }
}

pub(super) async fn get_screen_twcc(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_twcc_report();
    json_ok(&rep)
}

// Screen active bandwidth probing & fast ramp-up pipeline (v2.09 / ADR-095).
pub(super) async fn post_screen_twcc(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct TwccBody {
        mode: Option<String>,
        twcc_mode: Option<String>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TwccBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.twcc_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("twcc_mode").cloned());

    if let Some(m_str) = mode_str {
        match m_str.parse::<tetherly_core::ScreenTwccMode>() {
            Ok(mode) => {
                let rep = node.screen_set_twcc_mode(mode);
                json_ok(&rep)
            }
            Err(_) => err(
                400,
                "invalid twcc mode: expected auto, aggressive, conservative, or disabled",
            ),
        }
    } else {
        let rep = node.screen_twcc_report();
        json_ok(&rep)
    }
}

pub(super) async fn get_screen_sync(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_sync_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_sync(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct SyncBody {
        mode: Option<String>,
        sync_mode: Option<String>,
        trigger: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<SyncBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.sync_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("sync_mode").cloned());

    let trigger_flag = parsed_body
        .as_ref()
        .and_then(|b| b.trigger)
        .or_else(|| qmap.get("trigger").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(false);

    if let Some(m_str) = mode_str {
        match m_str.parse::<tetherly_core::ScreenAudioSyncMode>() {
            Ok(mode) => {
                let _ = node.screen_set_sync_mode(mode);
            }
            Err(_) => {
                return err(
                    400,
                    "invalid sync mode: expected auto, strict, loose, or disabled",
                );
            }
        }
    }

    if trigger_flag {
        match node.trigger_screen_sync().await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(500, &format!("failed to trigger screen sync beacon: {e}")),
        }
    } else {
        let rep = node.screen_sync_report();
        json_ok(&rep)
    }
}

// Screen multi-display & virtual display stream multiplexing pipeline (v2.11 / ADR-097).
pub(super) async fn post_screen_sync_trigger(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.trigger_screen_sync().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to trigger screen sync beacon: {e}")),
    }
}

pub(super) async fn get_screen_latency(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_latency_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_latency(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct LatencyBody {
        mode: Option<String>,
        profiler_mode: Option<String>,
        reset: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<LatencyBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.profiler_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("profiler_mode").cloned());

    let reset_flag = parsed_body
        .as_ref()
        .and_then(|b| b.reset)
        .or_else(|| qmap.get("reset").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(false);

    if let Some(m_str) = mode_str {
        match m_str.parse::<tetherly_core::ScreenProfilerMode>() {
            Ok(mode) => {
                let _ = node.screen_set_latency_mode(mode);
            }
            Err(_) => {
                return err(
                    400,
                    "invalid latency profiler mode: expected auto, detailed, or disabled",
                );
            }
        }
    }

    if reset_flag {
        match node.screen_reset_latency().await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(500, &format!("failed to reset latency profiler: {e}")),
        }
    } else {
        json_ok(&node.screen_latency_report())
    }
}

pub(super) async fn post_screen_latency_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_latency().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset latency profiler: {e}")),
    }
}

pub(super) async fn get_screen_tuning(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_tuning_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_tuning(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize)]
    struct TuningBody {
        mode: Option<String>,
        tuner_mode: Option<String>,
        reset: Option<bool>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<TuningBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.clone().or_else(|| b.tuner_mode.clone()))
        .or_else(|| qmap.get("mode").cloned())
        .or_else(|| qmap.get("tuner_mode").cloned());

    let reset_flag = parsed_body
        .as_ref()
        .and_then(|b| b.reset)
        .or_else(|| qmap.get("reset").and_then(|v| v.parse::<bool>().ok()))
        .unwrap_or(false);

    if let Some(m_str) = mode_str {
        match m_str.parse::<tetherly_core::ScreenTunerMode>() {
            Ok(mode) => {
                let _ = node.screen_set_tuning_mode(mode).await;
            }
            Err(_) => {
                return err(
                    400,
                    "invalid tuning mode: expected disabled, standard, or aggressive",
                );
            }
        }
    }

    if reset_flag {
        match node.screen_reset_tuning().await {
            Ok(rep) => json_ok(&rep),
            Err(e) => err(500, &format!("failed to reset autonomous tuning: {e}")),
        }
    } else {
        json_ok(&node.screen_tuning_report())
    }
}

pub(super) async fn post_screen_tuning_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_tuning().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset autonomous tuning: {e}")),
    }
}

pub(super) async fn get_screen_color(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_color_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_color(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct ColorBody {
        mode: Option<String>,
        threshold: Option<u8>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<ColorBody> = if !body.is_empty() {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    let mode_str = parsed_body
        .as_ref()
        .and_then(|b| b.mode.as_deref())
        .or_else(|| qmap.get("mode").map(|s| s.as_str()))
        .unwrap_or("full");
    let mode: tetherly_core::ScreenColorMode = match mode_str.parse() {
        Ok(m) => m,
        Err(e) => return err(400, &format!("invalid color mode: {e}")),
    };
    let threshold = parsed_body
        .as_ref()
        .and_then(|b| b.threshold)
        .or_else(|| qmap.get("threshold").and_then(|v| v.parse::<u8>().ok()));

    match node.screen_set_color_mode(mode, threshold).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen color mode: {e}")),
    }
}

// Screen differential delta patch, temporal compression & macroblock dictionary cache pipeline (v2.23 / ADR-109)
pub(super) async fn post_screen_color_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_color_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen color mode: {e}")),
    }
}

pub(super) async fn get_screen_delta(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let rep = node.screen_delta_report();
    json_ok(&rep)
}

pub(super) async fn post_screen_delta(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    #[derive(serde::Deserialize, Default)]
    struct DeltaBody {
        enabled: Option<bool>,
        keyframe_interval: Option<u16>,
        interval: Option<u16>,
    }
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<DeltaBody> = if !body.is_empty() {
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
        .and_then(|b| b.keyframe_interval.or(b.interval))
        .or_else(|| {
            qmap.get("keyframe_interval")
                .or_else(|| qmap.get("interval"))
                .and_then(|v| v.parse::<u16>().ok())
        });

    match node.screen_set_delta_mode(enabled, interval).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to set screen delta mode: {e}")),
    }
}

// Screen objective visual quality metric, PSNR/SSIM distortion profiling pipeline (v2.24 / ADR-110)
pub(super) async fn post_screen_delta_reset(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    match node.screen_reset_delta_mode().await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(500, &format!("failed to reset screen delta mode: {e}")),
    }
}
