# Session log

## 2026-10-09
- Task: Land Phase 1 CI slice (LAN desktop + Android skeleton); update spec to v1.3; push.
- Actions: `tetherly-node` persist/LAN/notify/clip/file; loopback HTTP UI; Android NLService; `tests/phase1.rs`; `copy_otp` CandidateExpired; deny BSL-1.0; DEVELOPMENT.md v1.3; Obsidian 速读.
- Results: workspace tests green (loopback 10, phase1 7, core 58); clippy `-D warnings`; cargo deny licenses/bans/sources ok.
- Next: Stop. Do not open Phase 2. Physical Android / JNI `.so` remain Manual-required.
- Blockers: No physical phone in this session; no cargo-ndk `.so`.

## 2026-10-09
- Task: Root-cause-fix Phase 0 review findings; push GitHub; mount Obsidian.
- Actions: Split PairBind AEAD nonces (`tetherlybndI`/`tetherlybndR`); commit trust only after Noise transport; Clock + handshake 20s timeout + TCP 30/min; UTF-8 truncate; RNG fail-closed; inner `msg_id` + ReplayGuard + `needs_rekey`; on-wire Hello-name MITM test; delete tmp helper.
- Results: clippy `-D warnings` and `cargo test --workspace --locked` green. Spec bumped to v1.2. Public repo created.
- Next: Phase 1 only on explicit request.
- Blockers: None.

## 2026-10-09
- Task: Fix Windows CI `cargo fmt --check` newline failure.
- Actions: Add `.gitattributes` (`eol=lf`) so Windows runners do not rewrite rustfmt Unix newlines; disable `core.autocrlf` on the Windows job before toolchain setup.
- Results: Pending CI after push.
- Next: Confirm Windows check is green.
- Blockers: None.
