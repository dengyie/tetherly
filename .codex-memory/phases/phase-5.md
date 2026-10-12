# Phase 5 — Remote screen (screen streaming) CI slice

- Closed: 2026-10-10 (CI slice) / 2026-10-11 (real-machine loop)
- Spec: `docs/DEVELOPMENT.md` v1.8 (Phase 5 opened on owner waiver; encoder route: lossless-first, encoder-later;
  v1.8 added DXGI Desktop Duplication capture + zstd lossless encode + GDI BitBlt present + p95 budget on Windows)
- Delivered:
  - **CI slice (2026-10-10)**:
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
  - **Real-machine loop (v1.8, 2026-10-11)**: `DxgiSource` (DXGI Desktop Duplication; `D3D11CreateDevice(BGRA)` →
    `IDXGIDevice::GetAdapter` → `IDXGIOutput1::DuplicateOutput`; `AcquireNextFrame(0)` WAIT_TIMEOUT → `Ok(None)`;
    staging texture + `Map(READ)` + row-tail trim → packed BGRA8), `ZstdCodec` (lossless level 3, `ScreenCodec` port)
    + `EncodedSource<S>` (per-frame compress, raw fallback when not smaller), `SystemSink` (GDI `CreateDIBSection`
    32bpp top-down + synchronous `BitBlt`, no message pump); `ScreenFrame::to_frames()` + `ScreenAssembler::push`
    fragmentation/reassembly; client decompress via `screen_codec.decompress(expected_len)`; `ScreenIntervalTracker`
    (wall-clock µs, bounded 1024) with `Node::screen_latency_p95_us()`; `tetherly run` injects `SystemSink` + 16ms pump;
    UI `Snapshot.screen.p95_frame_interval_us`.
- Validation: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --locked` (loopback 10, phase1 7, phase2 4, phase3 4, phase4 10,
  phase5 4, core 116, crypto 10, net 1, node 4), `cargo deny check` all ok.
  Real-machine (v1.8): DXGI device init + WAIT_TIMEOUT→None idle path + GDI present (60/60) verified; DXGI pixel capture not exercised (agent shell, no interactive desktop flip).
  Cross-device real E2E (v1.8 `AdbSource`): cloud phone (Galaxy Fold 720x1280, adb) → node A
  (`--screen-adb`) → zstd/TMF1/45720 → node B (SystemSink GDI) — 1063 sent / 1060 received /
  0 dropped (~20 min), frames decompress to exactly 720×1280×4 BGRA, window pixels 97.6% within ±8
  (residual = animation phase + DWM color management), p95 ≈0.9–1.45s (adb source-rate-bound).
  Live M5.2 refusal observed. Two UI gaps fixed: `/api/screen/start` auto-dials the screen channel;
  visitor `screen_rx_stats` in Snapshot.
  Perf pass (same day): persistent zstd bulk contexts (1080p enc 16.1→12.6ms, dec 4.7→4.1ms), offset
  `ScreenAssembler` (3.0→1.28ms), single-copy `SystemSink::present`, release LTO profile; `AdbSource`
  persistent length-prefixed PNG stream (grab 543→436ms; raw 3.7MB/frame=17.5s on the overlay vs
  PNG 31KB=0.17s — PNG default, `TETHERLY_ADB_RAW=1` opt-in). Bench example `bench_screen` kept.
- Manual-required (narrowed): DXGI-side pixel capture (needs interactive desktop); **Windows DXGI cross-desktop** p95 data; real open encoder
  (OpenH264/rav1e) as `ScreenCodec` port; macOS CGImage/CGWindow, Linux X11 SHM (Wayland portal).
  M5.5 hardware encoder / real injection not started.
- Stop: Do not claim Windows DXGI cross-desktop remote desktop is usable (DXGI init + idle + present verified; the cross-device AdbSource loop IS verified; DXGI pixel capture and cross-desktop p95 still Manual-required). RustDesk AGPL code not linked (deny.toml enforces).
  Encoder route is lossless-first, swappable port later. Phase 6 is a separate single-item立项.