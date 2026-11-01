// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `run`/`identity`/`pin`/`dial` plus node status, metrics, events and caps.

use super::util::{format_bytes, format_speed, format_uptime};
use super::*;

/// `tetherly identity` -- Identity arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn identity(cfg: NodeConfig) -> Result<()> {
    let node = Node::start(cfg)?;
    println!("{}", node.identity().device_id());
    Ok(())
}

/// `tetherly pin` -- Pin arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn pin(status: bool, clear: bool, json: bool, ui_port: u16) -> Result<()> {
    if clear {
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/pin/clear", b"{}")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node refused PIN clear request (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            let cleared = v.get("cleared").and_then(|c| c.as_bool()).unwrap_or(false);
            if cleared {
                println!("armed PIN successfully revoked and cleared.");
            } else {
                println!("no active PIN was armed.");
            }
        }
    } else if status {
        let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/pin")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node refused PIN status request (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            let active = v.get("active").and_then(|a| a.as_bool()).unwrap_or(false);
            if active {
                let pin = v.get("pin").and_then(|p| p.as_str()).unwrap_or("???? ????");
                let expires = v
                    .get("expires_in_secs")
                    .and_then(|e| e.as_u64())
                    .unwrap_or(0);
                let ttl = v.get("ttl_secs").and_then(|t| t.as_u64()).unwrap_or(180);
                println!("Active PIN: {pin} (valid for {expires}s / {ttl}s TTL)");
            } else {
                println!("No active PIN armed.");
            }
        }
    } else {
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/pin", b"{}")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node refused PIN request (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        if json {
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            let pin = v
                .get("pin")
                .and_then(|p| p.as_str())
                .ok_or_else(|| anyhow::anyhow!("node returned no pin"))?;
            let expires = v
                .get("expires_in_secs")
                .and_then(|e| e.as_u64())
                .unwrap_or(180);
            println!("{pin}");
            println!("不要把数字发给任何人,只在你自己的另一台设备上输入。(有效时间: {expires} 秒)");
        }
    }
    Ok(())
}

