# Decisions

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
