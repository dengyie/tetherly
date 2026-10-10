# Phase 5 — Remote screen (screen streaming) CI slice

- Closed: 2026-10-10
- Spec: `docs/DEVELOPMENT.md` v1.7 (Phase 5 opened on owner waiver; encoder route: lossless-first, encoder-later)
- Delivered:
  - `tetherly-core::screen` (OS-free, `#![forbid(unsafe_code)]`, no `cfg(target_os)`): `SCREEN_MAGIC=*b"TMV1"`,
    `SCREEN_PROTO=1`, `SCREEN_SEQ_WINDOW=65536`, `SCREEN_TRACE_MAX=64`, `PixelFormat{Bgra8,Rgba8}`,
    `ScreenRect`, `ScreenFrame` codec (header **31 bytes**: magic + proto + seq + rect + format + compression + stride),
    `ControlMsg{Start,Stop,Ack{seq}}` codec (14 bytes), `ScreenSender` (monotonic seq + consent gate +
    push/drain/refused counters), `ScreenReceiver` (seq window drop-stale/reject-big-jump/bounded trace),
    `ScreenState{Idle,Requested,Allowed,Streaming}`, `ScreenSource`/`ScreenSink` ports,
    `MemoryScreenSource`/`MemoryScreenSink` (deterministic CI fakes).
  - `tetherly-net`: `SCREEN_PORT=45720`, `NetError::ScreenRequiresTrust`, `SessionConfig.screen_only: bool`,
    `hello()` caps advertise `"screen"`; `screen_only` sessions reject `HelloDisposition::Pair`.
  - Mandatory local consent M5.2: `ScreenState` gate — peer connecting only sets `Requested`; host must
    explicitly call `Node::screen_allow()` to reach `Allowed`; `Start` before `Allowed` is refused + counted;
    `screen_revoke()` returns to `Idle`; `on_peer_gone` forces `Idle` + clears seq.
  - `tetherly-node` 14-anchor wiring: `NodeConfig.screen_port` (default 45720) + `screen_source`,
    `Shared.bound_screen`/`screen_out`/`screen_engine`/`screen_sink`/`screen_ctl`/`screen_source`,
    `spawn_listeners`/`spawn_overlay` each add 45720 bind + accept, caps add `"screen"` + `screenport=`,
    `LivePeer.screen_port`, `open_screen`/`accept_screen`/`attach_screen_server`/`attach_screen_client`/
    `screen_send_frame`, public methods `screen_allow/revoke/start/stop/state/stats/send_frame/sink_snapshot/port/addr`.
  - `crates/tetherly-node/src/screen.rs`: `SystemSource`/`SystemSink` stubs (return `ScreenRefused`),
    comments mark DXGI/WGC (Win), core-graphics (mac), x11rb (Linux) as next iteration;
    Wayland honestly marked unsupported.
  - `deny.toml` bans `hbb_common`/`rustdesk`/`rustdesk-server`/`scrap` (AGPL structured red line).
  - UI: `POST /api/screen/allow|start|stop` routes + `ui/index.html`「远程屏幕（实验）」section
    (① local allow required to stream; ② Wayland unsupported; ③ real capture Manual-required).
  - `tests/phase5.rs`: M5.1 consent-then-start frame seq strictly increasing (vec![1,2,3,4,5]),
    M5.2 Start-before-allow refused + refused counter increments, M5.3 peer gone → `Idle`
    resets seq, M5.4 45720 refuses unpaired dial (`ScreenRequiresTrust`) — 4 tests, all green.
- Validation: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --locked` (loopback 10, phase1 7, phase2 4, phase3 4, phase4 10,
  phase5 4, core 112, crypto 10, net 1, node 4), `cargo deny check` all ok.
- Manual-required: real DXGI Desktop Duplication / WGC (Win), CGImage (macOS), X11 SHM (Linux)
  capture; real encoder (OpenH264/rav1e); real window presentation (Win HWND BitBlt / macOS
  CGWindow / Linux portal); p95 frame-interval budget on hardware. M5.5 hardware encoder /
  real injection not started.
- Stop: Do not claim real remote desktop is usable. RustDesk AGPL code not linked (deny.toml enforces).
  Encoder route is lossless-first, swappable port later. Phase 6 is a separate single-item立项.