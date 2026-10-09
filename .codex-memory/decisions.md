# Decisions

## 2026-10-09 - Input uses a separate resume-only Noise port (45719)
- Decision: keyboard/mouse traffic runs on its own TCP port 45719 with `SessionConfig.resume_only`. Hello after that only accepts `ResumeNoise`; an unknown peer gets `NetError::InputRequiresTrust`. Pairing stays on 45717. Frames are binary `TIN1` and never enter control-plane JSON.
- Rationale: ADR-005 wants input off the control plane and off any DeskFlow/Lan Mouse/KDE code path. Reusing the existing trust store keeps pairing single-sourced while the input channel stays trust-gated.
- Alternatives considered: multiplex input onto 45717 (mixes latency-sensitive frames with control JSON); a JSON mouse payload (rejected by the spec).
- Impact: Node binds 45719 on LAN + overlay (never `0.0.0.0`), advertises `inputport=` in `caps.update`, and keeps one persistent `InputServer` so `seq` stays session-monotonic.
- Rollback trigger: Only with a spec revision; the port split is what keeps pairing and injection separable.
- Related files: `crates/tetherly-core/src/input.rs`, `crates/tetherly-net/src/session.rs`, `crates/tetherly-node/src/runtime.rs`, `tests/phase3.rs`

## 2026-10-09 - Input CI injects MemorySink, not Win32 SendInput
- Decision: CI and loopback tests apply input events to a `MemorySink`. Real OS injection (Win SendInput / macOS / Wayland portal+libei) stays Manual-required and out of the engine.
- Rationale: `tetherly-core` must stay OS-free (`#![forbid(unsafe_code)]`, no `cfg(target_os)`); CI runners have no real dual desktop. Keeps the engine deterministic and reviewable.
- Alternatives considered: calling SendInput behind `cfg(windows)` in CI (nondeterministic, unsafe, no second desktop).
- Impact: M3.1's 100 round-trips are proven in-process; the physical 100-trip and injection gates are documented as Manual-required.
- Rollback trigger: When the platform injector lands as a separate node-layer module.
- Related files: `crates/tetherly-core/src/input.rs`, `crates/tetherly-node/src/runtime.rs`

## 2026-10-09 - Overlay bind is separate from LAN/mDNS
- Decision: LAN listeners skip EasyTier `10.144.144.0/24` so mDNS never advertises on TUN. Overlay IPs bind on a second path. Missing sidecar returns empty lists (success).
- Rationale: EasyTier TUN often drops multicast; ADR-006 forbids mDNS as the cross-net discovery. Binding `0.0.0.0` is still forbidden.
- Alternatives considered: Advertise mDNS on overlay (fails on many TUNs); skip overlay bind entirely (peers cannot accept inbound overlay TCP).
- Impact: Discovery is unicast 45717 from RPC/CLI/`extra_peers`. UI shows「仅局域网」when sidecar is absent.
- Rollback trigger: None while EasyTier stays sidecar.
- Related files: `crates/tetherly-node/src/{lan,sidecar,runtime}.rs`, ADR-006

## 2026-10-09 - extra_peers dial even outside overlay CIDR
- Decision: Config/test `extra_peers` are dial targets regardless of CIDR. Overlay bind and JSON/CLI IP harvest still require the address to sit in overlay CIDR.
- Rationale: Loopback tests and manual IPs (127.0.0.1:ephemeral) must unicast without a real TUN. Host EasyTier `10.144.144.0/24` must not leak into “absent sidecar” tests; those use `10.199.199.0/24`.
- Alternatives considered: Filter extra_peers by CIDR (broke M2.5); treat any `et*`/`tun*` IP as overlay bind (broke isolation on this host).
- Impact: M2.4/M2.5 stay deterministic on a machine that already runs EasyTier.
- Rollback trigger: If production needs name-only TUN bind, gate it behind explicit config, not the default.
- Related files: `crates/tetherly-node/src/sidecar.rs`, `tests/phase2.rs`

## 2026-10-09 - Live LAN session wins over overlay attach
- Decision: If a peer is already live on LAN, drop inbound overlay TCP without recording. Overlay session teardown removes live/session only when `p.addr == sess_addr`.
- Rationale: Same Wi-Fi should stay on LAN (M2.2). Overlay scan loops must not replace or delete a healthy LAN map entry.
- Alternatives considered: Last-writer-wins (flaps to overlay); tear down any session for that device_id (kills LAN when overlay probe ends).
- Impact: Dual-path hosts keep one LAN peer; overlay is for when LAN is gone.
- Rollback trigger: None for v1 path preference.
- Related files: `crates/tetherly-node/src/runtime.rs`, `crates/tetherly-core/src/overlay.rs`