/// `tetherly run` -- Run arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    mut cfg: NodeConfig,
    port: u16,
    overlay_cidr: Option<String>,
    no_overlay: bool,
    screen_adb: Option<String>,
    screen_codec: String,
    screen_bitrate: String,
    screen_fps: String,
    screen_scale: String,
    screen_profile: String,
    screen_qp: Option<String>,
    screen_matrix: String,
    screen_playout: String,
    screen_cursor: String,
    screen_squelch: String,
    screen_fec: String,
    screen_pacer: String,
    screen_trendline: String,
    screen_harq: String,
    screen_twcc: String,
    screen_probe: String,
    screen_sync: String,
    screen_latency: String,
    screen_display: u8,
    file_port: Option<u16>,
    input_port: Option<u16>,
    screen_port: Option<u16>,
    ui_port: Option<u16>,
    ancs: Option<String>,
) -> Result<()> {
    cfg.control_port = port;
    // Real presentation: received frames open a window via GDI BitBlt.
    cfg.screen_sink = Some(Arc::new(SystemSink::new()));
    // Real injection for the controlled side (45719): received key and
    // mouse events drive SendInput. Harmless while no visitor holds
    // the seat (nothing arrives until the controller crosses an edge).
    cfg.input_sink = Some(Arc::new(std::sync::Mutex::new(
        tetherly_node::SystemInputSink,
    )));
    cfg.screen_bitrate_auto = screen_bitrate.eq_ignore_ascii_case("auto");
    if !cfg.screen_bitrate_auto {
        cfg.screen_bitrate_kbps = screen_bitrate
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("--screen-bitrate must be `auto` or a kbit/s number"))?;
    }
    cfg.screen_fps_auto = screen_fps.eq_ignore_ascii_case("auto");
    if !cfg.screen_fps_auto {
        cfg.screen_fps = screen_fps.trim().parse().map_err(|_| {
            anyhow::anyhow!("--screen-fps must be `auto` or an integer FPS (5..=120)")
        })?;
    }
    cfg.screen_scale_auto = screen_scale.eq_ignore_ascii_case("auto");
    if !cfg.screen_scale_auto {
        cfg.screen_scale_pct = screen_scale.trim().parse().map_err(|_| {
            anyhow::anyhow!("--screen-scale must be `auto` or an integer percentage (25..=100)")
        })?;
    }
    cfg.screen_profile_auto = screen_profile.eq_ignore_ascii_case("auto");
    if !cfg.screen_profile_auto {
        cfg.screen_profile = screen_profile
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("--screen-profile must be `auto`, `text`, or `motion`"))?;
    }
    if let Some(qp_str) = screen_qp {
        let parts: Vec<&str> = qp_str.split("..").collect();
        if parts.len() == 2 {
            let min_qp = parts[0]
                .trim()
                .parse::<u8>()
                .map_err(|_| anyhow::anyhow!("--screen-qp invalid min QP (0..=51)"))?;
            let max_qp = parts[1]
                .trim()
                .parse::<u8>()
                .map_err(|_| anyhow::anyhow!("--screen-qp invalid max QP (0..=51)"))?;
            cfg.screen_qp_min = min_qp.clamp(0, 51);
            cfg.screen_qp_max = max_qp.clamp(0, 51);
        } else if let Ok(single) = qp_str.trim().parse::<u8>() {
            cfg.screen_qp_min = single.clamp(0, 51);
            cfg.screen_qp_max = single.clamp(0, 51);
        } else {
            anyhow::bail!(
                "--screen-qp must be `min..max` (e.g. `12..35`) or a single integer (0..=51)"
            );
        }
    }
    cfg.screen_matrix_mode = match screen_matrix.trim().to_lowercase().as_str() {
        "auto" => tetherly_core::ScreenMatrixMode::Auto,
        "conservative" => tetherly_core::ScreenMatrixMode::Conservative,
        "manual" => tetherly_core::ScreenMatrixMode::Manual,
        _ => anyhow::bail!("--screen-matrix must be `auto`, `conservative`, or `manual`"),
    };
    cfg.screen_playout_mode = match screen_playout.trim().to_lowercase().as_str() {
        "adaptive" | "auto" => tetherly_core::ScreenPlayoutMode::Adaptive,
        "zerolatency" | "zero_latency" | "low_latency" | "interactive" => {
            tetherly_core::ScreenPlayoutMode::ZeroLatency
        }
        "smooth" | "video" => tetherly_core::ScreenPlayoutMode::Smooth,
        _ => {
            anyhow::bail!("--screen-playout must be `adaptive`, `zerolatency`, or `smooth`")
        }
    };
    cfg.screen_cursor_mode = match screen_cursor.trim().to_lowercase().as_str() {
        "decoupled" | "decouple" | "hw" | "hardware" => tetherly_core::ScreenCursorMode::Decoupled,
        "embedded" | "video" | "embed" => tetherly_core::ScreenCursorMode::Embedded,
        "hidden" | "hide" | "none" => tetherly_core::ScreenCursorMode::Hidden,
        _ => {
            anyhow::bail!("--screen-cursor must be `decoupled`, `embedded`, or `hidden`")
        }
    };
    cfg.screen_squelch_mode = match screen_squelch.trim().to_lowercase().as_str() {
        "auto" => tetherly_core::ScreenSquelchMode::Auto,
        "aggressive" => tetherly_core::ScreenSquelchMode::Aggressive,
        "disabled" | "off" | "none" => tetherly_core::ScreenSquelchMode::Disabled,
        _ => {
            anyhow::bail!("--screen-squelch must be `auto`, `aggressive`, or `disabled`")
        }
    };
    cfg.screen_fec_mode = match screen_fec.trim().parse::<tetherly_core::ScreenFecMode>() {
        Ok(mode) => mode,
        Err(_) => {
            anyhow::bail!(
                "--screen-fec must be `auto`, `conservative`, `aggressive`, or `disabled`"
            )
        }
    };
    cfg.screen_pacer_mode = match screen_pacer
        .trim()
        .parse::<tetherly_core::ScreenPacerMode>()
    {
        Ok(mode) => mode,
        Err(_) => {
            anyhow::bail!("--screen-pacer must be `auto`, `aggressive`, or `disabled`")
        }
    };
    cfg.screen_trendline_mode = match screen_trendline
        .trim()
        .parse::<tetherly_core::ScreenTrendlineMode>()
    {
        Ok(mode) => mode,
        Err(_) => {
            anyhow::bail!(
                "--screen-trendline must be `auto`, `aggressive`, `conservative`, or `disabled`"
            )
        }
    };
    cfg.screen_harq_mode = match screen_harq.trim().parse::<tetherly_core::ScreenHarqMode>() {
        Ok(mode) => mode,
        Err(_) => {
            anyhow::bail!(
                "--screen-harq must be `auto`, `aggressive`, `conservative`, or `disabled`"
            )
        }
    };
    cfg.screen_twcc_mode = match screen_twcc.trim().parse::<tetherly_core::ScreenTwccMode>() {
        Ok(mode) => mode,
        Err(_) => {
            anyhow::bail!(
                "--screen-twcc must be `auto`, `aggressive`, `conservative`, or `disabled`"
            )
        }
    };
    cfg.screen_probe_mode = match screen_probe
        .trim()
        .parse::<tetherly_core::ScreenProbeMode>()
    {
        Ok(mode) => mode,
        Err(_) => {
            anyhow::bail!(
                "--screen-probe must be `auto`, `aggressive`, `conservative`, or `disabled`"
            )
        }
    };
    cfg.screen_sync_mode = match screen_sync
        .trim()
        .parse::<tetherly_core::ScreenAudioSyncMode>()
    {
        Ok(mode) => mode,
        Err(_) => {
            anyhow::bail!("--screen-sync must be `auto`, `strict`, `loose`, or `disabled`")
        }
    };
    cfg.screen_latency_mode = match screen_latency
        .trim()
        .parse::<tetherly_core::ScreenProfilerMode>()
    {
        Ok(mode) => mode,
        Err(_) => {
            anyhow::bail!("--screen-latency must be `auto`, `detailed`, or `disabled`")
        }
    };
    cfg.screen_display_id = screen_display;
    let h264 = screen_codec.eq_ignore_ascii_case("h264");
    cfg.screen_codec_h264 = h264;
    let encode_codec: Arc<dyn tetherly_core::ScreenCodec> = if h264 {
        let sel = std::sync::Arc::new(tetherly_node::NegotiatedCodec::new(
            true,
            cfg.screen_bitrate_kbps.saturating_mul(1000),
            false,
        ));
        sel.set_fps(cfg.screen_fps);
        sel.set_scale_pct(cfg.screen_scale_pct);
        sel.set_content_profile(cfg.screen_profile);
        if cfg.screen_qp_min != tetherly_core::SCREEN_QP_MIN_DEFAULT
            || cfg.screen_qp_max != tetherly_core::SCREEN_QP_MAX_DEFAULT
        {
            sel.set_qp_range(cfg.screen_qp_min, cfg.screen_qp_max);
        }
        // The daemon's caps-merge hook flips this on when the peer
        // advertises screencodecs=h264; the CLI only sets the intent.
        sel.clone()
    } else {
        Arc::new(tetherly_node::ZstdCodec::default())
    };
    if let Some(serial) = screen_adb {
        // The screen is the adb device; encode it like DXGI.
        cfg.screen_source = Some(Arc::new(tetherly_node::EncodedSource::new(
            tetherly_node::AdbSource::new(Some(serial)),
            encode_codec,
        )));
    }
    if let Some(v) = file_port {
        cfg.file_port = v;
    }
    if let Some(v) = input_port {
        cfg.input_port = v;
    }
    if let Some(v) = screen_port {
        cfg.screen_port = v;
    }
    if let Some(v) = ui_port {
        cfg.ui_port = v;
    }
    if no_overlay {
        cfg.overlay.enabled = false;
    }
    if let Some(c) = overlay_cidr {
        cfg.overlay.cidrs = tetherly_node::parse_overlay_cidrs(&c);
    }
    // ANCS is optional and, on targets without a stack yet, honestly
    // absent; the pump below is only started when a link comes up.
    let ancs_link = match ancs.as_deref() {
        Some(address) => match tetherly_node::ancs::open(address) {
            Ok(Some(link)) => Some((address.to_string(), link)),
            Ok(None) => {
                info!("ANCS is not implemented on this platform; iPhone notifications stay off");
                None
            }
            Err(e) => {
                warn!(%e, "ANCS address {address} is malformed; iPhone notifications stay off");
                None
            }
        },
        None => None,
    };
    let (ancs_addr, inbound) = match ancs_link {
        Some((addr, (transport, inbound))) => {
            cfg.ancs = Some(transport);
            cfg.ancs_peripheral = addr.clone();
            (Some(addr), Some(inbound))
        }
        None => (None, None),
    };
    let node = Node::start(cfg)?;
    info!(device_id = %node.identity().device_id(), "run");
    info!("ui http://127.0.0.1:45716");
    if let Some(inbound) = inbound {
        node.spawn_ancs_pump(inbound);
        info!(
            peripheral = ?ancs_addr,
            "ANCS link up; pair the iPhone in system Bluetooth settings if you have not"
        );
    }
    // Controller-side capture: low-level mouse hook for edge crossings;
    // the keyboard hook is armed only while the seat is remote.
    if let Err(e) = node.spawn_input_capture() {
        info!(%e, "input capture unavailable (key/mouse sharing stays off)");
    }
    // Frame pump: grab + encode + send while a visitor is streaming.
    // Idle ticks are cheap (Ok(false)). Source errors are recoverable
    // (adb respawns on the next grab, DXGI retries): log and back off
    // instead of killing the pump, or the stream would be dead for
    // every future visitor until process restart.
    let pump = node.clone_handle();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(16));
        // A blocking grab overruns the 16ms tick constantly: skip
        // missed ticks instead of bursting catch-up grabs.
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut backoff = std::time::Duration::from_millis(250);
        loop {
            tick.tick().await;
            if !pump.screen_is_pacing_ready() {
                continue;
            }
            match pump.screen_send_frame().await {
                Ok(_) => backoff = std::time::Duration::from_millis(250),
                Err(e) => {
                    tracing::warn!(%e, "screen pump tick failed; backing off");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(std::time::Duration::from_secs(5));
                }
            }
        }
    });
    node.run().await?;
    Ok(())
}

