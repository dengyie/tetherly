// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen quality analysis: metric/qoe/pir/ltr/svc/motion/delta.

use super::util::{loopback_get, loopback_post};
use super::*;

/// `tetherly-cli screen delta` -- Delta arm of the screen dispatcher.
pub(crate) async fn screen_delta(
    action: Option<ScreenDeltaSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        Some(ScreenDeltaSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_get(port, "/api/screen/delta").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "screen delta query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_delta_report(&rep);
            }
        }
        Some(ScreenDeltaSubCmd::Set {
            enabled,
            interval,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let payload = serde_json::json!({
                "enabled": enabled,
                "keyframe_interval": interval,
            });
            let (code, resp) =
                loopback_post(port, "/api/screen/delta", &serde_json::to_vec(&payload)?)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen delta set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen delta mode configured successfully.");
                print_screen_delta_report(&rep);
            }
        }
        Some(ScreenDeltaSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_post(port, "/api/screen/delta/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen delta reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen delta mode reset to default full-frame transmission.");
                print_screen_delta_report(&rep);
            }
        }
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/delta")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen delta query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_delta_report(&rep);
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen metric` -- Metric arm of the screen dispatcher.
pub(crate) async fn screen_metric(
    action: Option<ScreenMetricSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        Some(ScreenMetricSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_get(port, "/api/screen/metric")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen metric query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_quality_metric_report(&rep);
            }
        }
        Some(ScreenMetricSubCmd::Set {
            enabled,
            interval,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let payload = serde_json::json!({
                "enabled": enabled,
                "sample_interval": interval,
            });
            let (code, resp) =
                loopback_post(port, "/api/screen/metric", &serde_json::to_vec(&payload)?)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen metric set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen quality metric mode configured successfully.");
                print_screen_quality_metric_report(&rep);
            }
        }
        Some(ScreenMetricSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_post(port, "/api/screen/metric/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen metric reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen quality metric mode reset to default disabled state.");
                print_screen_quality_metric_report(&rep);
            }
        }
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/metric")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen metric query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_quality_metric_report(&rep);
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen qoe` -- Qoe arm of the screen dispatcher.
pub(crate) async fn screen_qoe(
    action: Option<ScreenQoeSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        Some(ScreenQoeSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_get(port, "/api/screen/qoe").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "screen qoe query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_qoe_report(&rep);
            }
        }
        Some(ScreenQoeSubCmd::Set {
            mode,
            target_psnr,
            target_ssim,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let payload = serde_json::json!({
                "mode": mode,
                "target_psnr": target_psnr,
                "target_ssim": target_ssim,
            });
            let (code, resp) =
                loopback_post(port, "/api/screen/qoe", &serde_json::to_vec(&payload)?)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen qoe set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen autonomous closed-loop QoE mode configured successfully.");
                print_screen_qoe_report(&rep);
            }
        }
        Some(ScreenQoeSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_post(port, "/api/screen/qoe/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen qoe reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen autonomous closed-loop QoE mode reset to default disabled state.");
                print_screen_qoe_report(&rep);
            }
        }
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/qoe")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen qoe query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_qoe_report(&rep);
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen pir` -- Pir arm of the screen dispatcher.
pub(crate) async fn screen_pir(
    action: Option<ScreenPirSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        Some(ScreenPirSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_get(port, "/api/screen/pir").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "screen pir query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_pir_report(&rep);
            }
        }
        Some(ScreenPirSubCmd::Set {
            mode,
            bands,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let payload = serde_json::json!({
                "mode": mode,
                "bands": bands,
            });
            let (code, resp) =
                loopback_post(port, "/api/screen/pir", &serde_json::to_vec(&payload)?)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen pir set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen Progressive Intra-Refresh (PIR) mode configured successfully.");
                print_screen_pir_report(&rep);
            }
        }
        Some(ScreenPirSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_post(port, "/api/screen/pir/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen pir reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen Progressive Intra-Refresh (PIR) reset to default disabled state.");
                print_screen_pir_report(&rep);
            }
        }
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/pir")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen pir query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_pir_report(&rep);
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen ltr` -- Ltr arm of the screen dispatcher.
pub(crate) async fn screen_ltr(
    action: Option<ScreenLtrSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        Some(ScreenLtrSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_get(port, "/api/screen/ltr").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "screen ltr query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_ltr_report(&rep);
            }
        }
        Some(ScreenLtrSubCmd::Set {
            enabled,
            interval,
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let payload = serde_json::json!({
                "enabled": enabled.unwrap_or(true),
                "auto_interval": interval,
            });
            let (code, resp) =
                loopback_post(port, "/api/screen/ltr", &serde_json::to_vec(&payload)?)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                        )
                    })?;
            if code != 200 {
                anyhow::bail!(
                    "screen ltr set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen Long-Term Reference (LTR) mode configured successfully.");
                print_screen_ltr_report(&rep);
            }
        }
        Some(ScreenLtrSubCmd::Anchor {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_post(port, "/api/screen/ltr/anchor", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen ltr anchor failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen Long-Term Reference (LTR) golden anchor snapshot captured.");
                print_screen_ltr_report(&rep);
            }
        }
        Some(ScreenLtrSubCmd::Recover {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_post(port, "/api/screen/ltr/recover", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen ltr recovery request failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!(
                    "Screen Long-Term Reference (LTR) differential recovery request dispatched."
                );
                print_screen_ltr_report(&rep);
            }
        }
        Some(ScreenLtrSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let port = if p != 45716 { p } else { ui_port };
            let is_json = j || json;
            let (code, resp) = loopback_post(port, "/api/screen/ltr/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen ltr reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if is_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                println!("Screen Long-Term Reference (LTR) reset to default disabled state.");
                print_screen_ltr_report(&rep);
            }
        }
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/ltr")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen ltr query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_ltr_report(&rep);
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen svc` -- Svc arm of the screen dispatcher.
pub(crate) async fn screen_svc(
    action: Option<ScreenSvcSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/svc")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen svc query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_svc_report(&rep);
            }
        }
        Some(ScreenSvcSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/svc").await.map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
            if code != 200 {
                anyhow::bail!(
                    "screen svc query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_svc_report(&rep);
            }
        }
        Some(ScreenSvcSubCmd::Set {
            mode,
            drop_t1,
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let mut payload = serde_json::Map::new();
            if let Some(m) = mode {
                payload.insert("mode".to_string(), serde_json::Value::String(m));
            }
            if let Some(d) = drop_t1 {
                payload.insert("drop_t1".to_string(), serde_json::Value::Bool(d));
            }
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/svc", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen svc set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_svc_report(&rep);
            }
        }
        Some(ScreenSvcSubCmd::DropT1 {
            drop,
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let payload = serde_json::json!({
                "drop_t1": drop.unwrap_or(true)
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/svc/drop-t1", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen svc drop-t1 failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_svc_report(&rep);
            }
        }
        Some(ScreenSvcSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_post(port, "/api/screen/svc/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen svc reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_svc_report(&rep);
            }
        }
    }
    Ok(())
}

/// `tetherly-cli screen motion` -- Motion arm of the screen dispatcher.
pub(crate) async fn screen_motion(
    action: Option<ScreenMotionSubCmd>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    match action {
        None => {
            let (code, resp) = loopback_get(ui_port, "/api/screen/motion")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen motion query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_motion_report(&rep);
            }
        }
        Some(ScreenMotionSubCmd::Status {
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_get(port, "/api/screen/motion")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen motion query failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_motion_report(&rep);
            }
        }
        Some(ScreenMotionSubCmd::Set {
            enabled,
            max_displacement,
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let mut payload = serde_json::Map::new();
            if let Some(en) = enabled {
                payload.insert("enabled".to_string(), serde_json::Value::Bool(en));
            }
            if let Some(d) = max_displacement {
                payload.insert(
                    "max_displacement".to_string(),
                    serde_json::Value::Number(serde_json::Number::from(d)),
                );
            }
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = loopback_post(port, "/api/screen/motion", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                 start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen motion set failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_motion_report(&rep);
            }
        }
        Some(ScreenMotionSubCmd::Reset {
            json: j,
            ui_port: p,
        }) => {
            let effective_json = json || j;
            let port = if p != 45716 { p } else { ui_port };
            let (code, resp) = loopback_post(port, "/api/screen/motion/reset", b"{}")
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{port} ({e}); \
                                     start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "screen motion reset failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            if effective_json {
                println!("{}", String::from_utf8_lossy(&resp).trim());
            } else {
                let rep: serde_json::Value = serde_json::from_slice(&resp)
                    .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
                print_screen_motion_report(&rep);
            }
        }
    }
    Ok(())
}

pub(crate) fn print_screen_delta_report(rep: &serde_json::Value) {
    println!("Screen Differential Delta Patch & Temporal Compression (v2.23 / ADR-109):");
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let interval = rep
        .get("keyframe_interval")
        .and_then(|v| v.as_u64())
        .unwrap_or(60);
    let status_str = if enabled {
        "enabled (macroblock differential dirty sub-rectangle)"
    } else {
        "disabled (full frame transmission)"
    };
    println!("  Status:             {status_str}");
    println!("  Keyframe Interval:  {interval} frames");
    let delta_sent = rep
        .get("delta_frames_sent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let keyframes_sent = rep
        .get("keyframes_sent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let delta_recv = rep
        .get("delta_frames_received")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let keyframes_recv = rep
        .get("keyframes_received")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let saved_bytes = rep.get("saved_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
    println!("  Sent:               delta frames: {delta_sent} · keyframes: {keyframes_sent}");
    println!("  Received:           delta frames: {delta_recv} · keyframes: {keyframes_recv}");
    println!(
        "  Bandwidth Saved:    {saved_bytes} B ({:.2} KB)",
        saved_bytes as f64 / 1024.0
    );
}

pub(crate) fn print_screen_quality_metric_report(rep: &serde_json::Value) {
    println!("Screen Objective Visual Quality Metric Pipeline (v2.24 / ADR-110):");
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let interval = rep
        .get("sample_interval")
        .and_then(|v| v.as_u64())
        .unwrap_or(10);
    let status_str = if enabled {
        "enabled (real-time MSE/PSNR/SSIM distortion profiling)"
    } else {
        "disabled"
    };
    println!("  Status:             {status_str}");
    println!("  Sample Interval:    every {interval} frames");
    let evaluated = rep
        .get("evaluated_frames")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let avg_mse = rep.get("avg_mse").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let avg_psnr = rep
        .get("avg_psnr_db")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let avg_ssim = rep.get("avg_ssim").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let last_psnr = rep
        .get("last_psnr_db")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let last_ssim = rep.get("last_ssim").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let warning = rep
        .get("quality_warning")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let warn_str = if warning {
        " [WARNING: PSNR < 30.0 dB or SSIM < 0.85]"
    } else {
        " [OK]"
    };
    println!("  Evaluated Frames:   {evaluated}");
    println!("  Average MSE:        {avg_mse:.4}");
    println!("  Average PSNR:       {avg_psnr:.2} dB");
    println!("  Average SSIM:       {avg_ssim:.4}");
    println!("  Last Frame PSNR:    {last_psnr:.2} dB");
    println!("  Last Frame SSIM:    {last_ssim:.4}");
    println!("  Quality Health:    {warn_str}");
}

pub(crate) fn print_screen_qoe_report(rep: &serde_json::Value) {
    println!("Screen Autonomous Closed-Loop QoE Adaptation Pipeline (v2.25 / ADR-111):");
    let mode = rep.get("mode").and_then(|v| v.as_str()).unwrap_or("off");
    let target_psnr = rep
        .get("target_psnr_db")
        .and_then(|v| v.as_u64())
        .unwrap_or(30);
    let target_ssim = rep
        .get("target_ssim_pct")
        .and_then(|v| v.as_u64())
        .unwrap_or(85);
    let in_cooldown = rep
        .get("in_cooldown")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let actions_total = rep
        .get("actions_total")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let last_action = rep
        .get("last_action")
        .and_then(|v| v.as_str())
        .unwrap_or("hold");
    let last_reason = rep
        .get("last_action_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("none");
    let active_boosts = rep
        .get("active_boosts")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let status_str = match mode {
        "conservative" => "Conservative (Fidelity prioritized, PSNR >= 34 dB / SSIM >= 0.90)",
        "balanced" => "Balanced (Adaptive stability, PSNR >= 30 dB / SSIM >= 0.85)",
        "economy" => "Economy (Bandwidth preservation, PSNR >= 26 dB / SSIM >= 0.78)",
        _ => "Off (Closed-loop adaptation disabled)",
    };
    println!("  Policy Mode:        {status_str}");
    println!("  Target PSNR:        {target_psnr} dB");
    println!(
        "  Target SSIM:        {target_ssim}% ({:.2})",
        target_ssim as f64 / 100.0
    );
    println!("  Actions Total:      {actions_total}");
    println!("  Last Action:        {last_action}");
    println!("  Action Reason:      {last_reason}");
    println!("  In Cooldown:        {in_cooldown}");
    println!("  Active Boosts:      {active_boosts}");
}

pub(crate) fn print_screen_pir_report(rep: &serde_json::Value) {
    println!("Screen Progressive Intra-Refresh (PIR) Anti-Burst Slice Pipeline (v2.26 / ADR-112):");
    let mode = rep
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("disabled");
    let bands = rep.get("bands").and_then(|v| v.as_u64()).unwrap_or(8);
    let sweeps_completed = rep
        .get("sweeps_completed")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let slices_sent = rep.get("slices_sent").and_then(|v| v.as_u64()).unwrap_or(0);
    let current_band = rep
        .get("current_band")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let active_sweep = rep
        .get("active_sweep")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let status_str = match mode {
        "continuous" => {
            "Continuous (Rolling band slices refresh 1 band per frame, zero I-frame burst)"
        }
        "on-demand" | "ondemand" => {
            "On-Demand (Triggered N-frame progressive sweep on loss/QoE request)"
        }
        _ => "Disabled (Traditional full keyframe refresh)",
    };
    println!("  Operating Mode:     {status_str}");
    println!("  Band Slices (N):    {bands}");
    println!("  Sweeps Completed:   {sweeps_completed}");
    println!("  Slices Sent:        {slices_sent}");
    println!("  Current Band Index: {current_band} / {bands}");
    println!("  Active Sweep:       {active_sweep}");
}

pub(crate) fn print_screen_ltr_report(rep: &serde_json::Value) {
    println!("Screen Long-Term Reference (LTR) Frame Anchoring & Gap Recovery Pipeline (v2.27 / ADR-113):");
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let auto_interval = rep
        .get("auto_interval")
        .and_then(|v| v.as_u64())
        .unwrap_or(60);
    let current_ltr_seq = rep
        .get("current_ltr_seq")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let anchors_established = rep
        .get("anchors_established")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let recoveries_requested = rep
        .get("recoveries_requested")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let recoveries_completed = rep
        .get("recoveries_completed")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let saved_burst_bytes = rep
        .get("saved_burst_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let status_str = if enabled {
        format!("Enabled (Golden reference anchor auto-refresh every {auto_interval} frames)")
    } else {
        "Disabled (Traditional IDR keyframe burst on packet loss)".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Auto-Anchor Interval:  {auto_interval} frames");
    println!("  Current LTR Anchor:    Seq #{current_ltr_seq}");
    println!("  Anchors Established:   {anchors_established}");
    println!("  Recoveries Dispatched: {recoveries_requested}");
    println!("  Recoveries Completed:  {recoveries_completed}");
    println!("  Saved Burst Traffic:   {saved_burst_bytes} bytes");
}

pub(crate) fn print_screen_svc_report(rep: &serde_json::Value) {
    println!(
        "Screen 2-Layer Temporal Scalability (SVC-T) Hierarchical Pipeline (v2.28 / ADR-114):"
    );
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mode = rep
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("disabled");
    let drop_t1 = rep
        .get("drop_t1")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let t0_frames_sent = rep
        .get("t0_frames_sent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let t1_frames_sent = rep
        .get("t1_frames_sent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let t1_frames_dropped = rep
        .get("t1_frames_dropped")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let saved_bandwidth_bytes = rep
        .get("saved_bandwidth_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let status_str = if enabled {
        format!("Enabled (mode: {mode}, drop_t1: {drop_t1})")
    } else {
        "Disabled (Flat delta prediction chain)".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Operational Mode:      {mode}");
    println!("  Drop T1 Active:        {drop_t1}");
    println!("  T0 Base Frames Sent:   {t0_frames_sent}");
    println!("  T1 Enh. Frames Sent:   {t1_frames_sent}");
    println!("  T1 Frames Dropped:     {t1_frames_dropped}");
    println!("  Saved Bandwidth:       {saved_bandwidth_bytes} bytes");
}

pub(crate) fn print_screen_motion_report(rep: &serde_json::Value) {
    println!(
        "Screen 2D Motion Vector Estimation & Scroll Acceleration Pipeline (v2.29 / ADR-115):"
    );
    let enabled = rep
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_displacement = rep
        .get("max_displacement")
        .and_then(|v| v.as_u64())
        .unwrap_or(64);
    let motion_frames_sent = rep
        .get("motion_frames_sent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let fallback_frames_sent = rep
        .get("fallback_frames_sent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let saved_bandwidth_bytes = rep
        .get("saved_bandwidth_bytes")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);

    let status_str = if enabled {
        format!("Enabled (max_displacement: {max_displacement}px)")
    } else {
        "Disabled".to_string()
    };
    println!("  Pipeline Status:       {status_str}");
    println!("  Search Radius:         {max_displacement} px");
    println!("  Motion Frames Sent:    {motion_frames_sent}");
    println!("  Fallback Frames Sent:  {fallback_frames_sent}");
    println!("  Saved Bandwidth:       {saved_bandwidth_bytes} bytes");
}
