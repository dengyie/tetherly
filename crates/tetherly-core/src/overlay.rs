// SPDX-License-Identifier: Apache-2.0 OR MIT
//! EasyTier sidecar discovery helpers. No EasyTier crate. Overlay IPv4 only.

use crate::bind::{is_easytier_default_overlay, EASYTIER_DEFAULT_NET, EASYTIER_DEFAULT_PREFIX};
use serde_json::Value;
use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub const DEFAULT_RPC_PORT: u16 = 15888;
pub const OVERLAY_PROBE_MS: u64 = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    Lan,
    Overlay,
}

pub fn default_overlay_cidrs() -> Vec<(Ipv4Addr, u8)> {
    vec![(EASYTIER_DEFAULT_NET, EASYTIER_DEFAULT_PREFIX)]
}

pub fn ipv4_in_cidr(ip: Ipv4Addr, net: Ipv4Addr, prefix: u8) -> bool {
    if prefix == 0 {
        return true;
    }
    if prefix > 32 {
        return false;
    }
    let shift = 32 - prefix;
    let mask = if shift == 32 {
        0
    } else {
        !((1u32 << shift) - 1)
    };
    (u32::from(ip) & mask) == (u32::from(net) & mask)
}

pub fn parse_cidr(s: &str) -> Option<(Ipv4Addr, u8)> {
    let s = s.trim();
    if let Some((ip, p)) = s.split_once('/') {
        let ip = ip.parse().ok()?;
        let p: u8 = p.parse().ok()?;
        if (1..=32).contains(&p) {
            return Some((ip, p));
        }
        return None;
    }
    let ip: Ipv4Addr = s.parse().ok()?;
    Some((ip, 32))
}

pub fn in_overlay(ip: Ipv4Addr, cidrs: &[(Ipv4Addr, u8)]) -> bool {
    if cidrs.is_empty() {
        return is_easytier_default_overlay(ip);
    }
    cidrs.iter().any(|(net, p)| ipv4_in_cidr(ip, *net, *p))
}

pub fn is_overlay_addr(addr: SocketAddr, cidrs: &[(Ipv4Addr, u8)]) -> bool {
    match addr.ip() {
        IpAddr::V4(v) => in_overlay(v, cidrs),
        IpAddr::V6(_) => false,
    }
}

/// Interface names that look like an EasyTier TUN. Do not match `eth*`.
pub fn iface_looks_easytier(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    if n.contains("easytier") {
        return true;
    }
    if n.starts_with("eth") {
        return false;
    }
    n.starts_with("et") || n.starts_with("tun")
}

pub fn path_kind(addr: SocketAddr, cidrs: &[(Ipv4Addr, u8)]) -> PathKind {
    if is_overlay_addr(addr, cidrs) {
        PathKind::Overlay
    } else {
        PathKind::Lan
    }
}

/// Keep an existing LAN session; replace overlay with LAN; otherwise take the new path.
pub fn prefer_existing(existing: PathKind, incoming: PathKind) -> PathKind {
    match (existing, incoming) {
        (PathKind::Lan, PathKind::Overlay) => PathKind::Lan,
        (PathKind::Overlay, PathKind::Lan) => PathKind::Lan,
        (a, _) => a,
    }
}

fn push_ip(out: &mut BTreeSet<Ipv4Addr>, ip: Ipv4Addr, cidrs: &[(Ipv4Addr, u8)]) {
    if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() || ip.is_loopback() {
        return;
    }
    if in_overlay(ip, cidrs) {
        out.insert(ip);
    }
}

fn parse_ip_token(s: &str) -> Option<Ipv4Addr> {
    let s = s.trim().trim_matches(|c| c == '"' || c == '\'');
    if let Some((ip, _)) = s.split_once('/') {
        return ip.parse().ok();
    }
    s.parse().ok()
}

