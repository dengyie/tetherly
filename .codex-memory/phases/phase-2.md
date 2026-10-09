# Phase 2 — EasyTier sidecar CI slice

- Closed: 2026-10-09
- Spec: `docs/DEVELOPMENT.md` v1.4
- Delivered: sidecar discovery (RPC 15888 / easytier-cli / iface CIDR), unicast TCP 45717, overlay bind separate from LAN/mDNS, LAN path preference, UI lan_only, `tests/phase2.rs`
- Validation: cargo fmt/clippy `-D warnings`/test --workspace --locked/deny licenses bans sources
- Manual-required: M2.1 Android cellular p95; true Wi-Fi down → overlay for new notify
- Stop: Do not open Phase 3