/// `tetherly dial` -- Dial arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dial(
    addr: Option<SocketAddr>,
    peer: Option<String>,
    pin: Option<String>,
    ui_port: u16,
) -> Result<()> {
    if addr.is_none() && peer.is_none() {
        anyhow::bail!("either --addr <ip:port> or --peer <name|id> must be specified");
    }
    let payload = serde_json::json!({
        "addr": addr.map(|a| a.to_string()),
        "peer": peer,
        "pin": pin,
    });
    let body = serde_json::to_vec(&payload)?;
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/dial", &body)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node refused dial (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let target_desc = addr
        .map(|a| a.to_string())
        .or_else(|| peer.clone())
        .unwrap_or_default();
    println!("dialed {target_desc}");
    Ok(())
}

/// `tetherly discover` -- Discover arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn discover(rescan: bool, json: bool, ui_port: u16) -> Result<()> {
    if rescan {
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/discovery/rescan", b"{}")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node refused discovery rescan (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
    }
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/discovery")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node returned HTTP {code} on /api/discovery: {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }
    let rep: tetherly_node::DiscoveryReport = serde_json::from_slice(&resp)?;
    println!("Discovered LAN Peers ({}):", rep.count);
    if rep.peers.is_empty() {
        println!("  (no peers discovered on local network)");
    } else {
        for p in rep.peers {
            let trusted_mark = if p.is_trusted {
                "[trusted]"
            } else {
                "[unpaired]"
            };
            let online_mark = if p.is_online { "[online]" } else { "[idle]" };
            let fp_str = p.fp.as_deref().unwrap_or("none");
            let addrs_str = p.addrs.join(", ");
            println!(
                "  * {} ({}) {} {} - port: {}, fp: {}, caps: [{}], addrs: [{}]",
                p.name,
                p.device_id,
                trusted_mark,
                online_mark,
                p.port,
                fp_str,
                p.caps.join(", "),
                addrs_str
            );
        }
    }
    Ok(())
}

/// `tetherly status` -- Status arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn status(watch: bool, interval: u64, json: bool, ui_port: u16) -> Result<()> {
    loop {
        let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/state")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node refused status query (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
        } else {
            let v: serde_json::Value = serde_json::from_slice(&resp)
                .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
            if watch {
                print!("\x1b[2J\x1b[H");
            }
            print_status_summary(&v);
        }
        if !watch {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval.max(1))).await;
    }
    Ok(())
}

/// `tetherly sys` -- Sys arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn sys(watch: bool, interval: u64, json: bool, ui_port: u16) -> Result<()> {
    loop {
        let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/sys")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                             start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node refused system telemetry query (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp).trim());
        } else {
            let v: serde_json::Value = serde_json::from_slice(&resp)
                .map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
            if watch {
                print!("\x1b[2J\x1b[H");
            }
            print_system_report(&v);
        }
        if !watch {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(interval.max(1))).await;
    }
    Ok(())
}

/// `tetherly health` -- Health arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn health(json: bool, ui_port: u16) -> Result<()> {
    let t0 = std::time::Instant::now();
    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/health")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    let rtt = t0.elapsed();
    if code != 200 {
        anyhow::bail!(
            "node health check failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let rep: tetherly_node::HealthReport = serde_json::from_slice(&resp)?;
        println!("Node Health Status: {} (200 OK)", rep.status);
        println!("  Version:          {}", rep.version);
        println!("  Device ID:        {}", rep.device_id);
        println!("  Node Name:        {} ({})", rep.name, rep.platform);
        println!("  PID:              {}", rep.pid);
        println!(
            "  Uptime:           {} ({}s)",
            format_uptime(rep.uptime_secs),
            rep.uptime_secs
        );
        println!("  Bound Ports:");
        println!("    Control (Mesh): {}", rep.ports.control);
        println!("    File Transfer:  {}", rep.ports.file);
        println!("    Input (Remote): {}", rep.ports.input);
        println!("    Screen (Video): {}", rep.ports.screen);
        println!("    UI (Loopback):  {}", rep.ports.ui);
        println!("  Live Peers:       {}", rep.live_peers);
        println!("  Trusted Peers:    {}", rep.trusted_peers);
        println!("  Loopback Latency: {:.2}ms", rtt.as_secs_f64() * 1000.0);
    }
    Ok(())
}

