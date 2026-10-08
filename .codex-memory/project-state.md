# Project State

## Objective
- Personal LAN-first device mesh (Tetherly): pairing + Noise, local OTP later, EasyTier sidecar only.

## Current Phase
- Phase 0 protocol skeleton landed. Spec `docs/DEVELOPMENT.md` **v1.2**.

## Current Branch
- `main` (initial public repo)

## Last Verified
- `cargo fmt --all`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace --locked` (loopback 10, core 46, crypto 9)
- Host: rustc/cargo 1.99.0, `x86_64-pc-windows-gnu`, Scoop mingw 16.2.0 (not MinGW.org 6.3)

## Active Risks
- First-pair Noise still takes `n_pk` from Hello when trust is empty; PairBind already bound those keys.
- `snow` 0.9 has no `rekey()`; sessions return `RekeyRequired` after 2^16 msgs or 3600s.
- Windows GNU link fails if PATH prefers MinGW.org 6.3 over Scoop mingw.

## Active Blockers
- None for Phase 0.

## Current Focus
- Stop. Do not open Phase 1 (mDNS, Android, GUI).

## Next Milestone
- Phase 1 only when explicitly requested: LAN desktop + Android notify path.

## Key Artifacts
- `docs/DEVELOPMENT.md` (SSOT v1.2)
- `crates/tetherly-{core,crypto,net}`, `bins/tetherly-cli`, `tests/loopback.rs`
- GitHub: `dengyie/tetherly`
- Vault: `Note/Project/tetherly/tetherly 开发文档.md`
