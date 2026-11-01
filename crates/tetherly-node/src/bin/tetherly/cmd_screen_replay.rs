// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen latency, tuning, recording and playback.

use super::cmd_screen_session::print_screen_snapshot_report;
use super::util::urlencode_path;
use super::*;

/// `tetherly screen latency` -- Latency arm of the screen dispatcher.
pub(crate) async fn screen_latency(
    action: Option<ScreenLatencySubCmd>,
    mode: Option<String>,
    reset: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let (target_mode, target_reset, is_json, port) = match action {
        Some(ScreenLatencySubCmd::Set {
            mode: m,
            json: j,
            ui_port: p,
        }) => (Some(m), false, j || json, p),
        Some(ScreenLatencySubCmd::Reset {
            json: j,
            ui_port: p,
        }) => (None, true, j || json, p),
        Some(ScreenLatencySubCmd::Status {
            json: j,
            ui_port: p,
        }) => (None, false, j || json, p),
        None => (mode, reset, json, ui_port),
    };

    if target_reset && target_mode.is_none() {
        let (code, resp) = tetherly_node::loopback_post(port, "/api/screen/latency/reset", &[])
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "screen latency reset failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if is_json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let rep: tetherly_core::ScreenLatencyReport =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            println!("Screen latency profiler statistics and ring buffer reset successfully.");
            print_screen_latency_report(&rep);
        }
    } else if target_mode.is_some() || target_reset {
        let mut body_map = serde_json::Map::new();
        if let Some(m) = target_mode {
            body_map.insert("mode".into(), serde_json::json!(m));
        }
        if target_reset {
            body_map.insert("reset".into(), serde_json::json!(true));
        }
        let body = serde_json::to_vec(&body_map)?;
        let (code, resp) = tetherly_node::loopback_post(port, "/api/screen/latency", &body)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "screen latency configuration failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if is_json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let rep: tetherly_core::ScreenLatencyReport =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            println!("Screen latency profiler mode updated: {}", rep.mode);
            print_screen_latency_report(&rep);
        }
    } else {
        let (code, resp) = tetherly_node::loopback_get(port, "/api/screen/latency")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "screen latency query failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if is_json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let rep: tetherly_core::ScreenLatencyReport =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            print_screen_latency_report(&rep);
        }
    }
    Ok(())
}

/// `tetherly screen tuning` -- Tuning arm of the screen dispatcher.
pub(crate) async fn screen_tuning(
    action: Option<ScreenTuningSubCmd>,
    mode: Option<String>,
    reset: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let (target_mode, target_reset, is_json, port) = match action {
        Some(ScreenTuningSubCmd::Set {
            mode: m,
            json: j,
            ui_port: p,
        }) => (Some(m), false, j || json, p),
        Some(ScreenTuningSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => (None, true, j || json, p),
        Some(ScreenTuningSubCmd::Status {
            json: j,
            ui_port: p,
        }) => (None, false, j || json, p),
        None => (mode, reset, json, ui_port),
    };

    if target_reset && target_mode.is_none() {
        let (code, resp) = tetherly_node::loopback_post(port, "/api/screen/tuning/reset", &[])
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "screen tuning reset failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if is_json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let rep: tetherly_core::ScreenTuningReport =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            println!("Screen autonomous tuning statistics and recovery stack reset successfully.");
            print_screen_tuning_report(&rep);
        }
    } else if target_mode.is_some() || target_reset {
        let mut body_map = serde_json::Map::new();
        if let Some(m) = target_mode {
            body_map.insert("mode".into(), serde_json::json!(m));
        }
        if target_reset {
            body_map.insert("reset".into(), serde_json::json!(true));
        }
        let body = serde_json::to_vec(&body_map)?;
        let (code, resp) = tetherly_node::loopback_post(port, "/api/screen/tuning", &body)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "screen tuning configuration failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if is_json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let rep: tetherly_core::ScreenTuningReport =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            println!("Screen autonomous tuner mode updated: {}", rep.mode);
            print_screen_tuning_report(&rep);
        }
    } else {
        let (code, resp) = tetherly_node::loopback_get(port, "/api/screen/tuning")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "screen tuning query failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if is_json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let rep: tetherly_core::ScreenTuningReport =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            print_screen_tuning_report(&rep);
        }
    }
    Ok(())
}