/// `tetherly config` -- Config arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn config(
    name: Option<String>,
    advertise: Option<bool>,
    reconnect: Option<bool>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let has_updates = name.is_some() || advertise.is_some() || reconnect.is_some();
    let (code, resp) = if has_updates {
        let body = serde_json::json!({
            "name": name,
            "advertise": advertise,
            "reconnect": reconnect,
        });
        let bytes = serde_json::to_vec(&body)?;
        tetherly_node::loopback_post(ui_port, "/api/config", &bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?
    } else {
        tetherly_node::loopback_get(ui_port, "/api/config")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?
    };
    if code != 200 {
        anyhow::bail!(
            "node config query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let rep: tetherly_node::NodeConfigReport = serde_json::from_slice(&resp)?;
        if has_updates {
            println!("Tetherly Node Configuration (Updated):");
        } else {
            println!("Tetherly Node Configuration:");
        }
        println!("  Device ID:        {}", rep.device_id);
        println!("  Node Name:        {} ({})", rep.name, rep.platform);
        println!("  Data Directory:   {}", rep.data_dir);
        println!(
            "  Loopback Only:    {}",
            if rep.loopback_only {
                "true (127.0.0.1 only)"
            } else {
                "false (LAN + Loopback)"
            }
        );
        println!(
            "  mDNS Advertise:   {}",
            if rep.advertise { "enabled" } else { "disabled" }
        );
        println!(
            "  Auto Reconnect:   {}",
            if rep.reconnect { "enabled" } else { "disabled" }
        );
        println!("  Memory Clip:      {}", rep.memory_clip);
        println!(
            "  Overlay Sidecar:  {}",
            if rep.overlay_enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        println!(
            "  ANCS Service:     {}",
            if rep.ancs_attached {
                format!("attached ({})", rep.ancs_peripheral)
            } else {
                format!("detached ({})", rep.ancs_peripheral)
            }
        );
        println!("  Bound Ports:");
        println!("    Control (Mesh): {}", rep.control_port);
        println!("    File Transfer:  {}", rep.file_port);
        println!("    Input (Remote): {}", rep.input_port);
        println!("    Screen (Video): {}", rep.screen_port);
        println!("    UI (Loopback):  {}", rep.ui_port);
    }
    Ok(())
}

/// `tetherly stop` -- Stop arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn stop(ui_port: u16) -> Result<()> {
    let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/daemon/shutdown", b"")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node shutdown failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    println!("Node daemon shutdown initiated (HTTP 200).");
    Ok(())
}

/// `tetherly events` -- Events arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn events(
    count: usize,
    timeout_secs: u64,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut sse = tetherly_node::loopback_events_connect(ui_port)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if !json {
        println!("Listening for real-time node events on 127.0.0.1:{ui_port}...");
    }
    let deadline = if timeout_secs > 0 {
        Some(std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs))
    } else {
        None
    };
    let mut received = 0usize;
    loop {
        let next_res = if let Some(dl) = deadline {
            let now = std::time::Instant::now();
            if now >= dl {
                break;
            }
            match tokio::time::timeout(dl - now, sse.next_event()).await {
                Ok(res) => res,
                Err(_) => break,
            }
        } else {
            sse.next_event().await
        };

        match next_res {
            Ok(Some(raw)) => {
                received += 1;
                if json {
                    println!("{raw}");
                } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                    print_event(&val);
                } else {
                    println!("{raw}");
                }
                if count > 0 && received >= count {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                anyhow::bail!("event stream error: {e}");
            }
        }
    }
    Ok(())
}

/// `tetherly metrics` -- Metrics arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn metrics(reset: bool, json: bool, ui_port: u16) -> Result<()> {
    handle_traffic_cmd(reset, json, ui_port).await?;
    Ok(())
}

/// `tetherly caps` -- Caps arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn caps(
    peer: Option<String>,
    enable: Option<String>,
    disable: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    handle_caps_cmd(peer, enable, disable, json, ui_port).await?;
    Ok(())
}

pub(crate) async fn handle_traffic_cmd(
    reset: bool,
    json: bool,
    ui_port: u16,
) -> anyhow::Result<()> {
    if reset {
        let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/metrics/reset", b"{}")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "traffic telemetry reset failed (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
        } else {
            println!("Traffic and protocol telemetry counters successfully reset to zero.");
        }
        return Ok(());
    }

    let (code, resp) = tetherly_node::loopback_get(ui_port, "/api/metrics")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "traffic telemetry query failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let uptime_secs = v["uptime_secs"].as_u64().unwrap_or(0);
    let total_tx_bytes = v["total_tx_bytes"].as_u64().unwrap_or(0);
    let total_rx_bytes = v["total_rx_bytes"].as_u64().unwrap_or(0);
    let total_tx_frames = v["total_tx_frames"].as_u64().unwrap_or(0);
    let total_rx_frames = v["total_rx_frames"].as_u64().unwrap_or(0);

    let ctrl = &v["control"];
    let file = &v["file"];
    let input = &v["input"];
    let screen = &v["screen"];
    let msgs = &v["messages"];

    println!(
        "Network Traffic & Protocol Telemetry (Uptime: {})",
        format_uptime(uptime_secs)
    );
    println!("  Total Throughput:");
    println!(
        "    Outbound (TX): {:<14} ({} frames)",
        format_bytes(total_tx_bytes),
        total_tx_frames
    );
    println!(
        "    Inbound  (RX): {:<14} ({} frames)",
        format_bytes(total_rx_bytes),
        total_rx_frames
    );

    println!("\n  Multi-Channel Bandwidth Distribution:");
    println!(
        "    {:<18} {:<16} {:<16} {:<12} {:<12}",
        "Channel", "TX Bytes", "RX Bytes", "TX Frames", "RX Frames"
    );
    println!("    {}", "-".repeat(76));
    for (name, ch) in [
        ("Control (Noise)", ctrl),
        ("File Transfer", file),
        ("Input Stream", input),
        ("Screen Capture", screen),
    ] {
        let tx_b = ch["tx_bytes"].as_u64().unwrap_or(0);
        let rx_b = ch["rx_bytes"].as_u64().unwrap_or(0);
        let tx_f = ch["tx_frames"].as_u64().unwrap_or(0);
        let rx_f = ch["rx_frames"].as_u64().unwrap_or(0);
        println!(
            "    {:<18} {:<16} {:<16} {:<12} {:<12}",
            name,
            format_bytes(tx_b),
            format_bytes(rx_b),
            tx_f,
            rx_f
        );
    }

    println!("\n  Core Protocol Message Distribution:");
    println!(
        "    {:<18} {:<12} {:<12}",
        "Message Type", "TX Count", "RX Count"
    );
    println!("    {}", "-".repeat(44));
    for (name, tx_key, rx_key) in [
        ("Ping", "ping_tx", "ping_rx"),
        ("Pong", "pong_tx", "pong_rx"),
        ("Notify", "notify_tx", "notify_rx"),
        ("Clipboard", "clip_tx", "clip_rx"),
        ("File Offer", "file_offer_tx", "file_offer_rx"),
    ] {
        let tx_c = msgs[tx_key].as_u64().unwrap_or(0);
        let rx_c = msgs[rx_key].as_u64().unwrap_or(0);
        println!("    {:<18} {:<12} {:<12}", name, tx_c, rx_c);
    }

    Ok(())
}

