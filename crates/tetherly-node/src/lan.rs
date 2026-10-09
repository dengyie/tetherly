// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 1 listen set: loopback + private NICs, never 0.0.0.0.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tetherly_core::{is_unspecified_bind, should_listen};
use tokio::net::TcpListener;
use tracing::{info, warn};

pub fn lan_ips() -> Vec<IpAddr> {
    let mut out = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
    match if_addrs::get_if_addrs() {
        Ok(ifs) => {
            for iface in ifs {
                if iface.is_loopback() {
                    continue;
                }
                let ip = iface.ip();
                if should_listen(ip) && !out.contains(&ip) {
                    out.push(ip);
                }
            }
        }
        Err(e) => warn!(%e, "ifaddrs failed; loopback only"),
    }
    out
}

pub fn control_bind_addrs(port: u16) -> Vec<SocketAddr> {
    lan_ips()
        .into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect()
}

pub async fn bind_many(addrs: &[SocketAddr]) -> std::io::Result<Vec<TcpListener>> {
    let mut listeners = Vec::new();
    for addr in addrs {
        if is_unspecified_bind(addr.ip()) {
            warn!(%addr, "refusing unspecified bind");
            continue;
        }
        if !should_listen(addr.ip()) {
            warn!(%addr, "skipping non-LAN bind");
            continue;
        }
        match TcpListener::bind(*addr).await {
            Ok(l) => {
                info!(%addr, "listen");
                listeners.push(l);
            }
            Err(e) => {
                warn!(%addr, %e, "bind failed");
            }
        }
    }
    if listeners.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "no LAN address bound",
        ));
    }
    Ok(listeners)
}

/// Overlay listen set. Explicit EasyTier TUN IPs only; still never `0.0.0.0`.
/// Empty is success: EasyTier is optional (M2.4).
pub async fn bind_overlay(addrs: &[SocketAddr]) -> Vec<TcpListener> {
    let mut listeners = Vec::new();
    for addr in addrs {
        if is_unspecified_bind(addr.ip()) {
            warn!(%addr, "refusing unspecified overlay bind");
            continue;
        }
        match TcpListener::bind(*addr).await {
            Ok(l) => {
                info!(%addr, "overlay listen");
                listeners.push(l);
            }
            Err(e) => warn!(%addr, %e, "overlay bind failed"),
        }
    }
    listeners
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_includes_loopback() {
        assert!(lan_ips().contains(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(!lan_ips().iter().any(|ip| is_unspecified_bind(*ip)));
    }
}