/// `tetherly screen recording` -- Recording arm of the screen dispatcher.
pub(crate) async fn screen_recording(
    action: Option<ScreenRecordingSubCmd>,
    out: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let (action, out, is_json, port) = match action {
        Some(ScreenRecordingSubCmd::Start {
            out: o,
            json: j,
            ui_port: p,
        }) => (Some("start"), o, j || json, p),
        Some(ScreenRecordingSubCmd::Stop {
            json: j,
            ui_port: p,
        }) => (Some("stop"), None, j || json, p),
        Some(ScreenRecordingSubCmd::Status {
            json: j,
            ui_port: p,
        }) => (None, None, j || json, p),
        Some(ScreenRecordingSubCmd::Inspect {
            path,
            limit,
            json: j,
            ui_port: p,
        }) => {
            let q = format!("path={}&limit={}", urlencode_path(&path), limit);
            let (code, resp) =
                tetherly_node::loopback_get(p, &format!("/api/screen/recording/replay?{q}"))
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{p} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "recording inspect failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if j || json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_recording_replay(&rep);
            }
            return Ok(());
        }
        Some(ScreenRecordingSubCmd::Snapshot {
            path,
            frame,
            pts,
            out,
            json: j,
            ui_port: p,
        }) => {
            let mut body_map = serde_json::Map::new();
            body_map.insert("path".into(), serde_json::json!(path));
            if let Some(f) = frame {
                body_map.insert("frame".into(), serde_json::json!(f));
            }
            if let Some(pt) = pts {
                body_map.insert("pts_us".into(), serde_json::json!(pt));
            }
            if let Some(o) = out {
                body_map.insert("out".into(), serde_json::json!(o));
            }
            body_map.insert("format".into(), serde_json::json!("bmp"));
            let payload = serde_json::Value::Object(body_map);
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) =
                tetherly_node::loopback_post(p, "/api/screen/recording/snapshot", &bytes)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{p} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "recording frame snapshot failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if j || json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_snapshot_report(&rep);
            }
            return Ok(());
        }
        None => (None, out, json, ui_port),
    };

    match action {
        Some("start") => {
            let mut body_map = serde_json::Map::new();
            if let Some(o) = out {
                body_map.insert("path".into(), serde_json::json!(o));
            }
            let body = serde_json::to_vec(&body_map)?;
            let (code, resp) =
                tetherly_node::loopback_post(port, "/api/screen/recording/start", &body)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen recording start failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen session recording started.");
                print_screen_recording_report(&rep);
            }
        }
        Some("stop") => {
            let (code, resp) =
                tetherly_node::loopback_post(port, "/api/screen/recording/stop", &[])
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen recording stop failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen session recording stopped and flushed to disk.");
                print_screen_recording_report(&rep);
            }
        }
        _ => {
            let (code, resp) = tetherly_node::loopback_get(port, "/api/screen/recording")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen recording query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_recording_report(&rep);
            }
        }
    }
    Ok(())
}

