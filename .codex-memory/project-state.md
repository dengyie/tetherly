# Project State

## Objective
- Personal LAN-first device mesh (Tetherly): pairing + Noise, local OTP, EasyTier sidecar only.

## Current Phase
- Phase 1 CI slice landed. Spec `docs/DEVELOPMENT.md` **v1.3**.
- Physical Android p95 / 8h soak / JNI `.so` remain Manual-required.
- Do not open Phase 2 (EasyTier sidecar).

## Current Branch
- `main` (push pending this session) @ `https://github.com/dengyie/tetherly`

## Last Verified
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --locked` (loopback 10, phase1 7, core 58, crypto 10, net 1, node 3)
- `cargo deny check licenses bans sources` (BSL-1.0 allowed for clipboard-win)
- Host: rustc/cargo 1.99.0, `x86_64-pc-windows-gnu`, Scoop mingw 16.2.0 (not MinGW.org 6.3)

## Active Risks
- Android without `libtetherly_android.so` cannot complete SPAKE2/Noise; placeholder `idPk = sha256(idSk)` is not Ed25519.
- Desktop UI is loopback HTTP, not Tauri.
- First-pair Noise still takes `n_pk` from Hello when trust is empty; PairBind already bound those keys.
- `snow` 0.9 has no `rekey()`; sessions return `RekeyRequired` after 2^16 msgs or 3600s.

## Active Blockers
- Physical Android device + JNI `.so` for true M1.1 / 8h (Manual-required).
- Phase 2 blocked until Phase 1 device gates or explicit waiver.

## Current Focus
- Stop. Do not open Phase 2 EasyTier.

## Next Milestone
- Phase 2 only when Phase 1 exit is waived or physical Android gates land: EasyTier sidecar unicast.

## Key Artifacts
- `docs/DEVELOPMENT.md` (SSOT v1.3)
- `crates/tetherly-{core,crypto,net,node}`, `bins/tetherly-cli`, `tests/{loopback,phase1}.rs`, `ui/`, `android/`
- GitHub: `dengyie/tetherly`
- Vault: `Note/Project/tetherly/tetherly 开发文档.md`
