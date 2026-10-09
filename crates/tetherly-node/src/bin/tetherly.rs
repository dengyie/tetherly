// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 1 desktop CLI.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use tetherly_core::NotifyPush;
use tetherly_node::{Node, NodeConfig};
use tracing::info;

#[derive(Parser)]
#[command(name = "tetherly", about = "Tetherly Phase 1 LAN node")]
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
    },
    /// Print device_id of the persisted identity.
    Identity,
    /// Show an 8-digit PIN for pairing (3 min TTL).
    Pin,
    /// Dial a peer and pair or resume.
    Dial {
        #[arg(long)]
        addr: SocketAddr,
        #[arg(long)]
        pin: Option<String>,
    },
    /// Inject a synthetic notify.push (tests / demos). Body is not logged.
    InjectNotify {
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
        #[arg(long, default_value = "com.example.sms")]
        app_id: String,
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
        Cmd::Pin => {
            let node = Node::start(cfg)?;
            println!("{}", node.generate_pin()?);
            println!("不要把数字发给任何人，只在你自己的另一台设备上输入。");
        }
        Cmd::Run { port } => {
            cfg.control_port = port;
            let node = Node::start(cfg)?;
            info!(device_id = %node.identity().device_id(), "run");
            info!("ui http://127.0.0.1:45716");
            node.run().await?;
        }
        Cmd::Dial { addr, pin } => {
            let node = Node::start(cfg)?;
            if let Some(p) = pin {
                let digits = tetherly_core::pin::parse_pin(&p)?;
                node.set_pin(digits);
            }
            node.dial(addr).await?;
            println!("connected {}", node.identity().device_id());
            node.run().await?;
        }
        Cmd::InjectNotify {
            title,
            body,
            app_id,
        } => {
            let node = Node::start(cfg)?;
            let push = NotifyPush {
                uid: "inject".into(),
                app_id,
                app_name: "inject".into(),
                title,
                body,
                ts: 1,
                actions: vec!["copy".into()],
            };
            let _ = node
                .ingest_local_push(node.identity().device_id().clone(), push)
                .await;
            for c in node.candidates() {
                println!("candidate {} has_otp={}", c.id, c.has_otp);
            }
        }
    }
    Ok(())
}
