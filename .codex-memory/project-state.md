# Project State

## Objective
- Personal LAN-first device mesh (Tetherly): pairing + Noise, local OTP, EasyTier sidecar only;
  computer-side ANCS ingress for iPhone notifications (Phase 4 CI slice landed);
  remote screen streaming (Phase 5 CI slice landed) — RustDesk architecture borrow only, AGPL not linkable.

## Current Phase
- Phase 5 remote screen (screen streaming) CI slice landed. Spec `docs/DEVELOPMENT.md` **v1.7**.
- `tetherly-core::screen` — OS-free `TMV1` screen frame + `ControlMsg{Start,Stop,Ack}` codec,
  `ScreenSender` (monotonic seq + consent gate + push/drain/refused), `ScreenReceiver`
  (seq window, drop-stale, reject-big-jump, bounded trace), `ScreenState{Idle,Requested,Allowed,Streaming}`,
  `ScreenSource`/`ScreenSink` ports + `MemoryScreenSource`/`MemoryScreenSink` (deterministic CI fakes).
- New TCP port **45720** (`SCREEN_PORT`), resume-only Noise; `screen_only` sessions get
  `NetError::ScreenRequiresTrust` on `HelloDisposition::Pair` — pairing stays on 45717 only.
- Mandatory local consent (M5.2): a peer dial reaching 45720 only sets `Requested`; the host must
  explicitly call `Node::screen_allow()` before `Start` is honored; `Start` before `allow` is refused
  and counted. `screen_revoke()` returns to `Idle`; `on_peer_gone` forces `Idle` + clears seq.
- `tetherly-node`: `NodeConfig.screen_port` (default 45720) + `screen_source: Option<Arc<dyn ScreenSource>>`
  (tests inject `MemoryScreenSource`); `Shared` gains `bound_screen`/`screen_out`/`screen_engine`/`screen_sink`/`screen_ctl`/`screen_source`;
  `spawn_listeners`/`spawn_overlay` each add a 45720 bind + accept; caps advertise `"screen"` + `screenport=`;
  public surface `screen_allow/revoke/start/stop/state/stats/send_frame/sink_snapshot/port/addr`.
- `crates/tetherly-node/src/screen.rs`: `SystemSource`/`SystemSink` return `ScreenRefused`
  (real capture/present is Manual-required); comments mark DXGI/WGC (Win), core-graphics (mac),
  x11rb (Linux) as next iteration; Wayland honestly marked unsupported.
- `deny.toml` bans `hbb_common`/`rustdesk`/`rustdesk-server`/`scrap` (AGPL) — structured red line
  against linking RustDesk code. RustDesk architecture borrow only (separated capture + swappable
  encoder + P2P); its rendezvous (hbbs/hbbr) model is NOT adopted (matches §2.3 no-cloud-account).
- Encoder route decided: lossless frames first (`SCREEN_COMPRESS_NONE` now), encoder later as a
  swappable `ScreenSource` port impl (OpenH264/rav1e/zstd are permissive; x264/x265 GPL out).
- `tests/phase5.rs`: M5.1 consent-then-start frame seq strictly increasing, M5.2 Start-before-allow
  refused + counted, M5.3 peer gone → `Idle` resets seq, M5.4 45720 refuses unpaired dial — 4 tests green.
- UI: `/api/screen/allow|start|stop` routes + 「远程屏幕（实验）」 section in `ui/index.html`
  (① local allow required to stream; ② Wayland unsupported; ③ real capture is Manual-required).
- **Manual-required**: real DXGI Desktop Duplication / WGC (Win), CGImage (macOS), X11 SHM (Linux)
  capture; real encoders (OpenH264/rav1e); real window presentation; p95 frame-interval budget on
  hardware. M5.5 (hardware encode / real injection) not started.
- Physical dual-desktop and OS cursor/key injection remain Manual-required (Phase 3).
- Real BLE GATT + physical iPhone p95 remain Manual-required (Phase 4).

## Current Branch
- `main` @ `https://github.com/dengyie/tetherly`
- Branch commit will be created after this session.

## Last Verified
- Local: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --locked` (loopback 10, phase1 7, phase2 4, phase3 4, phase4 10,
  phase5 4, core 112, crypto 10, net 1, node 4).
- `cargo deny check licenses bans sources` (easytier* still banned; AGPL screen crates banned;
  no easytier/rustdesk/hbb_common/scrap in Cargo.toml).
- Host: rustc/cargo 1.99.0, `x86_64-pc-windows-gnu`, Scoop mingw 16.2.0.

## Active Risks
- Android without `libtetherly_android.so` cannot complete SPAKE2/Noise; placeholder `idPk = sha256(idSk)` is not Ed25519.
- Desktop UI is loopback HTTP, not Tauri.
- Host may already run EasyTier `10.144.144.0/24`; tests must isolate with a non-default CIDR (`10.199.199.0/24`).
- `snow` 0.9 has no `rekey()`; sessions return `RekeyRequired` after 2^16 msgs or 3600s.
- Screen `seq` must stay session-monotonic; `on_peer_gone` resets it and a reconnect must not replay old frames.
- Phase 4 real BLE transport does not exist yet; ANCS ingress is untested on Windows/macOS/Linux GATT drivers.
- AGPL/GPL screen-capture crates are banned by deny.toml; only permissive impls (windows, core-graphics, x11rb) are eligible.

## Active Blockers
- True SendInput / macOS / Wayland portal+libei and 100 physical dual-desktop round-trips (Manual-required).
- Physical Android cellular + user EasyTier for true M2.1 p95 (Manual-required).
- Real iPhone ANCS p95 < 2s and Bluetooth-off-30s-and-on verification (Manual-required).
- Real screen capture / encoder / presentation hardware verification (Manual-required, Phase 5).

## Current Focus
- Push Phase 5 CI slice to main, confirm CI green. Then documentation and stability.

## Next Milestone
- Phase 6 (single-item): Android controlled device, notify.reply, WinFsp mount, file persistent resume,
  Linux ANCS experience, embedded EasyTier FFI. Pending prioritisation.

## Key Artifacts
- `docs/DEVELOPMENT.md` (SSOT v1.7)
- `crates/tetherly-{core,crypto,net,node}` + `crates/tetherly-core/src/{ancs,screen}.rs` + `crates/tetherly-node/src/screen.rs`
- `bins/tetherly-cli`, `tests/{loopback,phase1,phase2,phase3,phase4,phase5}.rs`, `ui/`, `android/`
- GitHub: `dengyie/tetherly`
- Vault: `Note/Project/tetherly/tetherly 开发文档.md`
