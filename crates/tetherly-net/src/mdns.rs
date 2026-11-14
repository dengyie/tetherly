// SPDX-License-Identifier: Apache-2.0 OR MIT
//! LAN-only discovery. Never bind this to an EasyTier TUN.

use crate::error::NetError;
use std::net::IpAddr;
#[cfg(feature = "lan")]
use tetherly_core::should_listen;
use tetherly_core::DeviceId;

pub const SERVICE_TYPE: &str = "_tetherly._tcp.local.";

#[cfg(feature = "lan")]
pub use mdns_sd::{Receiver as MdnsReceiver, ServiceDaemon, ServiceEvent, ServiceInfo};

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
            ("name", hostname),
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
/// Whether an mDNS `fullname` (as delivered by `ServiceRemoved`) is exactly
/// the instance [`advertise`] registers for `name`. Exact, so removing
/// "macbook" never expires a peer called "mac".
pub fn instance_matches(fullname: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    fullname
        .strip_suffix(SERVICE_TYPE)
        .and_then(|s| s.strip_suffix('.'))
        .is_some_and(|inst| inst.eq_ignore_ascii_case(&name.replace('.', "-")))
}

pub fn browse(daemon: &ServiceDaemon) -> Result<MdnsReceiver<ServiceEvent>, NetError> {
    daemon
        .browse(SERVICE_TYPE)
        .map_err(|e| NetError::Discovery(e.to_string()))
}

#[cfg(feature = "lan")]
pub fn stop_browse(daemon: &ServiceDaemon) -> Result<(), NetError> {
    daemon
        .stop_browse(SERVICE_TYPE)
        .map_err(|e| NetError::Discovery(e.to_string()))
}

#[cfg(feature = "lan")]
pub fn daemon() -> Result<ServiceDaemon, NetError> {
    ServiceDaemon::new().map_err(|e| NetError::Discovery(e.to_string()))
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

#[cfg(not(feature = "lan"))]
pub fn daemon() -> Result<(), NetError> {
    Err(NetError::Discovery("lan feature disabled".into()))
}

#[cfg(test)]
mod instance_tests {
    use super::*;

    #[test]
    fn removal_matches_only_the_exact_instance() {
        let full = format!("macbook-pro.{SERVICE_TYPE}");
        assert!(instance_matches(&full, "macbook-pro"));
        assert!(instance_matches(&full, "macbook.pro"));
        assert!(!instance_matches(&full, "mac"));
        assert!(!instance_matches(&full, "book"));
        assert!(!instance_matches(
            &format!("my-macbook-pro.{SERVICE_TYPE}"),
            "macbook-pro"
        ));
        assert!(!instance_matches(&full, ""));
    }
}
