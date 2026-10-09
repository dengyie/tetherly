# Project State

## Objective
- Personal LAN-first device mesh (Tetherly): pairing + Noise, local OTP, EasyTier sidecar only.

## Current Phase
- Phase 2 CI slice landed. Spec `docs/DEVELOPMENT.md` **v1.4**.
- Physical Android cellular p95 (M2.1) and true Wi-Fi→overlay failover remain Manual-required.
- Do not open Phase 3 (desktop input / DeskFlow).

## Current Branch
- `main` @ `https://github.com/dengyie/tetherly`

## Last Verified
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --locked` (loopback 10, phase1 7, phase2 4, core 63, crypto 10, net 1, node 4)
- `cargo deny check licenses bans sources` (easytier* still banned; no easytier in Cargo.toml)
- Host: rustc/cargo 1.99.0, `x86_64-pc-windows-gnu`, Scoop mingw 16.2.0 (not MinGW.org 6.3)

## Active Risks
- Android without `libtetherly_android.so` cannot complete SPAKE2/Noise; placeholder `idPk = sha256(idSk)` is not Ed25519.
- Desktop UI is loopback HTTP, not Tauri.
- Host may already run EasyTier `10.144.144.0/24`; tests must isolate with a non-default CIDR.
- `snow` 0.9 has no `rekey()`; sessions return `RekeyRequired` after 2^16 msgs or 3600s.

## Active Blockers
- Physical Android cellular + user EasyTier for true M2.1 p95 (Manual-required).
- Phase 3 blocked until Phase 2 device gates or explicit waiver.

## Current Focus
- Stop. Do not open Phase 3 desktop input.

## Next Milestone
- Phase 3 only when Phase 2 exit is waived or physical M2.1 lands: native Noise input port 45719.

## Key Artifacts
- `docs/DEVELOPMENT.md` (SSOT v1.4)
- `crates/tetherly-{core,crypto,net,node}`, `bins/tetherly-cli`, `tests/{loopback,phase1,phase2}.rs`, `ui/`, `android/`
- GitHub: `dengyie/tetherly`
- Vault: `Note/Project/tetherly/tetherly 开发文档.md`