/// `tetherly screen playback` -- Playback arm of the screen dispatcher.
pub(crate) async fn screen_playback(
    action: Option<ScreenPlaybackSubCmd>,
    path: Option<String>,
    speed: Option<f32>,
    r#loop: bool,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let (action, path_opt, speed_opt, loop_opt, is_json, port) = match action {
        Some(ScreenPlaybackSubCmd::Start {
            path: p,
            speed: s,
            r#loop: l,
            json: j,
            ui_port: port,
        }) => (
            Some("start"),
            Some(p),
            s.or(speed),
            Some(l || r#loop),
            j || json,
            port,
        ),
        Some(ScreenPlaybackSubCmd::Pause {
            json: j,
            ui_port: port,
        }) => (Some("pause"), None, None, None, j || json, port),
        Some(ScreenPlaybackSubCmd::Resume {
            json: j,
            ui_port: port,
        }) => (Some("resume"), None, None, None, j || json, port),
        Some(ScreenPlaybackSubCmd::Stop {
            json: j,
            ui_port: port,
        }) => (Some("stop"), None, None, None, j || json, port),
        Some(ScreenPlaybackSubCmd::Status {
            json: j,
            ui_port: port,
        }) => (None, None, None, None, j || json, port),
        None => {
            if let Some(p) = path {
                (Some("start"), Some(p), speed, Some(r#loop), json, ui_port)
            } else {
                (None, None, None, None, json, ui_port)
            }
        }
    };

    match action {
        Some("start") => {
            let Some(p) = path_opt else {
                anyhow::bail!(
                    "playback start requires a container path; e.g. `screen playback start <path>`"
                );
            };
            let mut body_map = serde_json::Map::new();
            body_map.insert("path".into(), serde_json::json!(p));
            if let Some(s) = speed_opt {
                body_map.insert("speed".into(), serde_json::json!(s));
            }
            if let Some(l) = loop_opt {
                body_map.insert("loop".into(), serde_json::json!(l));
            }
            let body = serde_json::to_vec(&body_map)?;
            let (code, resp) =
                tetherly_node::loopback_post(port, "/api/screen/playback/start", &body)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen playback start failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen session in-container playback started.");
                print_screen_playback_report(&rep);
            }
        }
        Some("pause") => {
            let (code, resp) =
                tetherly_node::loopback_post(port, "/api/screen/playback/pause", &[])
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen playback pause failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen session playback paused.");
                print_screen_playback_report(&rep);
            }
        }
        Some("resume") => {
            let (code, resp) =
                tetherly_node::loopback_post(port, "/api/screen/playback/resume", &[])
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen playback resume failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen session playback resumed.");
                print_screen_playback_report(&rep);
            }
        }
        Some("stop") => {
            let (code, resp) = tetherly_node::loopback_post(port, "/api/screen/playback/stop", &[])
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen playback stop failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen session playback stopped.");
                print_screen_playback_report(&rep);
            }
        }
        _ => {
            let (code, resp) = tetherly_node::loopback_get(port, "/api/screen/playback")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen playback query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_playback_report(&rep);
            }
        }
    }
    Ok(())
}

