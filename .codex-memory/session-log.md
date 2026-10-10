# Session log

## 2026-10-10
- Task: Open Phase 4 (waiver granted) and land the computer-side iPhone ANCS CI slice; spec v1.6.
- Actions: new `tetherly-core::ancs` (UUID/event/CP codec, `DataAssembler` byte-wise reassembly, `AncsIngress`, `AncsTransport`, `MemoryAncsTransport`, `ancs_actions`, `ancs_source_id`); §9.4 constraints made structural (subscribe order, enqueue-only callback + `tick` write, serial `in_flight`, one retry of silence, 500ms→30s backoff, uid dedup, `PreExisting` drop); `Opener` port (`SystemOpener` no-shell / `MemoryOpener`); node `ancs_*` surface + `open_candidate`; `Candidate.actions` carried to DTO; UI two-step wizard and `/api/ancs/*`, `/api/open`; `tests/phase4.rs` (10 tests, M4.1/M4.2/M4.3 + serial/reassembly/Removed/inert/backoff).
- Results: local fmt/clippy `-D warnings`/`cargo test --workspace --locked`/`cargo deny check` all green (loopback 10, phase1 7, phase2 4, phase3 4, phase4 10, core 93, crypto 10, net 1, node 4). Real BLE GATT + physical iPhone stay Manual-required.
- Next: push to `main`, confirm GitHub Actions green.
- Blockers: No real BLE stack or iPhone on this host.

## 2026-10-09
- Task: Harden the landed Phase 3 input slice and remove a CI flake.
- Actions: `InputServer::local_move` now forwards normalized `Move` when the seat is already `Remote` (motion was silently dropped after the edge); `MemorySink.applied` bounded by `INPUT_SINK_TRACE_MAX` (was unbounded + cloned per frame); integration `wait_live` budget raised 8s → 25s to exceed `HANDSHAKE_TIMEOUT` (20s), which was flaking `m2_5` on loaded Windows runners.
- Results: local workspace tests green (loopback 10, phase1 7, phase2 4, phase3 4, core 71, crypto 10, net 1, node 4); CI runs `38006315548` and `38006923616` all five jobs **success**.
- Next: Stop. Phase 4 needs a new waiver.
- Blockers: None.

## 2026-10-09
- Task: Land Phase 3 native keyboard/mouse CI slice; spec v1.5; push so GitHub CI compiles and tests.
- Actions: `tetherly-core::input` (binary `TIN1`, seq window, `InputServer`/`InputClient`/`MemorySink`, edge state machine); node listens on separate TCP 45719 with `SessionConfig.resume_only` (pairing still 45717); `caps.update inputport=`; input frames only on the 45719 channel; `tests/phase3.rs`.
- Results: local fmt/clippy `-D warnings`/`cargo test --workspace --locked`/deny green. First CI run `37989970083` failed: Windows `m3_2` (shared `identity.bin.tmp` collision) and Android unit tests (`org.json` stub). Fixed with per-write temp names and a real `org.json` test dependency. CI run `37997269866` all five jobs **success**.
- Next: Stop. Do not open Phase 4. Physical SendInput / dual-desktop remain Manual-required.
- Blockers: No real dual-desktop or OS injection in CI.

## 2026-10-09
- Task: Land Phase 2 EasyTier sidecar CI slice; spec v1.4; do not open Phase 3.
- Actions: overlay CIDR/PathKind + sidecar RPC/CLI/iface discovery; overlay bind separate from LAN/mDNS; LAN-wins attach; UI「仅局域网」; `tests/phase2.rs` M2.2–M2.5; clippy nits; isolate host TUN with `10.199.199.0/24`.
- Results: fmt/clippy `-D warnings`/workspace tests (loopback 10, phase1 7, phase2 4, core 63)/cargo deny green. No `easytier*` in Cargo.toml.
- Next: Stop. Do not open Phase 3. M2.1 physical cellular p95 remains Manual-required.
- Blockers: No physical phone + cellular EasyTier soak in this session.

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
