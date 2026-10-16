// SPDX-License-Identifier: Apache-2.0 OR MIT
//! LAN bind policy. Listens on loopback + RFC1918 + link-local.
//! Never bind `0.0.0.0` / `::`. Skip the default EasyTier overlay so mDNS
//! cannot advertise on a TUN. Overlay bind is a separate Phase 2 path.

use std::net::{IpAddr, Ipv4Addr};

/// EasyTier's documented default overlay. Phase 2 may use it; Phase 1 must not.
pub const EASYTIER_DEFAULT_NET: Ipv4Addr = Ipv4Addr::new(10, 144, 144, 0);
pub const EASYTIER_DEFAULT_PREFIX: u8 = 24;

pub fn is_easytier_default_overlay(ip: Ipv4Addr) -> bool {
    let net = u32::from(EASYTIER_DEFAULT_NET);
    let mask = !((1u32 << (32 - EASYTIER_DEFAULT_PREFIX)) - 1);
    (u32::from(ip) & mask) == (net & mask)
}

/// Why an address is, or is not, in the LAN listen set. This is the single
/// source of truth for the decision: [`should_listen`] is exactly
/// `classify(ip).listens()`, so a diagnostic built on `classify` can never
/// disagree with what the binder actually does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindClass {
    /// In the set: bound, advertised, and mDNS-eligible.
    Listen,
    /// A `127/8` alias other than the canonical `127.0.0.1`. Only the canonical
    /// loopback address is bound — extra `127.x` aliases add no reachability and
    /// would multiply listeners on a host that assigns them (e.g. a resolver
    /// stub on `127.0.0.53`).
    LoopbackAlias,
    /// `0.0.0.0` / `::` — an unspecified bind would listen on every interface.
    Unspecified,
    /// `255.255.255.255`.
    Broadcast,
    /// `224.0.0.0/4`.
    Multicast,
    /// EasyTier's documented default overlay. Phase 1 keeps mDNS off the TUN;
    /// the explicit overlay bind is a separate Phase 2 path.
    Overlay,
    /// RFC 6598 shared address space (`100.64.0.0/10`) — the shape a
    /// Tailscale/NetBird fabric hands out. It is neither RFC1918 nor
    /// link-local, so v1 does not bind it on its own; the user must name it
    /// with `--overlay-cidr 100.64.0.0/10`.
    CgnatOverlay,
    /// v1 is IPv4-only, so every IPv6 address is skipped — loopback included,
    /// so a dual-stack OS cannot sneak `::` into the listen set.
    Ipv6,
    /// Not loopback, RFC1918 or link-local: a public address.
    NotLan,
}

impl BindClass {
    pub fn listens(self) -> bool {
        matches!(self, BindClass::Listen)
    }

    /// Stable, machine-readable reason token; `None` when the address listens.
    pub fn reason(self) -> Option<&'static str> {
        match self {
            BindClass::Listen => None,
            BindClass::LoopbackAlias => Some("loopback-alias-not-canonical"),
            BindClass::Unspecified => Some("unspecified"),
            BindClass::Broadcast => Some("broadcast"),
            BindClass::Multicast => Some("multicast"),
            BindClass::Overlay => Some("easytier-default-overlay"),
            BindClass::CgnatOverlay => Some("cgnat-overlay-needs-explicit-cidr"),
            BindClass::Ipv6 => Some("ipv6-unsupported-in-v1"),
            BindClass::NotLan => Some("not-loopback-rfc1918-or-link-local"),
        }
    }
}

/// RFC 6598 shared address space, `100.64.0.0/10`.
pub fn is_cgnat(ip: Ipv4Addr) -> bool {
    (u32::from(ip) & 0xffc0_0000) == 0x6440_0000
}

/// Classify an IPv4 address. Guard clauses, one decision per line.
pub fn classify_v4(ip: Ipv4Addr) -> BindClass {
    if ip.is_unspecified() {
        return BindClass::Unspecified;
    }
    if ip.is_broadcast() {
        return BindClass::Broadcast;
    }
    if ip.is_multicast() {
        return BindClass::Multicast;
    }
    if is_easytier_default_overlay(ip) {
        return BindClass::Overlay;
    }
    if is_cgnat(ip) {
        return BindClass::CgnatOverlay;
    }
    if ip.is_loopback() {
        return if ip == Ipv4Addr::LOCALHOST {
            BindClass::Listen
        } else {
            BindClass::LoopbackAlias
        };
    }
    if ip.is_private() || ip.is_link_local() {
        return BindClass::Listen;
    }
    BindClass::NotLan
}

pub fn classify(ip: IpAddr) -> BindClass {
    match ip {
        IpAddr::V4(v) => classify_v4(v),
        IpAddr::V6(_) => BindClass::Ipv6,
    }
}

/// IPv4 addresses Tetherly may bind or advertise in Phase 1.
pub fn should_listen_v4(ip: Ipv4Addr) -> bool {
    classify_v4(ip).listens()
}

/// v1 does not do IPv6. Loopback v6 is also refused so a dual-stack OS cannot
/// sneak `::` into the listen set.
pub fn should_listen(ip: IpAddr) -> bool {
    classify(ip).listens()
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

    #[test]
    fn classify_names_the_reason_and_stays_consistent() {
        use BindClass::*;
        assert_eq!(classify_v4("127.0.0.1".parse().unwrap()), Listen);
        assert_eq!(classify_v4("127.0.0.53".parse().unwrap()), LoopbackAlias);
        assert_eq!(classify_v4("192.168.1.9".parse().unwrap()), Listen);
        assert_eq!(classify_v4("169.254.10.2".parse().unwrap()), Listen);
        assert_eq!(classify_v4("0.0.0.0".parse().unwrap()), Unspecified);
        assert_eq!(classify_v4("255.255.255.255".parse().unwrap()), Broadcast);
        assert_eq!(classify_v4("239.1.2.3".parse().unwrap()), Multicast);
        assert_eq!(classify_v4("10.144.144.7".parse().unwrap()), Overlay);
        // A CGNAT fabric (Tailscale/NetBird) is named separately from a plain
        // public address, because only the former is the user's own network.
        assert_eq!(classify_v4("100.64.0.9".parse().unwrap()), CgnatOverlay);
        assert_eq!(classify_v4("100.127.255.1".parse().unwrap()), CgnatOverlay);
        assert_eq!(classify_v4("100.128.0.1".parse().unwrap()), NotLan);
        assert_eq!(classify_v4("8.8.8.8".parse().unwrap()), NotLan);
        assert_eq!(classify("::".parse().unwrap()), Ipv6);
        assert_eq!(classify("::1".parse().unwrap()), Ipv6);
        // Only `Listen` listens, and every non-`Listen` names a reason.
        for c in [
            Listen,
            LoopbackAlias,
            Unspecified,
            Broadcast,
            Multicast,
            Overlay,
            CgnatOverlay,
            Ipv6,
            NotLan,
        ] {
            assert_eq!(c.listens(), c == Listen);
            assert_eq!(c.reason().is_none(), c == Listen);
        }
    }
}