pub(crate) async fn handle_caps_cmd(
    peer: Option<String>,
    enable: Option<String>,
    disable: Option<String>,
    json: bool,
    ui_port: u16,
) -> anyhow::Result<()> {
    if enable.is_some() || disable.is_some() {
        let mut enables = Vec::new();
        if let Some(e) = enable {
            enables.push(e);
        }
        let mut disables = Vec::new();
        if let Some(d) = disable {
            disables.push(d);
        }
        let req_body = serde_json::json!({
            "enable": enables,
            "disable": disables,
        });
        let (code, resp) = tetherly_node::loopback_post(
            ui_port,
            "/api/caps",
            serde_json::to_vec(&req_body).unwrap_or_default().as_slice(),
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
        if code != 200 {
            anyhow::bail!(
                "node returned HTTP {code}: {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
        let rep: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let dev = rep["device_id"].as_str().unwrap_or("-");
        let feats = rep["features"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        let fp = rep["file_port"].as_u64().unwrap_or(0);
        let ip = rep["input_port"].as_u64().unwrap_or(0);
        let sp = rep["screen_port"].as_u64().unwrap_or(0);

        println!("Local node capabilities updated:");
        println!("  Node ID:     {dev}");
        println!("  Features:    [{feats}]");
        println!("  Ports:       file: {fp}, input: {ip}, screen: {sp}");
        return Ok(());
    }

    if let Some(ref target) = peer {
        let path = format!("/api/peers/caps?peer={target}");
        let (code, resp) = tetherly_node::loopback_get(ui_port, &path)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            anyhow::bail!(
                "node returned HTTP {code}: {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
        let rep: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!("Peer Capabilities ({target}):");
        if let Some(arr) = rep["peers"].as_array() {
            if arr.is_empty() {
                println!("  (peer not found)");
            } else {
                for p in arr {
                    let dev = p["device_id"].as_str().unwrap_or("-");
                    let name = p["name"].as_str().unwrap_or("-");
                    let alias = p["alias"].as_str();
                    let alias_str = match alias {
                        Some(a) if !a.is_empty() && a != name => format!(" (alias: \"{a}\")"),
                        _ => String::new(),
                    };
                    let online = p["online"].as_bool().unwrap_or(false);
                    let online_str = if online { "ONLINE" } else { "OFFLINE" };
                    let feats = p["features"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    let fp = p["file_port"]
                        .as_u64()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "n/a".into());
                    let ip = p["input_port"]
                        .as_u64()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "n/a".into());
                    let sp = p["screen_port"]
                        .as_u64()
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "n/a".into());

                    println!("  * [{online_str}] {name}{alias_str} · {dev}");
                    println!("    Features: [{feats}]");
                    println!("    Ports:    file: {fp}, input: {ip}, screen: {sp}");
                }
            }
        }
        return Ok(());
    }

    // Both local and peers
    let (code_local, resp_local) = tetherly_node::loopback_get(ui_port, "/api/caps")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    let (code_peers, resp_peers) = tetherly_node::loopback_get(ui_port, "/api/peers/caps")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code_local != 200 {
        anyhow::bail!(
            "node returned HTTP {code_local}: {}",
            String::from_utf8_lossy(&resp_local)
        );
    }
    if code_peers != 200 {
        anyhow::bail!(
            "node returned HTTP {code_peers}: {}",
            String::from_utf8_lossy(&resp_peers)
        );
    }
    if json {
        let local_v: serde_json::Value = serde_json::from_slice(&resp_local)
            .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        let peers_v: serde_json::Value = serde_json::from_slice(&resp_peers)
            .map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
        println!(
            "{}",
            serde_json::json!({
                "local": local_v,
                "peers": peers_v,
            })
        );
        return Ok(());
    }

    let local_v: serde_json::Value =
        serde_json::from_slice(&resp_local).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
    let peers_v: serde_json::Value =
        serde_json::from_slice(&resp_peers).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;

    let self_id = local_v["device_id"].as_str().unwrap_or("-");
    let local_feats = local_v["features"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let fp = local_v["file_port"].as_u64().unwrap_or(0);
    let ip = local_v["input_port"].as_u64().unwrap_or(0);
    let sp = local_v["screen_port"].as_u64().unwrap_or(0);

    println!("Local Node Capabilities:");
    println!("  Node ID:     {self_id}");
    println!("  Features:    [{local_feats}]");
    println!("  Ports:       file: {fp}, input: {ip}, screen: {sp}");

    let count = peers_v["count"].as_u64().unwrap_or(0);
    println!("\nConnected Peers Capabilities ({count}):");
    if let Some(arr) = peers_v["peers"].as_array() {
        if arr.is_empty() {
            println!("  (no live peers connected)");
        } else {
            for p in arr {
                let dev = p["device_id"].as_str().unwrap_or("-");
                let name = p["name"].as_str().unwrap_or("-");
                let alias = p["alias"].as_str();
                let alias_str = match alias {
                    Some(a) if !a.is_empty() && a != name => format!(" (alias: \"{a}\")"),
                    _ => String::new(),
                };
                let feats = p["features"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let p_fp = p["file_port"]
                    .as_u64()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".into());
                let p_ip = p["input_port"]
                    .as_u64()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".into());
                let p_sp = p["screen_port"]
                    .as_u64()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "n/a".into());

                println!("  * {name}{alias_str} · {dev}");
                println!("    Features: [{feats}]");
                println!("    Ports:    file: {p_fp}, input: {p_ip}, screen: {p_sp}");
            }
        }
    }
    Ok(())
}

pub(crate) fn print_event(v: &serde_json::Value) {
    let kind = v["kind"].as_str().unwrap_or("unknown");
    match kind {
        "status" => {
            let msg = v["message"].as_str().unwrap_or("");
            println!("[status] {msg}");
        }
        "peer_up" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let name = v["name"].as_str().unwrap_or("?");
            let platform = v["platform"].as_str().unwrap_or("?");
            println!("[peer_up] {name} ({dev}) [{platform}]");
        }
        "peer_down" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            println!("[peer_down] {dev}");
        }
        "file_offered" => {
            let tid = v["transfer_id"].as_str().unwrap_or("?");
            let src = v["source"].as_str().unwrap_or("?");
            let files_count = v["files"].as_array().map(|a| a.len()).unwrap_or(0);
            println!("[file_offered] transfer={tid} from={src} ({files_count} files)");
        }
        "file_done" => {
            let tid = v["transfer_id"].as_str().unwrap_or("?");
            println!("[file_done] transfer={tid}");
        }
        "file_rejected" => {
            let tid = v["transfer_id"].as_str().unwrap_or("?");
            let src = v["source"].as_str().unwrap_or("?");
            let reason = v["reason"]
                .as_str()
                .map(|r| format!(" reason=\"{r}\""))
                .unwrap_or_default();
            println!("[file_rejected] transfer={tid} peer={src}{reason}");
        }
        "file_auto_accept_changed" => {
            let enabled = v["enabled"].as_bool().unwrap_or(false);
            println!("[file_auto_accept_changed] enabled={enabled}");
        }
        "inbox_file_deleted" => {
            let name = v["name"].as_str().unwrap_or("?");
            let freed = v["freed_bytes"].as_u64().unwrap_or(0);
            println!(
                "[inbox_file_deleted] name={name} freed={}",
                format_bytes(freed)
            );
        }
        "inbox_cleared" => {
            let count = v["deleted_count"].as_u64().unwrap_or(0);
            let freed = v["freed_bytes"].as_u64().unwrap_or(0);
            println!(
                "[inbox_cleared] deleted={count} freed={}",
                format_bytes(freed)
            );
        }
        "inbox_file_opened" => {
            let name = v["name"].as_str();
            match name {
                Some(n) => println!("[inbox_file_opened] name={n}"),
                None => println!("[inbox_file_opened] directory"),
            }
        }
        "notification_banner_sent" => {
            let title = v["title"].as_str().unwrap_or("?");
            let app = v["app_name"].as_str().unwrap_or("-");
            let is_otp = v["is_otp"].as_bool().unwrap_or(false);
            println!("[notification_banner_sent] title=\"{title}\" app={app} is_otp={is_otp}");
        }
        "notify_config_changed" => {
            let banners = v["banners_enabled"].as_bool().unwrap_or(true);
            let dnd = v["dnd_mode"].as_str().unwrap_or("off");
            let sound = v["sound_enabled"].as_bool().unwrap_or(true);
            let mask_otp = v["mask_otp"].as_bool().unwrap_or(false);
            let allow_otp_dnd = v["allow_otp_in_dnd"].as_bool().unwrap_or(true);
            println!(
                "[notify_config_changed] banners={banners} dnd={dnd} sound={sound} mask_otp={mask_otp} allow_otp_in_dnd={allow_otp_dnd}"
            );
        }
        "pairing_needed" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let fp = v["fp"].as_str().unwrap_or("?");
            println!("[pairing_needed] peer={dev} fp={fp}");
        }
        "candidate" => {
            let c = &v["candidate"];
            let id = c["id"].as_str().unwrap_or("?");
            let app = c["app_name"]
                .as_str()
                .unwrap_or(c["app_id"].as_str().unwrap_or("?"));
            let title = c["title"].as_str().unwrap_or("");
            println!("[candidate] id={id} app={app} title={title}");
        }
        "candidate_expired" => {
            let id = v["id"].as_str().unwrap_or("?");
            println!("[candidate_expired] id={id}");
        }
        "candidate_dismissed" => {
            let id = v["id"].as_str().unwrap_or("?");
            let source = v["source"].as_str().unwrap_or("?");
            let app = v["app_id"].as_str().unwrap_or("?");
            println!("[candidate_dismissed] id={id} peer={source} app={app}");
        }
        "reply_received" => {
            let r = &v["reply"];
            let id = r["id"].as_str().unwrap_or("?");
            let app = r["app_id"].as_str().unwrap_or("?");
            println!("[reply_received] id={id} app={app}");
        }
        "overlay" => {
            let present = v["present"].as_bool().unwrap_or(false);
            let lan_only = v["lan_only"].as_bool().unwrap_or(false);
            println!("[overlay] present={present} lan_only={lan_only}");
        }
        "metrics_reset" => {
            let ts = v["timestamp_ms"].as_u64().unwrap_or(0);
            println!("[metrics_reset] timestamp_ms={ts}");
        }
        "peer_caps_changed" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let feats = v["features"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("[peer_caps_changed] peer={dev} features=[{feats}]");
        }
        "peer_blocked" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let reason = v["reason"]
                .as_str()
                .map(|r| format!(" reason=\"{r}\""))
                .unwrap_or_default();
            println!("[peer_blocked] peer={dev}{reason}");
        }
        "peer_unblocked" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            println!("[peer_unblocked] peer={dev}");
        }
        "clipboard_synced" => {
            let seq = v["seq"].as_u64().unwrap_or(0);
            let dir = v["direction"].as_str().unwrap_or("?");
            let src = v["source"].as_str().unwrap_or("local");
            let bytes = v["bytes"].as_u64().unwrap_or(0);
            let hash = v["hash"].as_str().unwrap_or("");
            let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
            let peers = v["sent_peers"].as_u64().unwrap_or(0);
            if dir == "outbound" {
                println!(
                    "[clipboard_synced] seq={seq} outbound from={src} ({bytes} bytes, sha256:{short_hash}...) to {peers} peer(s)"
                );
            } else {
                println!(
                    "[clipboard_synced] seq={seq} inbound from={src} ({bytes} bytes, sha256:{short_hash}...)"
                );
            }
        }
        "clipboard_cleared" => {
            let ts = v["timestamp_ms"].as_u64().unwrap_or(0);
            println!("[clipboard_cleared] timestamp_ms={ts}");
        }
        "screen_state_changed" => {
            let state = v["state"].as_str().unwrap_or("unknown");
            let allowed = v["allowed"].as_bool().unwrap_or(false);
            let peer = v["peer"]
                .as_str()
                .map(|p| format!(" peer={p}"))
                .unwrap_or_default();
            println!("[screen_state_changed] state={state} allowed={allowed}{peer}");
        }
        "input_seat_changed" => {
            let seat = v["seat"].as_str().unwrap_or("unknown");
            let open = v["channel_open"].as_bool().unwrap_or(false);
            let peer = v["peer"]
                .as_str()
                .map(|p| format!(" peer={p}"))
                .unwrap_or_default();
            println!("[input_seat_changed] seat={seat} channel_open={open}{peer}");
        }
        "network_refreshed" => {
            let iface_count = v["iface_count"].as_u64().unwrap_or(0);
            let listen_count = v["listen_count"].as_u64().unwrap_or(0);
            let overlay_present = v["overlay_present"].as_bool().unwrap_or(false);
            let overlay_source = v["overlay_source"].as_str().unwrap_or("Absent");
            println!(
                "[network_refreshed] scanned={iface_count} listening={listen_count} overlay={overlay_present} ({overlay_source})"
            );
        }
        "overlay_state_changed" => {
            let present = v["present"].as_bool().unwrap_or(false);
            let source = v["source"].as_str().unwrap_or("Absent");
            let peer_count = v["peer_count"].as_u64().unwrap_or(0);
            let local_ips = v["local_ips"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!(
                "[overlay_state_changed] present={present} source={source} peers={peer_count} local_ips=[{local_ips}]"
            );
        }
        "ancs_state_changed" => {
            let state = v["state"].as_str().unwrap_or("unknown");
            let attached = v["attached"].as_bool().unwrap_or(false);
            let retry = v["retry_at_ms"]
                .as_u64()
                .map(|r| format!(" retry_at_ms={r}"))
                .unwrap_or_default();
            println!("[ancs_state_changed] state={state} attached={attached}{retry}");
        }
        "ancs_rule_changed" => {
            let action = v["action"].as_str().unwrap_or("unknown");
            let app_id = v["app_id"]
                .as_str()
                .map(|a| format!(" app_id={a}"))
                .unwrap_or_default();
            let url = v["url"]
                .as_str()
                .map(|u| format!(" url={u}"))
                .unwrap_or_default();
            let rules_count = v["rules_count"].as_u64().unwrap_or(0);
            println!("[ancs_rule_changed] action={action}{app_id}{url} rules_count={rules_count}");
        }
        "peer_discovered" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let name = v["name"].as_str().unwrap_or("?");
            let addr = v["addr"].as_str().unwrap_or("?");
            let caps = v["caps"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("[peer_discovered] {name} ({dev}) at {addr} caps=[{caps}]");
        }
        "peer_expired" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            println!("[peer_expired] peer={dev}");
        }
        "system_metrics_refreshed" => {
            let pid = v["pid"].as_u64().unwrap_or(0);
            let rss = v["rss_bytes"].as_u64().unwrap_or(0);
            let virt = v["virt_bytes"].as_u64().unwrap_or(0);
            let threads = v["thread_count"].as_u64().unwrap_or(0);
            let fds = v["open_fds"]
                .as_u64()
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".into());
            let cpu_ms = v["cpu_time_ms"].as_u64().unwrap_or(0);
            println!(
                "[system_metrics_refreshed] pid={pid} rss={} virt={} threads={threads} fds={fds} cpu_time={cpu_ms}ms",
                format_bytes(rss),
                format_bytes(virt)
            );
        }
        "peer_link_quality_refreshed" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let rtt = v["rtt_ms"].as_u64().unwrap_or(0);
            let avg = v["avg_rtt_ms"]
                .as_f64()
                .map(|a| format!("{a:.1}ms"))
                .unwrap_or_else(|| "-".into());
            let jitter = v["jitter_ms"]
                .as_f64()
                .map(|j| format!("{j:.1}ms"))
                .unwrap_or_else(|| "-".into());
            let loss = v["packet_loss_rate"].as_f64().unwrap_or(0.0);
            let grade = v["grade"].as_str().unwrap_or("unknown");
            println!(
                "[peer_link_quality_refreshed] peer={dev} rtt={rtt}ms avg={avg} jitter={jitter} loss={loss:.1}% grade={grade}"
            );
        }
        "heartbeat_config_changed" => {
            let interval = v["interval_secs"].as_u64().unwrap_or(0);
            let deadline = v["deadline_secs"].as_u64().unwrap_or(0);
            println!("[heartbeat_config_changed] interval={interval}s deadline={deadline}s");
        }
        "audit_event_recorded" => {
            let id = v["id"].as_u64().unwrap_or(0);
            let cat = v["category"].as_str().unwrap_or("general");
            let sev = v["severity"].as_str().unwrap_or("info");
            let action = v["action"].as_str().unwrap_or("?");
            let target = v["target"]
                .as_str()
                .map(|t| format!(" target={t}"))
                .unwrap_or_default();
            let summary = v["summary"].as_str().unwrap_or("");
            let success = v["success"].as_bool().unwrap_or(true);
            let status = if success { "ok" } else { "failed" };
            println!("[audit] #{id} [{sev}] [{cat}] {action}{target} -> {summary} ({status})");
        }
        "message_received" => {
            let id = v["id"].as_str().unwrap_or("?");
            let peer = v["peer"].as_str().unwrap_or("?");
            let len = v["len"].as_u64().unwrap_or(0);
            println!("[message_received] id={id} from={peer} (length: {len} bytes)");
        }
        "message_sent" => {
            let id = v["id"].as_str().unwrap_or("?");
            let peer = v["peer"].as_str().unwrap_or("?");
            let len = v["len"].as_u64().unwrap_or(0);
            println!("[message_sent] id={id} to={peer} (length: {len} bytes)");
        }
        "outbox_updated" => {
            let id = v["id"].as_str().unwrap_or("?");
            let tid = v["transfer_id"].as_str().unwrap_or("?");
            let peer = v["peer"].as_str().unwrap_or("?");
            let fname = v["filename"].as_str().unwrap_or("?");
            let sz = v["size"].as_u64().unwrap_or(0);
            let status = v["status"].as_str().unwrap_or("?");
            println!(
                "[outbox_updated] id={id} transfer={tid} peer={peer} file=\"{fname}\" ({}) status={status}",
                format_bytes(sz)
            );
        }
        "outbox_cleared" => {
            let count = v["count"].as_u64().unwrap_or(0);
            let peer = v["peer"]
                .as_str()
                .map(|p| format!(" peer={p}"))
                .unwrap_or_default();
            let status = v["status"]
                .as_str()
                .map(|s| format!(" status={s}"))
                .unwrap_or_default();
            println!("[outbox_cleared] purged={count} entries{peer}{status}");
        }
        "peer_profile_updated" => {
            let dev = v["device_id"].as_str().unwrap_or("?");
            let dev_type = v["device_type"].as_str().unwrap_or("other");
            let grp = v["group"].as_str().unwrap_or("<none>");
            let tags = v["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!("[peer_profile_updated] peer={dev} type={dev_type} group={grp} tags=[{tags}]");
        }
        "multi_dispatch_completed" => {
            let kind = v["kind"].as_str().unwrap_or("?");
            let selector = v["selector"].as_str().unwrap_or("?");
            let total = v["target_count"].as_u64().unwrap_or(0);
            let success = v["success_count"].as_u64().unwrap_or(0);
            let skipped = v["skipped_offline_count"].as_u64().unwrap_or(0);
            let failed = v["failed_count"].as_u64().unwrap_or(0);
            println!(
                "[multi_dispatch_completed] kind={kind} selector={selector} targets={total} (success={success}, skipped={skipped}, failed={failed})"
            );
        }
        "auto_accept_policy_changed" => {
            let enabled = v["enabled"].as_bool().unwrap_or(false);
            let grps = v["allowed_groups"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            let tags = v["allowed_tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            println!(
                "[auto_accept_policy_changed] enabled={enabled} groups=[{grps}] tags=[{tags}]"
            );
        }
        other => {
            println!("[{other}] {v}");
        }
    }
}

