// SPDX-License-Identifier: Apache-2.0 OR MIT
//! LAN-only discovery. Never bind this to an EasyTier TUN.

#[cfg(feature = "lan")]
use crate::error::NetError;
#[cfg(feature = "lan")]
use mdns_sd::{ServiceDaemon, ServiceInfo};
#[cfg(feature = "lan")]
use tetherly_core::DeviceId;

pub const SERVICE_TYPE: &str = "_tetherly._tcp.local.";

#[cfg(feature = "lan")]
pub fn advertise(
    daemon: &ServiceDaemon,
    hostname: &str,
    port: u16,
    device_id: &DeviceId,
    fp12: &str,
    caps: &str,
) -> Result<(), NetError> {
    let inst = hostname.replace('.', "-");
    let info = ServiceInfo::new(
        SERVICE_TYPE,
        &inst,
        &format!("{hostname}.local."),
        "",
        port,
        &[
            ("v", "1"),
            ("id", device_id.as_str()),
            ("fp", fp12),
            ("caps", caps),
        ][..],
    )
    .map_err(|e| NetError::Discovery(e.to_string()))?
    .enable_addr_auto();
    daemon
        .register(info)
        .map_err(|e| NetError::Discovery(e.to_string()))
}

#[cfg(feature = "lan")]
pub fn daemon() -> Result<ServiceDaemon, NetError> {
    ServiceDaemon::new().map_err(|e| NetError::Discovery(e.to_string()))
}
