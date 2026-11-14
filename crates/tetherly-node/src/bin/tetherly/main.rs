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
    let stdout_tty = std::io::stdout().is_terminal();
    // One tracing pipeline: rotating tetherly.log, live-reloadable filter
    // (`POST /api/log/level`), panic hook. See `tetherly_node::logging`.
    tetherly_node::logging::init(&data_dir, stdout_tty);
    // P0-1: client commands authenticate privileged loopback routes with the
    // running daemon's per-start token. `run` mints its own in spawn_ui.
    if !matches!(&cli.cmd, Cmd::Run { .. }) {
        let token = std::env::var("TETHERLY_API_TOKEN")
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .or_else(|| tetherly_node::apitoken::read(&data_dir));
        tetherly_node::set_api_token(token);
    }
    if matches!(&cli.cmd, Cmd::Run { .. }) {
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            pid = std::process::id(),
            os = std::env::consts::OS,
            arch = std::env::consts::ARCH,
            data_dir = %data_dir.display(),
            filter = %tetherly_node::logging::control()
                .map(|c| c.current())
                .unwrap_or_default(),
            "tetherly daemon starting"
        );
    }
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
