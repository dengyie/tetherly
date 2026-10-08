# Decisions

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
