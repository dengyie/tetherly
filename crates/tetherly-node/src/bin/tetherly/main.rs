// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Desktop CLI: LAN + optional EasyTier sidecar overlay.

// GUI subsystem on Windows: `tetherly run` is a tray-resident daemon, not a
// console tool. Without this the console host (conhost) opens a visible
// window whenever the binary is launched by the autostart scheduled task or
// Explorer — the TUI-equivalent of a stray black rectangle next to the tray
// icon. `tetherly-cli` and the other CLI bins stay console-subsystem: they
// are piping tools. Logs go to the rotating `tetherly.log` (logsink); stdout
// is mirrored only when it is a terminal.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tetherly_node::{Node, NodeConfig, SystemSink};
use tracing::{info, warn};

// ---- CLI surface (v0.1.64 / ADR-128) ----
mod cli;
mod cli_device;
mod cli_ops;
mod cli_screen;
mod cli_tools;
pub(crate) use cli::*;
pub(crate) use cli_device::*;
pub(crate) use cli_ops::*;
pub(crate) use cli_screen::*;
pub(crate) use cli_tools::*;

// ---- dispatch + command handlers (v0.1.64 / ADR-128) ----
mod cmd_action;
mod cmd_ancs;
mod cmd_audio;
mod cmd_audit;
mod cmd_bench;
mod cmd_camera;
mod cmd_clip;
mod cmd_core;
mod cmd_display;
mod cmd_files;
mod cmd_focus;
mod cmd_handoff;
mod cmd_input;
mod cmd_locate;
mod cmd_media;
mod cmd_messages;
mod cmd_mic;
mod cmd_notify;
mod cmd_peers;
mod cmd_power;
mod cmd_profile;
mod cmd_screen;
mod cmd_screen_encoding;
mod cmd_screen_overlay;
mod cmd_screen_quality;
mod cmd_screen_replay;
mod cmd_screen_session;
mod cmd_screen_tile;
mod cmd_topology;
mod dispatch;
mod util;
use dispatch::dispatch;

fn main() -> Result<()> {
    // GUI-subsystem binaries have no console at all; stdout/stderr writes
    // would fail outright on Windows (ERROR_INVALID_HANDLE) and every
    // eprintln! diagnostic — the whole [takeover] chain — would panic
    // silently via the panic-in-println path. Detach the parent-inherited
    // console (present when launched from a terminal or when the scheduled
    // task grants one) so writes become harmless no-ops, mirroring the unix
    // SIGPIPE defaulting below. Declared locally instead of via the
    // windows-sys feature tree: kernel32 has exported FreeConsole since
    // forever, and one extern beats a whole extra crate feature gate.
    #[cfg(all(windows, not(debug_assertions)))]
    unsafe {
        #[link(name = "kernel32")]
        extern "system" {
            fn FreeConsole() -> i32;
        }
        FreeConsole();
    }
    // A closed downstream pipe (`tetherly accept | head`) must end the
    // process quietly like any classic tool, not panic in println!.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let args: Vec<String> = std::env::args().collect();
    let effective_args = if args.len() <= 1 || (args.len() == 2 && args[1].starts_with("-psn_")) {
        vec![args[0].clone(), "run".into()]
    } else {
        args
    };

    let cli = Cli::parse_from(effective_args);

    // The log lives in the effective data dir: the Windows SYSTEM launcher
    // passes --data-dir, and default_data_dir() under LocalSystem resolves
    // to the hidden systemprofile tree (an unbounded 3.7 MB duplicate there,
    // 2026-11-14).
    let data_dir = cli
        .data_dir
        .clone()
        .unwrap_or_else(tetherly_node::store::default_data_dir);
    let _ = tetherly_node::store::ensure_dir(&data_dir);
    let log_file = tetherly_node::logsink::RotatingFile::open(
        &data_dir.join("tetherly.log"),
        tetherly_node::logsink::LOG_FILE_CAP,
    )
    .ok();
    let stdout_tty = std::io::stdout().is_terminal();
    let mirror = tetherly_node::logsink::mirror_stdout(log_file.is_some(), stdout_tty);
    let sink = tetherly_node::logsink::LogSink::new(log_file, mirror);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("tetherly=info".parse().unwrap())
                // mdns-sd spams ERROR "Cannot find valid addrs for TYPE_SRV"
                // on IPv6-only and TUN interfaces (sandbox-VPS soak logged it
                // continuously); it is interface-enumeration noise, not a
                // discovery failure — default it off so real errors surface.
                // RUST_LOG still overrides for debugging.
                .add_directive("mdns_sd=off".parse().unwrap()),
        )
        // Colour codes only for a human terminal; the file stays greppable.
        .with_ansi(stdout_tty)
        .with_writer(move || sink.clone())
        .init();
    if std::env::var("TETHERLY_INSECURE_LOG").ok().as_deref() == Some("1") {
        tracing::warn!("TETHERLY_INSECURE_LOG=1 is set; debug profile only");
    }

    #[cfg(target_os = "macos")]
    if matches!(&cli.cmd, Cmd::Run { .. }) {
        let ui_port = match &cli.cmd {
            Cmd::Run { ui_port, .. } => ui_port.unwrap_or(45716),
            _ => 45716,
        };
        let device_name = cli.name.clone().unwrap_or_else(|| "Tetherly".into());
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel();

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        std::thread::spawn(move || {
            let res = rt.block_on(async move { dispatch(cli).await });
            let _ = shutdown_tx.send(());
            if let Err(e) = res {
                eprintln!("tetherly runtime error: {e}");
            }
        });

        tetherly_node::platform::tray::run_macos_tray_on_main_thread(
            ui_port,
            device_name,
            shutdown_rx,
        );
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async { dispatch(cli).await })
}
