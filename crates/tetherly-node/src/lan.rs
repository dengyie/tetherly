// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Phase 1 listen set: loopback + private NICs, never 0.0.0.0.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tetherly_core::{classify, is_unspecified_bind, should_listen};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

/// One local address plus why it is, or is not, in the LAN listen set. A
/// read-only diagnostic: it never changes the bind set, it only explains it, and
/// it is built on the same `classify` the binder uses so the two cannot drift.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IfaceReport {
    /// Interface name, or `"loopback"` for the seeded `127.0.0.1`.
    pub iface: String,
    pub ip: IpAddr,
    /// In the LAN listen set (bound, advertised, mDNS-eligible).
    pub listen: bool,
    /// `None` when `listen` is true; otherwise a stable reason token.
    pub reason: Option<&'static str>,
}

static CACHED_REPORT: std::sync::Mutex<Option<(std::time::Instant, Vec<IfaceReport>)>> =
    std::sync::Mutex::new(None);

/// Every local address, classified. `127.0.0.1` is always first because
/// [`lan_ips`] seeds it unconditionally even on a host whose OS omits it.
/// Caches results for 5 seconds to prevent kernel syscall storms under UI polling.
pub fn interface_report() -> Vec<IfaceReport> {
    const TTL: std::time::Duration = std::time::Duration::from_secs(5);
    if let Ok(guard) = CACHED_REPORT.lock() {
        if let Some((ts, ref report)) = *guard {
            if ts.elapsed() < TTL {
                return report.clone();
            }
        }
    }

    let mut out = vec![IfaceReport {
        iface: "loopback".into(),
        ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        listen: true,
        reason: None,
    }];
    match if_addrs::get_if_addrs() {
        Ok(ifs) => {
            for iface in ifs {
                let ip = iface.ip();
                if out.iter().any(|r| r.ip == ip) {
                    continue;
                }
                out.push(report_row(iface.name, ip));
            }
        }
        Err(e) => warn!(%e, "ifaddrs failed; interface report is loopback only"),
    }

    if let Ok(mut guard) = CACHED_REPORT.lock() {
        *guard = Some((std::time::Instant::now(), out.clone()));
    }

    out
}

fn report_row(iface: String, ip: IpAddr) -> IfaceReport {
    let class = classify(ip);
    IfaceReport {
        iface,
        ip,
        listen: class.listens(),
        reason: class.reason(),
    }
}

/// Log, once at startup, every interface left out of the LAN set and why. The
/// common case it makes visible: a Tailscale/NetBird `100.64.0.0/10` address is
/// neither RFC1918 nor link-local, so it is silently skipped unless the user
/// names it with `--overlay-cidr`.
pub fn log_skipped_interfaces() {
    let report = interface_report();
    let mut skipped = 0;
    for r in &report {
        if let Some(reason) = r.reason {
            debug!(iface = %r.iface, ip = %r.ip, reason, "interface not in the LAN set");
            skipped += 1;
        }
    }
    if skipped > 0 {
        debug!(
            skipped_count = skipped,
            "filtered non-LAN network interfaces"
        );
    }
}

/// The LAN listen set: loopback plus every interface address `classify` puts in
/// [`tetherly_core::BindClass::Listen`]. The loopback seed is first so a host
/// whose OS does not enumerate `127.0.0.1` still gets it.
pub fn lan_ips() -> Vec<IpAddr> {
    let mut out = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
    match if_addrs::get_if_addrs() {
        Ok(ifs) => {
            for iface in ifs {
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
    let listeners = bind_many_optional(addrs).await;
    if listeners.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "no LAN address bound",
        ));
    }
    Ok(listeners)
}

async fn bind_with_retry(addr: SocketAddr) -> std::io::Result<TcpListener> {
    let mut attempts = 0;
    loop {
        match TcpListener::bind(addr).await {
            Ok(l) => return Ok(l),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && attempts < 20 => {
                attempts += 1;
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Same guards as [`bind_many`] but tolerates an empty result. Data ports
/// (file/input/screen) are optional: a missing interface is not fatal.
pub async fn bind_many_optional(addrs: &[SocketAddr]) -> Vec<TcpListener> {
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
        match bind_with_retry(*addr).await {
            Ok(l) => {
                info!(%addr, "listen");
                listeners.push(l);
            }
            Err(e) => {
                warn!(%addr, %e, "bind failed");
            }
        }
    }
    listeners
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
        match bind_with_retry(*addr).await {
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

    #[test]
    fn interface_report_matches_the_actual_listen_set() {
        let report = interface_report();
        assert_eq!(report[0].ip, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(report[0].listen && report[0].reason.is_none());
        for r in &report {
            // The report and the binder share one classifier: `listen` must be
            // exactly `should_listen`, and a row either listens or says why.
            assert_eq!(r.listen, should_listen(r.ip), "{r:?}");
            assert_eq!(r.listen, r.reason.is_none(), "{r:?}");
        }
        let mut listening: Vec<IpAddr> = report.iter().filter(|r| r.listen).map(|r| r.ip).collect();
        let mut ips = lan_ips();
        listening.sort();
        ips.sort();
        assert_eq!(
            ips, listening,
            "report and bind set must name the same addresses"
        );
    }
}