pub(crate) fn print_status_summary(v: &serde_json::Value) {
    let device_id = v.get("device_id").and_then(|s| s.as_str()).unwrap_or("?");
    let name = v.get("name").and_then(|s| s.as_str()).unwrap_or("?");
    let platform = v.get("platform").and_then(|s| s.as_str()).unwrap_or("?");
    let pin_active = v
        .get("pin_active")
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    let lan_only = v.get("lan_only").and_then(|s| s.as_bool()).unwrap_or(false);

    println!("Tetherly Node Status:");
    println!("  Device ID:  {device_id}");
    println!("  Node Name:  {name} ({platform})");
    println!("  PIN Active: {pin_active}");
    println!("  LAN Only:   {lan_only}");

    if let Some(peers) = v.get("peers").and_then(|p| p.as_array()) {
        println!("  Connected Peers ({}):", peers.len());
        for p in peers {
            let id = p.get("device_id").and_then(|s| s.as_str()).unwrap_or("?");
            let name = p.get("name").and_then(|s| s.as_str()).unwrap_or("?");
            let addr = p.get("addr").and_then(|s| s.as_str()).unwrap_or("?");
            let path = p.get("path").and_then(|s| s.as_str()).unwrap_or("?");
            println!("    - {id} ({name}) at {addr} [{path}]");
        }
    }

    if let Some(transfers) = v.get("transfers").and_then(|t| t.as_array()) {
        println!("  Transfers ({}):", transfers.len());
        for t in transfers {
            let id = t.get("transfer_id").and_then(|s| s.as_str()).unwrap_or("?");
            let dir = t.get("direction").and_then(|s| s.as_str()).unwrap_or("?");
            let filename = t.get("filename").and_then(|s| s.as_str()).unwrap_or("?");
            let transferred = t.get("transferred").and_then(|s| s.as_u64()).unwrap_or(0);
            let total = t.get("total").and_then(|s| s.as_u64()).unwrap_or(0);
            let speed = t.get("speed_bps").and_then(|s| s.as_u64()).unwrap_or(0);
            let phase = t.get("phase").and_then(|s| s.as_str()).unwrap_or("?");
            let pct = if total > 0 {
                (transferred as f64 / total as f64) * 100.0
            } else {
                0.0
            };
            let speed_str = if speed > 0 {
                format!("{} ", format_speed(speed))
            } else {
                "".to_string()
            };
            println!(
                "    - [{dir}] {filename} ({:>5.1}%, {} / {}) {speed_str}[{phase}] ({id})",
                pct,
                format_bytes(transferred),
                format_bytes(total)
            );
        }
    }

    if let Some(stats) = v.get("transfer_stats") {
        let sent = stats
            .get("bytes_sent")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let recv = stats
            .get("bytes_received")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let completed = stats
            .get("completed_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let failed = stats
            .get("failed_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        println!(
            "  Transfer Metrics: sent: {}, received: {}, completed: {}, failed: {}",
            format_bytes(sent),
            format_bytes(recv),
            completed,
            failed
        );
    }
}

pub(crate) fn print_system_report(v: &serde_json::Value) {
    let node_name = v["node_name"].as_str().unwrap_or("?");
    let device_id = v["device_id"].as_str().unwrap_or("?");
    let proc = &v["process"];
    let host = &v["host"];

    let pid = proc["pid"].as_u64().unwrap_or(0);
    let uptime_secs = proc["uptime_secs"].as_u64().unwrap_or(0);
    let rss = proc["rss_bytes"].as_u64().unwrap_or(0);
    let virt = proc["virt_bytes"].as_u64().unwrap_or(0);
    let threads = proc["thread_count"].as_u64().unwrap_or(0);
    let fds = proc["open_fds"]
        .as_u64()
        .map(|n| n.to_string())
        .unwrap_or_else(|| "N/A".to_string());
    let cpu_time_ms = proc["cpu_time_ms"].as_u64().unwrap_or(0);

    let os = host["os"].as_str().unwrap_or("?");
    let arch = host["arch"].as_str().unwrap_or("?");
    let num_cpus = host["num_cpus"].as_u64().unwrap_or(1);
    let total_mem = host["total_memory_bytes"].as_u64();

    println!("Tetherly Node System Telemetry & Performance Profile:");
    println!("  Node Name:        {node_name}");
    println!("  Device ID:        {device_id}");
    println!("  Host Platform:    {os} ({arch}), {num_cpus} CPU cores");
    if let Some(tm) = total_mem {
        println!("  Host Memory:      {}", format_bytes(tm));
    }
    println!("  Daemon PID:       {pid}");
    println!(
        "  Daemon Uptime:    {} ({uptime_secs}s)",
        format_uptime(uptime_secs)
    );
    println!("  Active Threads:   {threads}");
    println!("  Open Descriptors: {fds}");
    println!("  Resident Memory:  {} ({rss} B)", format_bytes(rss));
    println!("  Virtual Memory:   {} ({virt} B)", format_bytes(virt));
    println!(
        "  CPU Runtime:      {cpu_time_ms}ms ({:.2}s)",
        cpu_time_ms as f64 / 1000.0
    );
}
