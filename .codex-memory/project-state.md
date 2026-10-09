# Project State

## Objective
- Personal LAN-first device mesh (Tetherly): pairing + Noise, local OTP, EasyTier sidecar only.

## Current Phase
- Phase 3 CI slice landed and pushed. Spec `docs/DEVELOPMENT.md` **v1.5**.
- Native Noise input port 45719 is resume-only; payload is binary `TIN1`; CI injects `MemorySink`.
- Physical dual-desktop and OS cursor/key injection remain Manual-required.
- Do not open Phase 4 (iPhone ANCS) without a new waiver.

## Current Branch
- `main` @ `https://github.com/dengyie/tetherly`
- HEAD `5cac066` (CI green on all five jobs).

## Last Verified
- GitHub Actions `ci` run `37997269866`: android, deny, check(ubuntu/windows/macos) all **success**.
- Local: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --locked` (loopback 10, phase1 7, phase2 4, phase3 4, core 69, crypto 10, net 1, node 4).
- `cargo deny check licenses bans sources` (easytier* still banned; no easytier in Cargo.toml).
- Host: rustc/cargo 1.99.0, `x86_64-pc-windows-gnu`, Scoop mingw 16.2.0 (not MinGW.org 6.3).

## Active Risks
- Android without `libtetherly_android.so` cannot complete SPAKE2/Noise; placeholder `idPk = sha256(idSk)` is not Ed25519.
- Desktop UI is loopback HTTP, not Tauri.
- Host may already run EasyTier `10.144.144.0/24`; tests must isolate with a non-default CIDR (`10.199.199.0/24`).
- `snow` 0.9 has no `rekey()`; sessions return `RekeyRequired` after 2^16 msgs or 3600s.
- Input seq must stay session-monotonic; a new `InputServer` per call drops frames as replays.

## Active Blockers
- True SendInput / macOS / Wayland portal+libei and 100 physical dual-desktop round-trips (Manual-required).
- Physical Android cellular + user EasyTier for true M2.1 p95 (Manual-required).
- Phase 4 gated behind an explicit waiver.

## Current Focus
- Stop Phase 3 feature work. Keep memory and Obsidian docs at v1.5.

## Next Milestone
- Phase 4 iPhone ANCS only with a new waiver; otherwise polish and Manual-required device gates.

## Key Artifacts
- `docs/DEVELOPMENT.md` (SSOT v1.5)
- `crates/tetherly-{core,crypto,net,node}`, `bins/tetherly-cli`,
  `tests/{loopback,phase1,phase2,phase3}.rs`, `ui/`, `android/`
- GitHub: `dengyie/tetherly`
- Vault: `Note/Project/tetherly/tetherly 开发文档.md`
