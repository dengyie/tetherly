// SPDX-License-Identifier: Apache-2.0 OR MIT
//! LAN bind policy. Phase 1 listens on loopback + RFC1918 + link-local.
//! Never bind `0.0.0.0` / `::`. Skip the default EasyTier overlay so Phase 1
//! cannot accidentally advertise on a TUN.

use std::net::{IpAddr, Ipv4Addr};

/// EasyTier's documented default overlay. Phase 2 may use it; Phase 1 must not.
pub const EASYTIER_DEFAULT_NET: Ipv4Addr = Ipv4Addr::new(10, 144, 144, 0);
pub const EASYTIER_DEFAULT_PREFIX: u8 = 24;

pub fn is_easytier_default_overlay(ip: Ipv4Addr) -> bool {
    let net = u32::from(EASYTIER_DEFAULT_NET);
    let mask = !((1u32 << (32 - EASYTIER_DEFAULT_PREFIX)) - 1);
    (u32::from(ip) & mask) == (net & mask)
}

/// IPv4 addresses Tetherly may bind or advertise in Phase 1.
pub fn should_listen_v4(ip: Ipv4Addr) -> bool {
    if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
        return false;
    }
    if is_easytier_default_overlay(ip) {
        return false;
    }
    ip.is_loopback() || ip.is_private() || ip.is_link_local()
}

/// v1 does not do IPv6. Loopback v6 is also refused so a dual-stack OS cannot
/// sneak `::` into the listen set.
pub fn should_listen(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => should_listen_v4(v),
        IpAddr::V6(_) => false,
    }
}

pub fn is_unspecified_bind(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_unspecified(),
        IpAddr::V6(v) => v.is_unspecified(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_and_rfc1918_ok() {
        assert!(should_listen("127.0.0.1".parse().unwrap()));
        assert!(should_listen("10.0.0.5".parse().unwrap()));
        assert!(should_listen("192.168.1.9".parse().unwrap()));
        assert!(should_listen("172.16.4.1".parse().unwrap()));
        assert!(should_listen("169.254.10.2".parse().unwrap()));
    }

    #[test]
    fn public_and_unspecified_rejected() {
        assert!(!should_listen("0.0.0.0".parse().unwrap()));
        assert!(!should_listen("8.8.8.8".parse().unwrap()));
        assert!(!should_listen("1.1.1.1".parse().unwrap()));
        assert!(!should_listen("::".parse().unwrap()));
        assert!(!should_listen("::1".parse().unwrap()));
    }

    #[test]
    fn easytier_overlay_skipped_in_phase1() {
        assert!(!should_listen("10.144.144.1".parse().unwrap()));
        assert!(!should_listen("10.144.144.254".parse().unwrap()));
        assert!(should_listen("10.144.145.1".parse().unwrap()));
    }
}