fn walk(v: &Value, out: &mut BTreeSet<Ipv4Addr>, cidrs: &[(Ipv4Addr, u8)]) {
    match v {
        Value::String(s) => {
            if let Some(ip) = parse_ip_token(s) {
                push_ip(out, ip, cidrs);
            }
        }
        Value::Array(a) => {
            for x in a {
                walk(x, out, cidrs);
            }
        }
        Value::Object(m) => {
            for x in m.values() {
                walk(x, out, cidrs);
            }
        }
        _ => {}
    }
}

/// Pull overlay IPv4s from easytier-cli / RPC JSON of any shape.
pub fn overlay_ips_from_json(text: &str, cidrs: &[(Ipv4Addr, u8)]) -> Vec<Ipv4Addr> {
    let mut out = BTreeSet::new();
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        walk(&v, &mut out, cidrs);
    }
    overlay_ips_from_text(text, cidrs, &mut out);
    out.into_iter().collect()
}

pub fn overlay_ips_from_text(text: &str, cidrs: &[(Ipv4Addr, u8)], out: &mut BTreeSet<Ipv4Addr>) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'/')
            {
                i += 1;
            }
            if let Ok(s) = std::str::from_utf8(&bytes[start..i]) {
                if let Some(ip) = parse_ip_token(s) {
                    push_ip(out, ip, cidrs);
                }
            }
        } else {
            i += 1;
        }
    }
}

pub fn default_rpc_addr() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, DEFAULT_RPC_PORT))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cidr_default_overlay() {
        let c = default_overlay_cidrs();
        assert!(in_overlay("10.144.144.8".parse().unwrap(), &c));
        assert!(!in_overlay("10.0.0.8".parse().unwrap(), &c));
        assert!(!in_overlay("127.0.0.1".parse().unwrap(), &c));
    }

    #[test]
    fn json_peer_list_shapes() {
        let c = default_overlay_cidrs();
        let a = r#"{"peer_infos":[{"ipv4_addr":"10.144.144.2/24","hostname":"phone"}]}"#;
        let b = r#"[{"virtual_ipv4":"10.144.144.3","conns":[{"remote_ipv4":"10.144.144.4"}]}]"#;
        let ips = overlay_ips_from_json(a, &c);
        assert_eq!(ips, vec!["10.144.144.2".parse::<Ipv4Addr>().unwrap()]);
        let mut ips = overlay_ips_from_json(b, &c);
        ips.sort();
        assert_eq!(
            ips,
            vec![
                "10.144.144.3".parse::<Ipv4Addr>().unwrap(),
                "10.144.144.4".parse::<Ipv4Addr>().unwrap()
            ]
        );
    }

    #[test]
    fn table_text_extracts_overlay_only() {
        let c = default_overlay_cidrs();
        let mut out = BTreeSet::new();
        overlay_ips_from_text(
            "id cost ipv4\n1 p2p 10.144.144.9\n2 lan 192.168.1.8\n",
            &c,
            &mut out,
        );
        assert_eq!(
            out.into_iter().collect::<Vec<_>>(),
            vec!["10.144.144.9".parse::<Ipv4Addr>().unwrap()]
        );
    }

    #[test]
    fn iface_name_heuristic() {
        assert!(iface_looks_easytier("EasyTier"));
        assert!(iface_looks_easytier("et0"));
        assert!(iface_looks_easytier("tun0"));
        assert!(!iface_looks_easytier("eth0"));
        assert!(!iface_looks_easytier("WLAN"));
    }

    #[test]
    fn prefer_lan_over_overlay() {
        assert_eq!(
            prefer_existing(PathKind::Lan, PathKind::Overlay),
            PathKind::Lan
        );
        assert_eq!(
            prefer_existing(PathKind::Overlay, PathKind::Lan),
            PathKind::Lan
        );
        assert_eq!(
            prefer_existing(PathKind::Overlay, PathKind::Overlay),
            PathKind::Overlay
        );
    }
}