## 2026-10-09 - Phase 1 desktop is loopback HTTP, not Tauri
- Decision: Serve `ui/index.html` on `127.0.0.1:45716` only. `src-tauri` is the next desktop-shell iteration.
- Rationale: CI must not require WebView2. Host allowlist is 127.0.0.1/localhost; never bind `0.0.0.0`.
- Alternatives considered: Full Tauri now (CI/WebView2 cost); CLI-only (no click-to-copy/file confirm UX).
- Impact: Users open a local URL; PIN is returned only from POST `/api/pin`.
- Rollback trigger: When Tauri packaging is the milestone, keep the same `/api/*` contract.
- Related files: `crates/tetherly-node/src/uihttp.rs`, `ui/index.html`

## 2026-10-09 - Allow BSL-1.0 in cargo deny
- Decision: Add Boost Software License 1.0 to `deny.toml` allow list.
- Rationale: `arboard` → `clipboard-win` / `error-code` are BSL-1.0 (OSI/FSF). Not copyleft. Still deny GPL/AGPL/LGPL and ban `easytier*`.
- Alternatives considered: Drop `arboard` and use only Win32 clipboard (hurts Linux/mac later).
- Impact: Phase 1 clip path can use arboard.
- Rollback trigger: If a BSL crate with extra patent/field restrictions appears.
- Related files: `deny.toml`, ADR-009

## 2026-10-09 - File data uses advertised peer file port
- Decision: Sender writes TFL1 chunks to `peer.file_port` from `caps.update fileport=`, never the control port.
- Rationale: Two nodes on one host bind ephemeral ports; control 45717 ≠ file 45718.
- Alternatives considered: Always 45718 (collides in loopback tests with port 0).
- Impact: Caps must include `fileport=` after attach.
- Rollback trigger: None for Phase 1.
- Related files: `crates/tetherly-node/src/runtime.rs`, `crates/tetherly-net/src/filechan.rs`

## 2026-10-09 - Force LF in git for rustfmt
- Decision: `.gitattributes` sets `eol=lf` for text; Windows CI also sets `core.autocrlf=false` before fmt.
- Rationale: `rustfmt.toml` uses `newline_style = "Unix"`. GitHub `windows-latest` defaults `core.autocrlf=true`, so checkout rewrites LF to CRLF and `cargo fmt -- --check` fails while Linux/macOS pass.
- Alternatives considered: `newline_style = "Auto"` (lets CRLF leak into the repo); skip fmt on Windows (hides the mismatch).
- Impact: Working tree on Windows stays LF for tracked text files.
- Rollback trigger: None; mixed newlines would fail fmt again.
- Related files: `.gitattributes`, `.github/workflows/ci.yml`

## 2026-10-09 - PairBind direction-specific AEAD nonces
- Decision: Initiator uses nonce `tetherlybndI`, responder `tetherlybndR`. Decrypt with the peer role.
- Rationale: RFC 8439 forbids reusing `(key, nonce)` for two plaintexts. Both peers share one SPAKE2 `pair_key`.
- Alternatives considered: Single nonce `tetherlybnd1` (broken); random nonces prepended to ciphertext (wire change, unnecessary for one-shot bind).
- Impact: Pairing ciphertext is role-specific; wrong-direction decrypt fails.
- Rollback trigger: None; reuse is a crypto break.
- Related files: `crates/tetherly-crypto/src/pairbind.rs`, `docs/DEVELOPMENT.md` ADR-002

## 2026-10-09 - Trust after Noise, not after PairBind
- Decision: Write `TrustStore` only after Noise enters transport. On Noise failure, `remove` the peer. Alias is signed PairBind `device_id`, not Hello `name`.
- Rationale: Hello name/caps MITM would otherwise leave a trusted peer with a poisoned prologue.
- Alternatives considered: Commit after PairBind then rollback (TOCTOU on crash).
- Impact: First-pair Noise reads `n_pk` from Hello when trust is empty; PairBind already authenticated those keys.
- Rollback trigger: Revisit if a durable store is added before handshake completion.
- Related files: `crates/tetherly-net/src/session.rs`

## 2026-10-09 - EasyTier stays sidecar
- Decision: No `easytier*` in Cargo.toml. `cargo deny` bans those crate names and rejects LGPL in the graph.
- Rationale: EasyTier is LGPL-3.0; linking would contaminate Apache-2.0 OR MIT.
- Alternatives considered: Dynamic link / FFI (deferred to a future ADR revision).
- Impact: Cross-net is Phase 2 unicast over user-installed EasyTier.
- Rollback trigger: Written ADR-006 revision only.
- Related files: `deny.toml`, `docs/adr/006-easytier.md`
