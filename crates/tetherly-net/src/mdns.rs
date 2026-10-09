// SPDX-License-Identifier: Apache-2.0 OR MIT
//! LAN-only discovery. Never bind this to an EasyTier TUN.

use crate::error::NetError;
use std::net::{IpAddr, SocketAddr};
use tetherly_core::{should_listen, DeviceId};

pub const SERVICE_TYPE: &str = "_tetherly._tcp.local.";

#[derive(Debug, Clone)]
pub struct LanPeer {
    pub hostname: String,
    pub device_id: DeviceId,
    pub fp: String,
    pub caps: String,
    pub addrs: Vec<SocketAddr>,
}

#[cfg(feature = "lan")]
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

#[cfg(feature = "lan")]
pub fn advertise(
    daemon: &ServiceDaemon,
    hostname: &str,
    port: u16,
    device_id: &DeviceId,
    fp12: &str,
    caps: &str,
    addrs: &[IpAddr],
) -> Result<(), NetError> {
    let inst = hostname.replace('.', "-");
    let lan: Vec<IpAddr> = addrs
        .iter()
        .copied()
        .filter(|ip| should_listen(*ip))
        .collect();
    if lan.is_empty() {
        return Err(NetError::Discovery("no LAN address to advertise".into()));
    }
    let joined = lan
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let info = ServiceInfo::new(
        SERVICE_TYPE,
        &inst,
        &format!("{hostname}.local."),
        joined.as_str(),
        port,
        &[
            ("v", "1"),
            ("id", device_id.as_str()),
            ("fp", fp12),
            ("caps", caps),
        ][..],
    )
    .map_err(|e| NetError::Discovery(e.to_string()))?;
    daemon
        .register(info)
        .map_err(|e| NetError::Discovery(e.to_string()))
}

#[cfg(feature = "lan")]
pub fn daemon() -> Result<ServiceDaemon, NetError> {
    ServiceDaemon::new().map_err(|e| NetError::Discovery(e.to_string()))
}

#[cfg(feature = "lan")]
pub fn browse(daemon: &ServiceDaemon) -> Result<mdns_sd::Receiver<ServiceEvent>, NetError> {
    daemon
        .browse(SERVICE_TYPE)
        .map_err(|e| NetError::Discovery(e.to_string()))
}

#[cfg(feature = "lan")]
pub fn lan_peer_from_info(info: &ServiceInfo) -> Option<LanPeer> {
    let id = info.get_property_val_str("id")?;
    let device_id = DeviceId::parse(id).ok()?;
    let fp = info
        .get_property_val_str("fp")
        .unwrap_or_default()
        .to_string();
    let caps = info
        .get_property_val_str("caps")
        .unwrap_or("notify,clip,file")
        .to_string();
    let port = info.get_port();
    let addrs = info
        .get_addresses()
        .iter()
        .copied()
        .filter(|ip| should_listen(*ip))
        .map(|ip| SocketAddr::new(ip, port))
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return None;
    }
    Some(LanPeer {
        hostname: info.get_hostname().to_string(),
        device_id,
        fp,
        caps,
        addrs,
    })
}

#[cfg(not(feature = "lan"))]
pub fn advertise(
    _hostname: &str,
    _port: u16,
    _device_id: &DeviceId,
    _fp12: &str,
    _caps: &str,
    _addrs: &[IpAddr],
) -> Result<(), NetError> {
    Err(NetError::Discovery("lan feature disabled".into()))
}
