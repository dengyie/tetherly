// SPDX-License-Identifier: Apache-2.0 OR MIT
//! EasyTier sidecar discovery. Never links easytier*. Unicast overlay IPv4.
//! mDNS stays on Phase 1 LAN addresses only.

use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tetherly_core::{
    default_overlay_cidrs, in_overlay, overlay_ips_from_json, parse_cidr, DEFAULT_RPC_PORT,
    OVERLAY_PROBE_MS,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::debug;

#[derive(Debug, Clone)]
pub struct OverlayConfig {
    pub enabled: bool,
    pub cidrs: Vec<(Ipv4Addr, u8)>,
    pub rpc_addr: SocketAddr,
    pub cli: PathBuf,
    /// Explicit overlay targets (tests / config). Production uses RPC/CLI IPs + 45717.
    pub extra_peers: Vec<SocketAddr>,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cidrs: default_overlay_cidrs(),
            rpc_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_RPC_PORT)),
            cli: PathBuf::from("easytier-cli"),
            extra_peers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayStatus {
    pub present: bool,
    pub local_ips: Vec<Ipv4Addr>,
    pub peers: Vec<Ipv4Addr>,
    pub source: OverlaySource,
}

impl OverlayStatus {
    pub fn lan_only() -> Self {
        Self {
            present: false,
            local_ips: Vec::new(),
            peers: Vec::new(),
            source: OverlaySource::Absent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlaySource {
    Absent,
    Ifaces,
    Rpc,
    Cli,
    Manual,
}

pub fn local_overlay_ips(cidrs: &[(Ipv4Addr, u8)]) -> Vec<Ipv4Addr> {
    let mut out = BTreeSet::new();
    match if_addrs::get_if_addrs() {
        Ok(ifs) => {
            for iface in ifs {
                if iface.is_loopback() {
                    continue;
                }
                if let IpAddr::V4(v) = iface.ip() {
                    // Name hints (`et*`/`tun*`) are used to classify a path as overlay;
                    // bind/advertise still requires the address to sit in an overlay CIDR
                    // so a host TUN with a different prefix cannot leak into tests.
                    if in_overlay(v, cidrs) {
                        out.insert(v);
                    }
                }
            }
        }
        Err(e) => debug!(%e, "ifaddrs overlay scan"),
    }
    out.into_iter().collect()
}

pub fn overlay_bind_ips(cfg: &OverlayConfig) -> Vec<IpAddr> {
    if !cfg.enabled {
        return Vec::new();
    }
    local_overlay_ips(&cfg.cidrs)
        .into_iter()
        .map(IpAddr::V4)
        .collect()
}

pub fn overlay_bind_addrs(cfg: &OverlayConfig, port: u16) -> Vec<SocketAddr> {
    overlay_bind_ips(cfg)
        .into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect()
}

/// Collect overlay peer IPv4s. Missing EasyTier is success with empty lists (M2.4).
pub async fn discover(cfg: &OverlayConfig) -> OverlayStatus {
    if !cfg.enabled {
        return OverlayStatus::lan_only();
    }
    let local_ips = local_overlay_ips(&cfg.cidrs);
    let mut peers = BTreeSet::new();
    let mut source = if !local_ips.is_empty() {
        OverlaySource::Ifaces
    } else {
        OverlaySource::Absent
    };

    if let Some(text) = rpc_peer_text(cfg.rpc_addr).await {
        for ip in overlay_ips_from_json(&text, &cfg.cidrs) {
            if !local_ips.contains(&ip) {
                peers.insert(ip);
            }
        }
        source = OverlaySource::Rpc;
    } else if let Some(text) = cli_peer_text(&cfg.cli) {
        for ip in overlay_ips_from_json(&text, &cfg.cidrs) {
            if !local_ips.contains(&ip) {
                peers.insert(ip);
            }
        }
        source = OverlaySource::Cli;
    }

    let present = !local_ips.is_empty()
        || source == OverlaySource::Rpc
        || source == OverlaySource::Cli
        || !cfg.extra_peers.is_empty();
    if present && source == OverlaySource::Absent && !cfg.extra_peers.is_empty() {
        source = OverlaySource::Manual;
    }
    OverlayStatus {
        present,
        local_ips,
        peers: peers.into_iter().collect(),
        source: if present {
            source
        } else {
            OverlaySource::Absent
        },
    }
}

fn cli_peer_text(cli: &PathBuf) -> Option<String> {
    let mut cmd = Command::new(cli);
    cmd.arg("-o")
        .arg("json")
        .arg("peer")
        .arg("list")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        let mut cmd = Command::new(cli);
        cmd.arg("peer")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd.output().ok()?;
        if out.stdout.is_empty() {
            return None;
        }
        return String::from_utf8(out.stdout).ok();
    }
    if out.stdout.is_empty() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

async fn rpc_peer_text(addr: SocketAddr) -> Option<String> {
    let fut = async {
        let mut s = TcpStream::connect(addr).await.ok()?;
        let req = b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAccept: application/json\r\n\r\n";
        s.write_all(req).await.ok()?;
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.ok()?;
        let text = String::from_utf8_lossy(&buf);
        let body = text.split("\r\n\r\n").nth(1).unwrap_or(&text);
        if body.trim().is_empty() {
            return None;
        }
        Some(body.to_string())
    };
    tokio::time::timeout(Duration::from_millis(OVERLAY_PROBE_MS), fut)
        .await
        .unwrap_or_default()
}

pub async fn probe_control(ip: Ipv4Addr, port: u16) -> Option<SocketAddr> {
    let addr = SocketAddr::from((ip, port));
    match tokio::time::timeout(
        Duration::from_millis(OVERLAY_PROBE_MS),
        TcpStream::connect(addr),
    )
    .await
    {
        Ok(Ok(_)) => Some(addr),
        _ => None,
    }
}

pub fn parse_overlay_cidrs(raw: &str) -> Vec<(Ipv4Addr, u8)> {
    let parsed: Vec<_> = raw.split(',').filter_map(parse_cidr).collect();
    if parsed.is_empty() {
        default_overlay_cidrs()
    } else {
        parsed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn missing_sidecar_is_lan_only() {
        let cfg = OverlayConfig {
            enabled: true,
            cidrs: vec![("10.199.199.0".parse().unwrap(), 24)],
            cli: PathBuf::from("tetherly-no-such-easytier-cli"),
            rpc_addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 1)),
            extra_peers: Vec::new(),
        };
        let st = discover(&cfg).await;
        assert!(!st.present);
        assert!(st.peers.is_empty());
        assert_eq!(st.source, OverlaySource::Absent);
    }
}
