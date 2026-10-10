# Project State

## Objective
- Personal LAN-first device mesh (Tetherly): pairing + Noise, local OTP, EasyTier sidecar only;
  plus computer-side ANCS ingress for iPhone notifications (Phase 4 CI slice landed).

## Current Phase
- Phase 4 CI slice landed and pushed. Spec `docs/DEVELOPMENT.md` **v1.6**.
- `tetherly-core::ancs` — OS-free ANCS codec + `AncsIngress` state machine + `AncsTransport` port + `MemoryAncsTransport`.
- Hard constraints from §9.4 enforced by code structure: Data Source subscribed before Notification Source;
  `on_notification_source` only enqueues, `tick` writes Control Point; serial CP; byte-wise fragment reassembly;
  one retry of silent first response; disconnect backoff (500ms → 30s cap); uid dedup + PreExisting suppression (M4.3).
- `Node` wiring: `ancs_connect`, `ancs_*_source`, `ancs_tick`, `ancs_disconnected`, `ancs_state`, `set_open_allowlist`,
  `add_open_rule`, `open_candidate(id, &dyn Opener)`. UI two-step wizard (ANCS → SPAKE2).
- `tests/phase4.rs`: M4.1 simulated latency budget, M4.2 allowlist-gated open, M4.3 reconnect no re-pop,
  serial CP/fragmentation/Removed/no-transport inertness/backoff — 10 tests all green.
- **Manual-required**: real BLE GATT (Windows `windows` / CoreBluetooth / BlueZ), physical iPhone p95 < 2s,
  Bluetooth off-30s-on real-device test. M4.4 / M4.5 (optional iOS App, no VPN) not begun.
- Physical dual-desktop and OS cursor/key injection remain Manual-required.

## Current Branch
- `main` @ `https://github.com/dengyie/tetherly`
- Branch commit will be created after this session.

## Last Verified
- Local: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --locked` (loopback 10, phase1 7, phase2 4, phase3 4, phase4 10, core 93, crypto 10, net 1, node 4).
- `cargo deny check licenses bans sources` (easytier* still banned; no easytier in Cargo.toml).
- Host: rustc/cargo 1.99.0, `x86_64-pc-windows-gnu`, Scoop mingw 16.2.0.

## Active Risks
- Android without `libtetherly_android.so` cannot complete SPAKE2/Noise; placeholder `idPk = sha256(idSk)` is not Ed25519.
- Desktop UI is loopback HTTP, not Tauri.
- Host may already run EasyTier `10.144.144.0/24`; tests must isolate with a non-default CIDR (`10.199.199.0/24`).
- `snow` 0.9 has no `rekey()`; sessions return `RekeyRequired` after 2^16 msgs or 3600s.
- Input seq must stay session-monotonic; a new `InputServer` per call drops frames as replays.
- Phase 4 real BLE transport does not exist yet; ANCS ingress is untested on Windows/macOS/Linux GATT drivers.

## Active Blockers
- True SendInput / macOS / Wayland portal+libei and 100 physical dual-desktop round-trips (Manual-required).
- Physical Android cellular + user EasyTier for true M2.1 p95 (Manual-required).
- Real iPhone ANCS p95 < 2s and Bluetooth-off-30s-and-on verification (Manual-required).

## Current Focus
- Push Phase 4 CI slice to main, confirm CI green. Then begin documentation and stability.

## Next Milestone
- Phase 5 (single-item): Android controlled device, notify.reply, WinFsp mount, file persistent resume,
  Linux ANCS experience, embedded EasyTier FFI. Pending prioritisation.

## Key Artifacts
- `docs/DEVELOPMENT.md` (SSOT v1.6)
- `crates/tetherly-{core,crypto,net,node}` + `crates/tetherly-core/src/ancs.rs`
- `bins/tetherly-cli`, `tests/{loopback,phase1,phase2,phase3,phase4}.rs`, `ui/`, `android/`
- GitHub: `dengyie/tetherly`
- Vault: `Note/Project/tetherly/tetherly 开发文档.md`