pub(crate) fn print_screen_latency_report(rep: &tetherly_core::ScreenLatencyReport) {
    println!("Screen End-to-End Microsecond Latency Breakdown & Multi-Stage Profiler (ADR-099):");
    println!("  Profiler Mode:        {}", rep.mode);
    println!("  Dominant Bottleneck:  {}", rep.bottleneck);
    println!("  Frames Profiled:      {}", rep.frames_profiled);
    if rep.frames_profiled == 0 {
        println!("  (No telemetry frames profiled yet; start an active screen streaming session to collect latency breakdown)");
        return;
    }
    println!("  Latest Frame Seq:     #{}", rep.current.frame_seq);
    let b = &rep.current;
    println!("  Stage Breakdown (Latest Frame):");
    println!(
        "    Capture (OS):       {:>6.2} ms ({:>7} us)",
        b.capture_us as f64 / 1000.0,
        b.capture_us
    );
    println!(
        "    Encode (Codec):     {:>6.2} ms ({:>7} us)",
        b.encode_us as f64 / 1000.0,
        b.encode_us
    );
    println!(
        "    Pacer (Outbound):   {:>6.2} ms ({:>7} us)",
        b.pacer_us as f64 / 1000.0,
        b.pacer_us
    );
    println!(
        "    Network (Transit):  {:>6.2} ms ({:>7} us)",
        b.network_us as f64 / 1000.0,
        b.network_us
    );
    println!(
        "    Jitter (Playout):   {:>6.2} ms ({:>7} us)",
        b.jitter_us as f64 / 1000.0,
        b.jitter_us
    );
    println!(
        "    Decode (Receiver):  {:>6.2} ms ({:>7} us)",
        b.decode_us as f64 / 1000.0,
        b.decode_us
    );
    println!(
        "    Render (Display):   {:>6.2} ms ({:>7} us)",
        b.render_us as f64 / 1000.0,
        b.render_us
    );
    println!(
        "    Total End-to-End:   {:>6.2} ms ({:>7} us)",
        b.total_e2e_us as f64 / 1000.0,
        b.total_e2e_us
    );

    let tot = if b.total_e2e_us > 0 {
        b.total_e2e_us as f64
    } else {
        1.0
    };
    let tag = |stage: &str| {
        if rep.bottleneck.eq_ignore_ascii_case(stage) {
            " [BOTTLENECK]"
        } else {
            ""
        }
    };
    println!("  Stage Distribution (%):");
    println!(
        "    Capture:            {:>5.1}%{}",
        (b.capture_us as f64 / tot) * 100.0,
        tag("capture")
    );
    println!(
        "    Encode:             {:>5.1}%{}",
        (b.encode_us as f64 / tot) * 100.0,
        tag("encode")
    );
    println!(
        "    Pacer:              {:>5.1}%{}",
        (b.pacer_us as f64 / tot) * 100.0,
        tag("pacer")
    );
    println!(
        "    Network:            {:>5.1}%{}",
        (b.network_us as f64 / tot) * 100.0,
        tag("network")
    );
    println!(
        "    Jitter:             {:>5.1}%{}",
        (b.jitter_us as f64 / tot) * 100.0,
        tag("jitter")
    );
    println!(
        "    Decode:             {:>5.1}%{}",
        (b.decode_us as f64 / tot) * 100.0,
        tag("decode")
    );
    println!(
        "    Render:             {:>5.1}%{}",
        (b.render_us as f64 / tot) * 100.0,
        tag("render")
    );

    println!("  Latency Distribution (Total E2E):");
    println!(
        "    EWMA (Smoothed):    {:>6.2} ms ({:>7} us)",
        rep.avg_e2e_us as f64 / 1000.0,
        rep.avg_e2e_us
    );
    println!(
        "    P50 (Median):       {:>6.2} ms ({:>7} us)",
        rep.p50_e2e_us as f64 / 1000.0,
        rep.p50_e2e_us
    );
    println!(
        "    P90:                {:>6.2} ms ({:>7} us)",
        rep.p90_e2e_us as f64 / 1000.0,
        rep.p90_e2e_us
    );
    println!(
        "    P95:                {:>6.2} ms ({:>7} us)",
        rep.p95_e2e_us as f64 / 1000.0,
        rep.p95_e2e_us
    );
    println!(
        "    P99:                {:>6.2} ms ({:>7} us)",
        rep.p99_e2e_us as f64 / 1000.0,
        rep.p99_e2e_us
    );
}

pub(crate) fn print_screen_tuning_report(rep: &tetherly_core::ScreenTuningReport) {
    println!("Screen Bottleneck-Driven Autonomous Tuning Closed Loop (ADR-100):");
    println!("  Tuner Mode:           {}", rep.mode);
    println!("  Actions Applied:      {}", rep.actions_applied);
    println!("  Recoveries Applied:   {}", rep.recoveries_applied);
    println!("  Bottleneck Streak:    {} frames", rep.bottleneck_streak);
    println!("  Balanced Streak:      {} frames", rep.balanced_streak);
    println!("  Last Stage:           {}", rep.last_stage);
    println!("  Last Action:          {}", rep.last_action);
    if rep.actions_applied == 0 && rep.recoveries_applied == 0 && rep.last_decision.is_none() {
        println!("  (No autonomous tuning actions yet; the tuner retunes the pipeline when ADR-099 profiling reports a sustained dominant bottleneck)");
        return;
    }
    if let Some(d) = rep.last_decision.as_ref() {
        println!("  Last Decision (Frame #{}):", d.frame_seq);
        println!("    Stage:              {}", d.stage);
        println!(
            "    Action:             {}{}",
            d.action,
            if d.recovered { " (recovery)" } else { "" }
        );
        if let Some(bps) = d.new_bitrate_bps {
            println!("    New Bitrate:        {} bps", bps);
        }
        if let Some(fps) = d.new_fps {
            println!("    New FPS:            {}", fps);
        }
        if let Some(scale) = d.new_scale_pct {
            println!("    New Scale:          {}%", scale);
        }
        if let Some(qp) = d.new_qp_max {
            println!("    New QP Max:         {}", qp);
        }
        if let Some(delay) = d.new_playout_delay_us {
            println!("    New Playout Delay:  {} us", delay);
        }
    }
}

