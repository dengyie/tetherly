# Phase 4 — Computer-side iPhone ANCS CI slice

- Closed: 2026-10-10
- Spec: `docs/DEVELOPMENT.md` v1.6 (v1.5's "do not open Phase 4" gate waived by the project owner)
- Delivered:
  - `tetherly-core::ancs` (OS-free): canonical UUID text constants + LE decoder, 8-byte
    `NotificationEvent` parser (`EventId` / `EventFlags` / `Category` with unknown-value
    tolerance), Control Point encoders (`GetNotificationAttributes`, `GetAppAttributes`,
    `PerformNotificationAction`), `DataAssembler` byte-wise fragment reassembly,
    `AncsIngress` state machine, `AncsTransport` port, `MemoryAncsTransport`,
    `ancs_actions(app_id, &OpenAllowlist)`, `ancs_source_id(peripheral)`.
  - §9.4 constraints enforced structurally: `connect()` subscribes Data Source before
    Notification Source; `on_notification_source` only enqueues while `tick()` writes the
    Control Point; `in_flight` keeps the Control Point serial; one retry of a silent first
    response then abandon; `on_disconnected` backoff 500ms → 30s cap; uid dedup plus
    `PreExisting` suppression so a reconnect never re-pops.
  - `tetherly-node`: `NodeConfig.ancs` (`Option<Arc<dyn AncsTransport>>`) + `ancs_peripheral`,
    `Node::ancs_connect/ancs_notification_source/ancs_data_source/ancs_tick/ancs_disconnected/ancs_state`,
    `set_open_allowlist`/`add_open_rule`/`open_allowlist`, `open_candidate(id, &dyn Opener)`.
    `Opener` port with `SystemOpener` (no shell) and `MemoryOpener`. `Candidate`/`CandidateView`/
    `CandidateViewDto` now carry `actions`.
  - UI: pairing wizard split into two steps (① OS-Bluetooth ANCS, ② SPAKE2 pairing); routes
    `/api/ancs/connect`, `/api/ancs/allow`, `/api/open`; snapshot exposes `ancs_state`/`open_apps`.
  - `tests/phase4.rs`: M4.1 simulated latency budget, M4.1 held-notification cap, M4.2
    allowlist-gated open + payload url never opened, M4.3 reconnect no re-pop, serial CP and
    no write on the callback thread, cross-UTF-8 fragment reassembly, `Removed` clears the
    candidate, no-transport inertness, backoff refuses a hot reconnect.
- Validation: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --locked` (loopback 10, phase1 7, phase2 4, phase3 4, phase4 10,
  core 93, crypto 10, net 1, node 4), `cargo deny check` all ok.
- Manual-required: real BLE GATT (Windows `windows` crate / macOS CoreBluetooth / Linux BlueZ),
  physical iPhone p95 < 2s, Bluetooth-off-30s-then-on real-device reproduction.
- Not started: M4.4 / M4.5 optional iOS App (same-LAN 50MB sha256, no VPN / Network Extension).
- Stop: Do not claim M4.1/M4.3 device verification. Phase 5 is a separate single-item立项.