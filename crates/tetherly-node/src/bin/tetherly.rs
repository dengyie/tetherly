// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Desktop CLI: LAN + optional EasyTier sidecar overlay.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tetherly_node::{Node, NodeConfig, SystemSink};
use tracing::{info, warn};

#[derive(Parser)]
#[command(
    name = "tetherly",
    about = "Tetherly LAN node with optional EasyTier sidecar"
)]
struct Cli {
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    name: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Persist identity, listen on LAN + loopback, advertise mDNS.
    Run {
        #[arg(long, default_value_t = 45717)]
        port: u16,
        /// Extra EasyTier overlay CIDRs, comma-separated. Default 10.144.144.0/24.
        #[arg(long)]
        overlay_cidr: Option<String>,
        /// Disable overlay bind/scan (LAN only).
        #[arg(long, default_value_t = false)]
        no_overlay: bool,
        /// Stream an adb device's screen (serial, e.g. 10.144.144.51:5555)
        /// instead of the local DXGI desktop.
        #[arg(long)]
        screen_adb: Option<String>,
        /// Bind a second node on one host: file/input/screen/UI ports.
        #[arg(long)]
        file_port: Option<u16>,
        #[arg(long)]
        input_port: Option<u16>,
        #[arg(long)]
        screen_port: Option<u16>,
        #[arg(long)]
        ui_port: Option<u16>,
        /// Receive iPhone notifications over ANCS/BLE from the paired device with
        /// this Bluetooth address (Windows notation, e.g. `11:22:33:44:55:66`).
        /// Pair the phone in Windows Bluetooth settings first. Runtime against a
        /// real iPhone is Manual-required.
        #[arg(long)]
        ancs: Option<String>,
    },
    /// Print device_id of the persisted identity.
    Identity,
    /// Show an 8-digit PIN (3 min TTL) on the *running* node.
    ///
    /// A PIN is only meaningful to the process holding it, so this asks the
    /// already-running node (its loopback UI API) to arm one. Starting a second
    /// node here would print a code no listener could ever accept.
    Pin {
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Ask the *running* node to dial a peer and pair or resume.
    ///
    /// Starting a second node here would bind the same LAN ports as the daemon
    /// and fail with "no LAN address bound". The live node already holds the
    /// identity, the PIN, and the listeners, so this only posts to its loopback
    /// UI API — the same route the desktop shell's 「连接」 button uses.
    Dial {
        #[arg(long)]
        addr: SocketAddr,
        #[arg(long)]
        pin: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Ask the *running* node to deliver a notify.push to one live peer.
    ///
    /// The body is the user's own text: it is delivered, never executed, and
    /// never logged. A second node cannot deliver it — only the process that
    /// holds the Noise session can. With one live peer the target is implied;
    /// with more than one, `--peer` is required.
    Notify {
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
        #[arg(long, default_value = "com.example.sms")]
        app_id: String,
        /// `tdev_…` of the peer. Omit only when exactly one peer is live.
        #[arg(long)]
        peer: Option<String>,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Ask the *running* node to send a file to one live peer.
    ///
    /// The file is read from the local filesystem and offered over the live
    /// session. With one live peer the target is implied; with more than one,
    /// `--peer` is required.
    Send {
        /// Path to the file to send.
        path: PathBuf,
        /// Optional filename override to declare to the peer.
        #[arg(long)]
        name: Option<String>,
        /// `tdev_…` of the peer. Omit only when exactly one peer is live.
        #[arg(long)]
        peer: Option<String>,
        /// Watch transfer progress until completion.
        #[arg(long)]
        watch: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Cancel an in-flight file transfer by its transfer ID.
    Cancel {
        /// Transfer ID to cancel (e.g. tfl_...).
        id: String,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
    /// Query status of the *running* daemon (peers, transfers, and listen interfaces).
    Status {
        /// Watch mode: refresh status periodically until interrupted.
        #[arg(long, short)]
        watch: bool,
        /// Refresh interval in seconds for --watch (default: 1).
        #[arg(long, default_value_t = 1)]
        interval: u64,
        /// Output raw JSON snapshot instead of formatted text.
        #[arg(long)]
        json: bool,
        /// UI API port of the running node (loopback only).
        #[arg(long, default_value_t = 45716)]
        ui_port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("tetherly=info".parse().unwrap()),
        )
        .init();
    if std::env::var("TETHERLY_INSECURE_LOG").ok().as_deref() == Some("1") {
        tracing::warn!("TETHERLY_INSECURE_LOG=1 is set; debug profile only");
    }

    let cli = Cli::parse();
    let mut cfg = NodeConfig::default();
    if let Some(d) = cli.data_dir {
        cfg.data_dir = d;
    }
    if let Some(n) = cli.name {
        cfg.name = n;
    }

    match cli.cmd {
        Cmd::Identity => {
            let node = Node::start(cfg)?;
            println!("{}", node.identity().device_id());
        }
        Cmd::Pin { ui_port } => {
            // Arm the PIN in the live node via its loopback API: the code is
            // only usable by the process that generated it.
            let (code, body) = tetherly_node::loopback_post(ui_port, "/api/pin", b"{}")
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
                    String::from_utf8_lossy(&body)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&body).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let pin = v
                .get("pin")
                .and_then(|p| p.as_str())
                .ok_or_else(|| anyhow::anyhow!("node returned no pin"))?;
            println!("{pin}");
            println!("不要把数字发给任何人,只在你自己的另一台设备上输入。");
        }
        Cmd::Run {
            port,
            overlay_cidr,
            no_overlay,
            screen_adb,
            file_port,
            input_port,
            screen_port,
            ui_port,
            ancs,
        } => {
            cfg.control_port = port;
            // Real presentation: received frames open a window via GDI BitBlt.
            cfg.screen_sink = Some(Arc::new(SystemSink::new()));
            // Real injection for the controlled side (45719): received key and
            // mouse events drive SendInput. Harmless while no visitor holds
            // the seat (nothing arrives until the controller crosses an edge).
            cfg.input_sink = Some(Arc::new(std::sync::Mutex::new(
                tetherly_node::SystemInputSink,
            )));
            if let Some(serial) = screen_adb {
                // The screen is the adb device; encode it losslessly like DXGI.
                cfg.screen_source = Some(Arc::new(tetherly_node::EncodedSource::new(
                    tetherly_node::AdbSource::new(Some(serial)),
                    Arc::new(tetherly_node::ZstdCodec::default()),
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
                    Ok(Some(link)) => Some(link),
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
            let node = Node::start(cfg)?;
            info!(device_id = %node.identity().device_id(), "run");
            info!("ui http://127.0.0.1:45716");
            if let Some((_transport, inbound)) = ancs_link {
                node.spawn_ancs_pump(inbound);
                info!(
                    "ANCS link up; pair the iPhone in Windows Bluetooth settings if you have not"
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
        }
        Cmd::Dial { addr, pin, ui_port } => {
            // The running node owns the listeners. A second `Node::start` binds
            // the same ports and dies with "no LAN address bound".
            let payload = serde_json::json!({ "addr": addr.to_string(), "pin": pin });
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
            println!("dialed {addr}");
        }
        Cmd::Notify {
            title,
            body,
            app_id,
            peer,
            ui_port,
        } => {
            let payload = serde_json::json!({
                "title": title,
                "body": body,
                "app_id": app_id,
                "peer": peer,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/notify", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused notify (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let peer = v.get("peer").and_then(|p| p.as_str()).unwrap_or("?");
            println!("notified {peer}");
        }
        Cmd::Send {
            path,
            name,
            peer,
            watch,
            ui_port,
        } => {
            let path_str = path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("path contains invalid UTF-8"))?;
            let payload = serde_json::json!({
                "path": path_str,
                "name": name,
                "peer": peer,
            });
            let bytes = serde_json::to_vec(&payload)?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/send", &bytes)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "node refused file send (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            let v: serde_json::Value =
                serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response: {e}"))?;
            let transfer_id = v.get("transfer_id").and_then(|p| p.as_str()).unwrap_or("?");
            let target_peer = v.get("peer").and_then(|p| p.as_str()).unwrap_or("?");
            let file_name = v.get("name").and_then(|p| p.as_str()).unwrap_or("?");
            let size = v.get("size").and_then(|p| p.as_u64()).unwrap_or(0);
            println!("offered {file_name} ({size} bytes) to {target_peer} [{transfer_id}]");

            if watch {
                println!("watching transfer progress for [{transfer_id}]...");
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    let (status_code, state_resp) =
                        tetherly_node::loopback_get(ui_port, "/api/state").await?;
                    if status_code != 200 {
                        continue;
                    }
                    let state: serde_json::Value = match serde_json::from_slice(&state_resp) {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let transfers = state.get("transfers").and_then(|t| t.as_array());
                    let current = transfers.and_then(|list| {
                        list.iter().find(|t| {
                            t.get("transfer_id").and_then(|id| id.as_str()) == Some(transfer_id)
                        })
                    });
                    if let Some(t) = current {
                        let transferred =
                            t.get("transferred").and_then(|v| v.as_u64()).unwrap_or(0);
                        let total = t.get("total").and_then(|v| v.as_u64()).unwrap_or(size);
                        let speed_bps = t.get("speed_bps").and_then(|v| v.as_u64()).unwrap_or(0);
                        let phase = t
                            .get("phase")
                            .and_then(|v| v.as_str())
                            .unwrap_or("transferring");
                        let pct = if total > 0 {
                            (transferred as f64 / total as f64) * 100.0
                        } else {
                            0.0
                        };
                        print!(
                            "\r[{:>5.1}%] {} / {} ({}) [{}]      ",
                            pct,
                            format_bytes(transferred),
                            format_bytes(total),
                            format_speed(speed_bps),
                            phase
                        );
                        use std::io::Write;
                        let _ = std::io::stdout().flush();

                        if phase == "completed" {
                            println!(
                                "\n✓ Transfer completed: {file_name} ({})",
                                format_bytes(total)
                            );
                            break;
                        } else if phase == "failed" {
                            let err_msg = t
                                .get("error")
                                .and_then(|e| e.as_str())
                                .unwrap_or("transfer failed");
                            anyhow::bail!("\n✗ Transfer failed: {err_msg}");
                        }
                    }
                }
            }
        }
        Cmd::Cancel { id, ui_port } => {
            let body = serde_json::to_vec(&serde_json::json!({ "id": id }))?;
            let (code, resp) = tetherly_node::loopback_post(ui_port, "/api/file/cancel", &body)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                         start one with `tetherly --data-dir <dir> run` first"
                    )
                })?;
            if code != 200 {
                anyhow::bail!(
                    "cancel failed (HTTP {code}): {}",
                    String::from_utf8_lossy(&resp)
                );
            }
            println!("cancelled transfer [{id}]");
        }
        Cmd::Status {
            watch,
            interval,
            json,
            ui_port,
        } => loop {
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
        },
    }
    Ok(())
}

fn print_status_summary(v: &serde_json::Value) {
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
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.2} GiB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MiB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KiB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn format_speed(bps: u64) -> String {
    format!("{}/s", format_bytes(bps))
}
