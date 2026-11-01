// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `run`/`identity`/`pin`/`dial` plus node status, metrics, events and caps.

use super::util::{
    format_bytes, format_speed, format_uptime, loopback_events_connect, loopback_get,
    loopback_post, now_ms,
};
use super::*;

/// `tetherly-cli identity` -- Identity arm of the top-level dispatch.
pub(crate) async fn identity() -> Result<()> {
    let id = Identity::generate().context("identity")?;
    println!("{}", id.device_id());
    Ok(())
}

/// `tetherly-cli pin` -- Pin arm of the top-level dispatch.
pub(crate) async fn pin(status: bool, clear: bool, json: bool, ui_port: u16) -> Result<()> {
    if clear {
        let (code, resp) = loopback_post(ui_port, "/api/pin/clear", b"{}")
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
        let (code, resp) = loopback_get(ui_port, "/api/pin").await.map_err(|e| {
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
        let (code, resp) = loopback_post(ui_port, "/api/pin", b"{}")
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

/// `tetherly-cli listen` -- Listen arm of the top-level dispatch.
pub(crate) async fn listen(port: u16, pin: String, name: String) -> Result<()> {
    let pin = parse_pin(&pin)?;
    let identity = Identity::generate()?;
    info!(device_id = %identity.device_id(), port, "listen");
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let (stream, addr) = listener.accept().await?;
    let cfg = cfg(identity, &name, pin);
    let mut sess = accept_session(stream, addr, &cfg).await?;
    info!(peer = %sess.peer_id, "paired; waiting for ping");
    serve_pong_once(&mut sess).await?;
    Ok(())
}

/// `tetherly-cli dial` -- Dial arm of the top-level dispatch.
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
    let (code, resp) = loopback_post(ui_port, "/api/dial", &body)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        anyhow::bail!(
            "node refused dial request (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    let target_desc = addr
        .map(|a| a.to_string())
        .or_else(|| peer.clone())
        .unwrap_or_default();
    println!("dial dispatched to {target_desc}");
    Ok(())
}

/// `tetherly-cli discover` -- Discover arm of the top-level dispatch.
pub(crate) async fn discover(rescan: bool, json: bool, ui_port: u16) -> Result<()> {
    if rescan {
        let (code, resp) = loopback_post(ui_port, "/api/discovery/rescan", b"{}")
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
    let (code, resp) = loopback_get(ui_port, "/api/discovery").await.map_err(|e| {
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
    let rep: serde_json::Value = serde_json::from_slice(&resp)?;
    let count = rep["count"].as_u64().unwrap_or(0);
    println!("Discovered LAN Peers ({count}):");
    let peers = rep["peers"].as_array();
    if peers.map(|p| p.is_empty()).unwrap_or(true) {
        println!("  (no peers discovered on local network)");
    } else if let Some(peers) = peers {
        for p in peers {
            let name = p["name"].as_str().unwrap_or("unknown");
            let dev_id = p["device_id"].as_str().unwrap_or("unknown");
            let trusted_mark = if p["is_trusted"].as_bool().unwrap_or(false) {
                "[trusted]"
            } else {
                "[unpaired]"
            };
            let online_mark = if p["is_online"].as_bool().unwrap_or(false) {
                "[online]"
            } else {
                "[idle]"
            };
            let port = p["port"].as_u64().unwrap_or(0);
            let fp_str = p["fp"].as_str().unwrap_or("none");
            let caps_str = p["caps"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let addrs_str = p["addrs"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            println!(
                "  * {} ({}) {} {} - port: {}, fp: {}, caps: [{}], addrs: [{}]",
                name, dev_id, trusted_mark, online_mark, port, fp_str, caps_str, addrs_str
            );
        }
    }
    Ok(())
}

/// `tetherly-cli status` -- Status arm of the top-level dispatch.
pub(crate) async fn status(watch: bool, interval: u64, json: bool, ui_port: u16) -> Result<()> {
    loop {
        let (code, resp) = loopback_get(ui_port, "/api/state").await.map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
        if code != 200 {
            bail!(
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

/// `tetherly-cli health` -- Health arm of the top-level dispatch.
pub(crate) async fn health(json: bool, ui_port: u16) -> Result<()> {
    let t0 = std::time::Instant::now();
    let (code, resp) = loopback_get(ui_port, "/api/health").await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    let rtt = t0.elapsed();
    if code != 200 {
        bail!(
            "node health check failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        let status = v["status"].as_str().unwrap_or("unknown");
        let version = v["version"].as_str().unwrap_or("?");
        let device_id = v["device_id"].as_str().unwrap_or("?");
        let name = v["name"].as_str().unwrap_or("?");
        let platform = v["platform"].as_str().unwrap_or("?");
        let pid = v["pid"].as_u64().unwrap_or(0);
        let uptime_secs = v["uptime_secs"].as_u64().unwrap_or(0);
        let live_peers = v["live_peers"].as_u64().unwrap_or(0);
        let trusted_peers = v["trusted_peers"].as_u64().unwrap_or(0);
        let ports = &v["ports"];
        let control_port = ports["control"].as_u64().unwrap_or(0);
        let file_port = ports["file"].as_u64().unwrap_or(0);
        let input_port = ports["input"].as_u64().unwrap_or(0);
        let screen_port = ports["screen"].as_u64().unwrap_or(0);
        let ui_port = ports["ui"].as_u64().unwrap_or(0);

        println!("Node Health Status: {status} (200 OK)");
        println!("  Version:          {version}");
        println!("  Device ID:        {device_id}");
        println!("  Node Name:        {name} ({platform})");
        println!("  PID:              {pid}");
        println!(
            "  Uptime:           {} ({uptime_secs}s)",
            format_uptime(uptime_secs)
        );
        println!("  Bound Ports:");
        println!("    Control (Mesh): {control_port}");
        println!("    File Transfer:  {file_port}");
        println!("    Input (Remote): {input_port}");
        println!("    Screen (Video): {screen_port}");
        println!("    UI (Loopback):  {ui_port}");
        println!("  Live Peers:       {live_peers}");
        println!("  Trusted Peers:    {trusted_peers}");
        println!("  Loopback Latency: {:.2}ms", rtt.as_secs_f64() * 1000.0);
    }
    Ok(())
}

/// `tetherly-cli sys` -- Sys arm of the top-level dispatch.
pub(crate) async fn sys(watch: bool, interval: u64, json: bool, ui_port: u16) -> Result<()> {
    loop {
        let (code, resp) = loopback_get(ui_port, "/api/sys").await.map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
        if code != 200 {
            bail!(
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

/// `tetherly-cli config` -- Config arm of the top-level dispatch.
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
        loopback_post(ui_port, "/api/config", &bytes)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?
    } else {
        loopback_get(ui_port, "/api/config").await.map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?
    };
    if code != 200 {
        bail!(
            "node config query/update failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
    } else {
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        if has_updates {
            println!("Tetherly Node Configuration (Updated):");
        } else {
            println!("Tetherly Node Configuration:");
        }
        let device_id = v["device_id"].as_str().unwrap_or("?");
        let name = v["name"].as_str().unwrap_or("?");
        let platform = v["platform"].as_str().unwrap_or("?");
        let data_dir = v["data_dir"].as_str().unwrap_or("?");
        let loopback_only = v["loopback_only"].as_bool().unwrap_or(false);
        let advertise = v["advertise"].as_bool().unwrap_or(false);
        let reconnect = v["reconnect"].as_bool().unwrap_or(false);
        let memory_clip = v["memory_clip"].as_bool().unwrap_or(false);
        let overlay_enabled = v["overlay_enabled"].as_bool().unwrap_or(false);
        let ancs_attached = v["ancs_attached"].as_bool().unwrap_or(false);
        let ancs_peripheral = v["ancs_peripheral"].as_str().unwrap_or("-");
        let control_port = v["control_port"].as_u64().unwrap_or(0);
        let file_port = v["file_port"].as_u64().unwrap_or(0);
        let input_port = v["input_port"].as_u64().unwrap_or(0);
        let screen_port = v["screen_port"].as_u64().unwrap_or(0);
        let ui_port_num = v["ui_port"].as_u64().unwrap_or(0);

        println!("  Device ID:        {device_id}");
        println!("  Node Name:        {name} ({platform})");
        println!("  Data Directory:   {data_dir}");
        println!(
            "  Loopback Only:    {}",
            if loopback_only {
                "true (127.0.0.1 only)"
            } else {
                "false (LAN + Loopback)"
            }
        );
        println!(
            "  mDNS Advertise:   {}",
            if advertise { "enabled" } else { "disabled" }
        );
        println!(
            "  Auto Reconnect:   {}",
            if reconnect { "enabled" } else { "disabled" }
        );
        println!("  Memory Clip:      {memory_clip}");
        println!(
            "  Overlay Sidecar:  {}",
            if overlay_enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
        println!(
            "  ANCS Service:     {}",
            if ancs_attached {
                format!("attached ({ancs_peripheral})")
            } else {
                format!("detached ({ancs_peripheral})")
            }
        );
        println!("  Bound Ports:");
        println!("    Control (Mesh): {control_port}");
        println!("    File Transfer:  {file_port}");
        println!("    Input (Remote): {input_port}");
        println!("    Screen (Video): {screen_port}");
        println!("    UI (Loopback):  {ui_port_num}");
    }
    Ok(())
}

/// `tetherly-cli stop` -- Stop arm of the top-level dispatch.
pub(crate) async fn stop(ui_port: u16) -> Result<()> {
    let (code, resp) = loopback_post(ui_port, "/api/daemon/shutdown", b"")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "node shutdown failed (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    println!("Node daemon shutdown initiated (HTTP 200).");
    Ok(())
}

/// `tetherly-cli events` -- Events arm of the top-level dispatch.
pub(crate) async fn events(
    count: usize,
    timeout_secs: u64,
    json: bool,
    ui_port: u16,
) -> Result<()> {
    let mut sse = loopback_events_connect(ui_port).await.map_err(|e| {
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
                bail!("event stream error: {e}");
            }
        }
    }
    Ok(())
}

/// `tetherly-cli metrics` -- Metrics arm of the top-level dispatch.
pub(crate) async fn metrics(reset: bool, json: bool, ui_port: u16) -> Result<()> {
    handle_traffic_cmd(reset, json, ui_port).await?;
    Ok(())
}

/// `tetherly-cli caps` -- Caps arm of the top-level dispatch.
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

pub(crate) fn cfg(identity: Identity, name: &str, pin: [u8; 8]) -> SessionConfig {
    SessionConfig {
        identity,
        name: name.to_string(),
        platform: std::env::consts::OS.to_string(),
        trust: Arc::new(Mutex::new(Default::default())),
        lockout: Arc::new(Mutex::new(LockoutTable::default())),
        tcp_limiter: Arc::new(Mutex::new(TcpLimiter::default())),
        pin: Some(pin),
        pin_created_ms: now_ms(),
        clock: Arc::new(SystemClock),
        handshake_timeout: HANDSHAKE_TIMEOUT,
        hello_override: None,
        resume_only: false,
        screen_only: false,
        caps: None,
    }
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
        "notification_banner_sent" => {
            let title = v["title"].as_str().unwrap_or("");
            let app = v["app_name"]
                .as_str()
                .map(|a| format!(" app={a}"))
                .unwrap_or_default();
            let is_otp = v["is_otp"].as_bool().unwrap_or(false);
            let otp_str = if is_otp { " [OTP]" } else { "" };
            println!("[notification_banner_sent] title=\"{title}\"{app}{otp_str}");
        }
        "notify_config_changed" => {
            let banners = v["banners_enabled"].as_bool().unwrap_or(false);
            let dnd = v["dnd_mode"].as_str().unwrap_or("off");
            let sound = v["sound_enabled"].as_bool().unwrap_or(true);
            let mask = v["mask_otp"].as_bool().unwrap_or(true);
            let allow_otp = v["allow_otp_in_dnd"].as_bool().unwrap_or(true);
            println!(
                "[notify_config_changed] banners={banners} dnd={dnd} sound={sound} mask_otp={mask} allow_otp_in_dnd={allow_otp}"
            );
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
            let grp = v["group"]
                .as_str()
                .map(|g| format!(" group=\"{g}\""))
                .unwrap_or_default();
            let tags = v["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();
            println!("[peer_profile_updated] peer={dev} type={dev_type}{grp} tags=[{tags}]");
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

pub(crate) async fn handle_caps_cmd(
    peer: Option<String>,
    enable: Option<String>,
    disable: Option<String>,
    json: bool,
    ui_port: u16,
) -> Result<()> {
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
        let (code, resp) = loopback_post(
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
            bail!(
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
        let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
        if code != 200 {
            bail!(
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
    let (code_local, resp_local) = loopback_get(ui_port, "/api/caps").await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    let (code_peers, resp_peers) = loopback_get(ui_port, "/api/peers/caps")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code_local != 200 {
        bail!(
            "node returned HTTP {code_local}: {}",
            String::from_utf8_lossy(&resp_local)
        );
    }
    if code_peers != 200 {
        bail!(
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

pub(crate) async fn handle_traffic_cmd(reset: bool, json: bool, ui_port: u16) -> Result<()> {
    if reset {
        let (code, resp) = loopback_post(ui_port, "/api/metrics/reset", b"{}")
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if code != 200 {
            bail!(
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

    let (code, resp) = loopback_get(ui_port, "/api/metrics").await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if code != 200 {
        bail!(
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
    println!("    {}", "-".repeat(74));
    println!(
        "    {:<18} {:<16} {:<16} {:<12} {:<12}",
        "Control (45717)",
        format_bytes(ctrl["tx_bytes"].as_u64().unwrap_or(0)),
        format_bytes(ctrl["rx_bytes"].as_u64().unwrap_or(0)),
        ctrl["tx_frames"].as_u64().unwrap_or(0),
        ctrl["rx_frames"].as_u64().unwrap_or(0)
    );
    println!(
        "    {:<18} {:<16} {:<16} {:<12} {:<12}",
        "File (45718)",
        format_bytes(file["tx_bytes"].as_u64().unwrap_or(0)),
        format_bytes(file["rx_bytes"].as_u64().unwrap_or(0)),
        "-",
        "-"
    );
    println!(
        "    {:<18} {:<16} {:<16} {:<12} {:<12}",
        "Input (45719)",
        format_bytes(input["tx_bytes"].as_u64().unwrap_or(0)),
        format_bytes(input["rx_bytes"].as_u64().unwrap_or(0)),
        input["tx_frames"].as_u64().unwrap_or(0),
        input["rx_frames"].as_u64().unwrap_or(0)
    );
    println!(
        "    {:<18} {:<16} {:<16} {:<12} {:<12}",
        "Screen (45720)",
        format_bytes(screen["tx_bytes"].as_u64().unwrap_or(0)),
        format_bytes(screen["rx_bytes"].as_u64().unwrap_or(0)),
        screen["tx_frames"].as_u64().unwrap_or(0),
        screen["rx_frames"].as_u64().unwrap_or(0)
    );

    println!("\n  Protocol Message Telemetry:");
    println!(
        "    Ping Probes:        Sent {:<8} Recv {}",
        msgs["pings_sent"].as_u64().unwrap_or(0),
        msgs["pings_recv"].as_u64().unwrap_or(0)
    );
    println!(
        "    Pong Replies:       Sent {:<8} Recv {}",
        msgs["pongs_sent"].as_u64().unwrap_or(0),
        msgs["pongs_recv"].as_u64().unwrap_or(0)
    );
    println!(
        "    Notifications:      Sent {:<8} Recv {}",
        msgs["notifies_sent"].as_u64().unwrap_or(0),
        msgs["notifies_recv"].as_u64().unwrap_or(0)
    );
    println!(
        "    Clipboard Sync:     Sent {:<8} Recv {}",
        msgs["clips_sent"].as_u64().unwrap_or(0),
        msgs["clips_recv"].as_u64().unwrap_or(0)
    );
    println!(
        "    File Offers:        Sent {:<8} Recv {}",
        msgs["file_offers_sent"].as_u64().unwrap_or(0),
        msgs["file_offers_recv"].as_u64().unwrap_or(0)
    );

    let established = v["sessions_established"].as_u64().unwrap_or(0);
    let closed = v["sessions_closed"].as_u64().unwrap_or(0);
    let handshakes_failed = v["handshakes_failed"].as_u64().unwrap_or(0);
    println!("\n  Session Lifecycle:");
    println!("    Sessions Established: {established}");
    println!("    Sessions Closed:      {closed}");
    println!("    Handshakes Failed:    {handshakes_failed}");

    Ok(())
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