pub(crate) fn print_screen_recording_report(rep: &serde_json::Value) {
    println!("Screen Session Recording (TREC Container, ADR-101):");
    let active = rep.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
    let peer = rep
        .get("peer_recording")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let path = rep.get("path").and_then(|v| v.as_str()).unwrap_or("");
    println!("  Active:               {active}");
    println!("  Peer Recording:       {peer}");
    if !path.is_empty() {
        println!("  Container Path:       {path}");
    }
    if !active {
        if !peer {
            println!("  (No active recording; use `screen recording start` to record the locally presented stream)");
        }
        return;
    }
    let frames = rep
        .get("frames_recorded")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let bytes = rep
        .get("bytes_recorded")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let dur_us = rep.get("duration_us").and_then(|v| v.as_u64()).unwrap_or(0);
    let last_seq = rep
        .get("last_frame_seq")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let width = rep.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
    let height = rep.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
    println!("  Frames Recorded:      {frames}");
    println!("  Container Bytes:      {bytes}");
    println!(
        "  Duration:             {:.2} s",
        dur_us as f64 / 1_000_000.0
    );
    println!("  Last Frame Seq:       #{last_seq}");
    if width > 0 {
        println!("  Resolution:           {width}x{height}");
    }
}

pub(crate) fn print_screen_playback_report(rep: &serde_json::Value) {
    println!("Screen Session Playback (TREC Engine, ADR-102):");
    let state = rep.get("state").and_then(|v| v.as_str()).unwrap_or("idle");
    let path = rep.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let speed = rep.get("speed").and_then(|v| v.as_f64()).unwrap_or(1.0);
    let loop_enabled = rep
        .get("loop_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let current_frame = rep
        .get("current_frame")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let total_frames = rep
        .get("total_frames")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let elapsed_ms = rep.get("elapsed_ms").and_then(|v| v.as_u64()).unwrap_or(0);
    let duration_ms = rep.get("duration_ms").and_then(|v| v.as_u64()).unwrap_or(0);
    let width = rep.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
    let height = rep.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
    let format = rep
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let codec = rep.get("codec").and_then(|v| v.as_str()).unwrap_or("raw");

    println!("  State:                {state}");
    if !path.is_empty() {
        println!("  Container Path:       {path}");
    }
    println!(
        "  Progress:             frame {current_frame} / {total_frames} ({elapsed_ms} ms / {duration_ms} ms)"
    );
    println!("  Speed:                {speed:.1}x · Loop: {loop_enabled}");
    if width > 0 && height > 0 {
        println!("  Resolution:           {width}x{height} ({format}, {codec})");
    }
    if state == "idle" {
        println!("  (No active playback session; use `screen playback start <path>` to play a .trec recording)");
    }
}

pub(crate) fn print_screen_recording_replay(rep: &serde_json::Value) {
    println!("TREC Container Replay Inspection (ADR-101):");
    let path = rep.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let width = rep.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
    let height = rep.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
    let fmt = rep.get("format_tag").and_then(|v| v.as_u64()).unwrap_or(0);
    let codec = rep.get("codec_tag").and_then(|v| v.as_u64()).unwrap_or(0);
    let started = rep
        .get("started_at_us")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let returned = rep
        .get("records_returned")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let truncated = rep
        .get("truncated")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    println!("  Container Path:       {path}");
    println!("  Resolution:           {width}x{height}");
    println!("  Format/Codec Tag:     {fmt}/{codec}",);
    println!("  Started At:           {started} us");
    println!(
        "  Records Returned:     {returned}{}",
        if truncated { " (truncated)" } else { "" }
    );
    if let Some(records) = rep.get("records").and_then(|v| v.as_array()) {
        for r in records {
            let seq = r.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
            let pts = r.get("pts_us").and_then(|v| v.as_u64()).unwrap_or(0);
            let len = r.get("len").and_then(|v| v.as_u64()).unwrap_or(0);
            println!("    frame #{seq} pts={pts}us len={len}");
        }
    }
}
