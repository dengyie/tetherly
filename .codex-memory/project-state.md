# Project State

## Objective
- Personal LAN-first device mesh (Tetherly): pairing + Noise, local OTP, EasyTier sidecar only;
  computer-side ANCS ingress for iPhone notifications (Phase 4 CI slice landed);
  remote screen streaming — Phase 5 CI slice + Windows real-machine loop landed (v1.8);
  Phase 3 key/mouse real-machine chain landed (v1.9);
  **Phase 1 Android native library landed (v1.10)** — real Ed25519 + Noise session `.so`,
  verified on two physical phones;
  **`notify.reply` quick reply landed (v1.11)** — desktop→phone, deliver-only;
  **file cross-process resume landed (v1.12)** — content-addressed id + TFL2 start offset + `.part`/`.resume.json` sidecar;
  **Windows credential-vault identity landed (v1.13)** — Credential Manager generic credential + `identity.ref`, file fallback kept;
  **macOS Keychain / Linux libsecret backends landed (v1.14)** — backend-per-platform `cred/*.rs`, compile-verified per target, runtime Manual-required;
  **peer-input hardening (v1.15)** — `ReplayGuard` msg_id overflow + `ScreenAssembler` fragment-idx underflow fixed (both remote-triggerable), +3 regression tests, 205 green;
  **proptest + fuzz layers landed (v1.16)** — §13's two declared-but-missing layers now real on the stable toolchain: decoder never-panic + round-trip properties (core 27, net 7) and a deterministic xorshift mutation corpus driver (core 2); no decoder bug found (v1.15 fixes hold), 251 green;
  RustDesk architecture borrow only, AGPL not linkable.

## Current Phase
- Spec `docs/DEVELOPMENT.md` **v1.16**. Phase 5 real-machine loop + Phase 3 real-machine chain +
  Phase 1 Android native lib + `notify.reply` quick reply + file cross-process resume + Windows credential-vault identity storage + macOS Keychain / Linux libsecret backends + **§13 proptest/fuzz layers** all landed.
- **§13 proptest + fuzz layers (v1.16)**: `tetherly-core/tests/decoders.rs` (27 props) — every inbound decoder never panics on arbitrary input, every encodable message round-trips (notify push/dismiss/reply, clip set, file offer/decision/done, caps update, input event, screen frame, control msg, hello, JSON control-plane); plus sanitize_file_name stays a basename (non-empty, != "."/"..", no '/', no NUL), DataAssembler bounded to 8 KiB, any-order reassembly. `tetherly-net/tests/decoders.rs` (7 props) — `read_len_prefixed`/`read_tagged`/`read_header` never panic, oversized length prefix refused (anti-DoS), valid headers round-trip, `TFL1` reads as `start=0`, wrong magic refused. `tetherly-core/tests/fuzz_decoders.rs` (2) — deterministic xorshift mutator over a valid corpus (inner frames, all 7 control JSON types, all 7 input kinds, whole+fragmented screen frames, control msgs, ANCS source events + data-source tuples, canonical Hello) applying 6 mutation kinds (bit flip, extreme byte, truncate, extend, multi-byte overwrite, splice), feeding every decoder under `catch_unwind`; fixed seed `0x9E3779B97F4A7C15`, 512 mutations/seed, every 4th stacked. `proptest = "1"` (MIT) added as a workspace dev-dependency.
- **Honest boundary**: libfuzzer / `cargo-fuzz` (nightly) remains Manual-required; the deterministic mutation driver is the stable-toolchain substitute (documented in the spec §13 row).
- **`notify.reply` (v1.11)**: `tetherly-core::frame::NotifyReply{uid,app_id,text}` + `NOTIFY_TEXT_MAX=4 KiB`
  (UTF-8-safe truncate); `TYPE_NOTIFY_REPLY=0x0103`. `tetherly-net` `SessionEvent::ReplyStub` replaced by
  `SessionEvent::Reply(NotifyReply)`. `NotifyHub` reply inbox (`ingest_reply`/`take_reply_id`/`take_reply`/
  `reply_views`, key = `reply_id(source,app_id,uid)` sha256, **take-once**, blank rejected, last-write-wins,
  `QueuedReply` Debug redacts text; `ReplyView` carries no text for list views). `Node::{send_reply,
  reply_to_candidate,take_reply,reply_views}` + `UiEvent::ReplyReceived` (logs text_len only). UI reply box +
  `/api/reply` + `/api/reply/take` + `Snapshot.replies`. Android `SessionHandle::take_replies` +
  `nativeTakeReplies` (JSON) + Kotlin `takeReplies`/`PhoneReply` + `NotifyListener.drainReplies` (fires the
  notification's OWN `RemoteInput` action; never synthesises an app's reply). CI: 3 core units +
  `phase2::m2_6` (two-node real session) + `client_e2e::android_client_receives_a_quick_reply_once`.
- **File cross-process resume (v1.12)**: `transfer_id` content-addressed (`"tr_"+hex(sha256(file))[..16]`); wire
  gains **`TFL2`** (`magic|id|total u64be|start u64be`), `read_header` also accepts `TFL1`(start=0). `tetherly-core::filexfer`
  `ResumeState`/`ResumeTable` (`claim(source,id,total)` → aligned offset or `None`; `put` aligns down); `IncomingTransfer.resume_from`;
  `FileHub::{offer_resumable,accept_at}`; `FileDecision.resume_from` + `accept_resuming`; `CoreError::ResumeOffset`; `FILE_CHUNK=64 KiB`.
  net `send_chunks_from`/`recv_chunks_after_header`/`resumable_offset`, per-chunk AAD `id||chan_offset_le`, nonce `offset/CHUNK+1`
  (so resume MUST be 64 KiB-aligned and a chunk cannot be relocated). node: incremental `inbox/.<transfer_id>.part` write
  (`sync_all` + `on_chunk`), `note_resume` sidecar on interrupt, `load_resume_table` startup scan (drops torn sidecars), tail-only
  send from `resume_from`. CI: core 2 units + net 4 units + `tests/phase1::m1_8_file_resumes_from_persisted_offset` (restart scan).
- **NEW crate `crates/tetherly-android` (v1.10)**: lib `tetherly_android`, crate-type `["cdylib","rlib"]`;
  `cdylib` → `libtetherly_android.so` (arm64-v8a via cargo-ndk), `rlib` → host/CI integration test.
  deps `tetherly-core`/`tetherly-crypto`/`tetherly-net` (`default-features=false`)/`tokio`/`serde_json`/
  `thiserror`/`tracing`/`zeroize`; `jni` gated `[target.'cfg(target_os="android")'.dependencies]`.
  `#![deny(unsafe_code)]` at lib root; only `src/jni.rs` carries `#![allow(unsafe_code)]` (single justified
  exemption — JNI ABI).
  - `store.rs`: `identity.bin` (64 bytes, temp+rename, 0600), `trusted.json` (SAME schema as desktop node,
    files interchangeable), `atomic_write`, `load_or_create_identity`, `load_trust`, `save_trust`.
  - `client.rs`: `AndroidClient::load_or_create` + `start(addr, pin, once) -> SessionHandle` spawns a
    "tetherly-android" thread with a `current_thread` tokio runtime; dials 45717 via the SAME `dial_session`
    as desktop (SPAKE2 8-digit PIN first run, Noise IK resume after). `SessionHandle`:
    `notify()`/`state_code()`/`connected_peer()`/`last_error()`/`stop()`. `PHONE_CAPS=["notify","clip","file"]`
    (never input/screen). Backoff 250ms→5s; `TYPE_PING`→pong.
  - `jni.rs`: `Java_app_tetherly_NativeBridge_nativeAttach|SendNotify|DeviceId|IdPk|State|LastError|Stop`
    with `<'local>` JNIEnv lifetimes. Kotlin `NativeBridge`/`NodeSession`/`MainActivity` now use the real
    device_id and drive `notify.push`.
  - `src/bin/android_probe.rs`: pushed to phone, run from `adb shell` — real on-device pair+notify proof.
  - `tetherly-net` gained `SessionConfig.caps: Option<Vec<String>>` (explicit Hello cap override; default
    `None` = existing inference) — phone sets `Some(PHONE_CAPS)`; all 8 SessionConfig literal sites updated.
- `tetherly-core::screen` — OS-free `TMV1` screen frame + `ControlMsg{Start,Stop,Ack}` codec,
  `ScreenSender` (monotonic seq + consent gate + push/drain/refused), `ScreenReceiver`
  (seq window, drop-stale, reject-big-jump, bounded trace), `ScreenState{Idle,Requested,Allowed,Streaming}`,
  `ScreenSource`/`ScreenSink` ports + `MemoryScreenSource`/`MemoryScreenSink` (deterministic CI fakes).
- New TCP port **45720** (`SCREEN_PORT`), resume-only Noise; `screen_only` sessions get
  `NetError::ScreenRequiresTrust` on `HelloDisposition::Pair` — pairing stays on 45717 only.
- Mandatory local consent (M5.2): a peer dial reaching 45720 only sets `Requested`; the host must
  explicitly call `Node::screen_allow()` before `Start` is honored; `Start` before `allow` is refused
  and counted. `screen_revoke()` returns to `Idle`; `on_peer_gone` forces `Idle` + clears seq.
- `tetherly-node`: `NodeConfig.screen_port` (default 45720) + `screen_source`/`screen_codec`/`screen_sink`
  (`Option<Arc<dyn ...>>`; tests inject `MemoryScreenSource`); `Shared` gains
  `bound_screen`/`screen_out`/`screen_engine`/`screen_observer`/`screen_ctl`/`screen_source`/`screen_codec`/
  `screen_sink`/`screen_interval` (`ScreenIntervalTracker`, p95);
  `spawn_listeners`/`spawn_overlay` each add a 45720 bind + accept; caps advertise `"screen"` + `screenport=`;
  public surface `screen_allow/revoke/start/stop/state/stats/send_frame/sink_snapshot/port/addr/latency_p95_us`.
- `crates/tetherly-node/src/screen.rs`: **v1.8 real-machine loop** — `DxgiSource` (DXGI Desktop Duplication,
  `AcquireNextFrame` WAIT_TIMEOUT → `Ok(None)`), `ZstdCodec` (lossless, level 3) + `EncodedSource<S>`
  decorator (emit raw if compressed not smaller), `SystemSink` (GDI `CreateDIBSection` 32bpp top-down +
  synchronous `BitBlt`); non-Windows returns honest `ScreenRefused`. macOS core-graphics/x11rb next iteration;
  Wayland honestly marked unsupported. Production default `EncodedSource<SystemSource>` (DXGI+zstd);
  `tetherly run` injects `SystemSink` + 16ms frame pump (skips grab unless Streaming).
  `AdbSource` (portable): ONE persistent device-side stream — default length-prefixed PNG loop
  (`screencap -p > tmp; wc -c; cat`, pid-suffixed tmp), `TETHERLY_ADB_RAW=1` raw RGBA for fast links
  (overlay adb leg ~187KB/s: raw 3.7MB/frame=17.5s vs PNG 31KB=0.17s, so PNG is default), auto-fallback
  raw→PNG; byte-identical frame → `Ok(None)`; selected via `tetherly run --screen-adb <serial>`
  (TETHERLY_ADB overrides adb path). Perf pass 2026-10-11: persistent zstd bulk contexts,
  offset-based `ScreenAssembler` (-57% reassembly), single-copy `SystemSink::present`, `[profile.release]`
  lto=thin/codegen-units=1; bench: `cargo run -p tetherly-node --example bench_screen --release`.
- `deny.toml` bans `hbb_common`/`rustdesk`/`rustdesk-server`/`scrap` (AGPL) — structured red line
  against linking RustDesk code. RustDesk architecture borrow only (separated capture + swappable
  encoder + P2P); its rendezvous (hbbs/hbbr) model is NOT adopted (matches §2.3 no-cloud-account).
- Encoder route landed: lossless frames first (`ZstdCodec` + `EncodedSource`, level 3), encoder later as a
  swappable `ScreenCodec` port impl (OpenH264/rav1e are permissive; x264/x265 GPL out).
- Fragmentation + reassembly: `ScreenFrame::to_frames()` splits over-64 KiB frames into `TMF1` fragments
  (header 24 bytes); `ScreenAssembler::push` reassembles (reset on stream change, single-stream memory cap
  `SCREEN_PAYLOAD_MAX`); client decompresses via `screen_codec.decompress(expected_len)` before seq window.
- p95 budget: `ScreenIntervalTracker` (wall-clock µs, bounded 1024), `Node::screen_latency_p95_us()`,
  UI `Snapshot.screen.p95_frame_interval_us`.
- `tests/phase5.rs`: M5.1 consent-then-start frame seq strictly increasing, M5.2 Start-before-allow
  refused + counted, M5.3 peer gone → `Idle` resets seq, M5.4 45720 refuses unpaired dial — 4 tests green.
- UI: `/api/screen/allow|start|stop` routes + 「远程屏幕（实验）」 section in `ui/index.html`
  (① local allow required to stream; ② Wayland unsupported; ③ Windows real capture/encode/present available;
  macOS/Linux next iteration).
- **Manual-required (narrowed)**: DXGI-side pixel capture on an interactive desktop (this agent
  shell's static desktop never flips DXGI); **Windows DXGI cross-desktop** p95 frame-interval data;
  real open encoders OpenH264/rav1e; macOS CGImage/CGWindow, Linux X11 SHM. M5.5 (hardware encode /
  real injection) not started.
- Phase 3 real-machine chain (v1.9): SystemInputSink (SendInput) + controller LL-hook capture
  (keyboard hook seat-gated, injected events filtered) landed and probe-verified (cursor really
  moves across the edge). Remaining Manual: dual-desktop 100 physical round-trips.
- Real BLE GATT + physical iPhone p95 remain Manual-required (Phase 4).

## Current Branch
- `main` @ `https://github.com/dengyie/tetherly`
- Branch commit will be created after this session.

## Last Verified
- Local: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace --locked` all green — **251 passed, 0 failed** (v1.16: +27 core props,
  +7 net props, +2 fuzz drivers over the prior 205).
- Cross-check: `cargo clippy --target x86_64-unknown-linux-gnu --workspace --all-targets --locked -- -D warnings`
  clean (needs `CC_x86_64_unknown_linux_gnu=gcc AR_...=ar` since Phase 5's `zstd-sys` wants a target C
  compiler that isn't installed; check-only, nothing links). macOS cross-check not possible on this host.
- `cargo deny check` → advisories ok, bans ok, licenses ok, sources ok (proptest 1.x MIT passes the allow-list).
- `cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build -p tetherly-android --release` →
  `libtetherly_android.so` (aarch64 ELF, NDK r29, ~1.6 MB), all 7 `Java_app_tetherly_*` symbols exported.
- Review hardening (2026-10-11): `ScreenSource::interrupt` port method + 10s grab timeout in
  `spawn_blocking` (kills wedged adb streams), pump warn+backoff instead of permanent break,
  viewer error arms logged, GDI DIB/DC leak fixed on reinit, present window on a pumped thread
  (live-verified resize to frame size), AdbSource parse functions unit-tested, exact zstd bound.
- `cargo deny check licenses bans sources` (easytier* still banned; AGPL screen crates banned;
  no easytier/rustdesk/hbb_common/scrap in Cargo.toml; `jni` permitted).
- Host: rustc/cargo 1.99.0, `x86_64-pc-windows-gnu`, Scoop mingw 16.2.0; NDK 29.0.13599879.
- **On-device E2E (v1.10, real hardware)**: desktop node on host overlay `10.144.144.10:45717`;
  two Galaxy Fold (SM-F900F, arm64-v8a, Android 13) run `android_probe` over `adb reverse`
  (`adb reverse tcp:45717 tcp:45717` — phones have no route to the host overlay, only adb access).
  - phone 1 `tdev_wvxo7aijpbbnasfq` paired with PIN → desktop got the `notify.push`
    (`candidates` shows `has_otp:true`, body NOT exposed in snapshot/logs); both `trusted.json` written.
  - Re-run with a deliberately WRONG PIN still connected → proved Noise IK resume, not re-pairing.
  - phone 2 `tdev_q4mpa2wypdwuacpa` paired on a fresh PIN; desktop now trusts two phones, both notifies received.
- Real-machine (v1.8): on this Windows host (real DISPLAY1 1920x1080), `DxgiSource` initializes
  (D3D11 → adapter → output → DuplicateOutput) and returns `Ok(None)` (WAIT_TIMEOUT) on an idle
  desktop; `SystemSink` GDI present succeeds (60/60). Actual DXGI pixel capture was not exercised in
  this agent shell (no interactive desktop changes DXGI would flip); DXGI-side capture + cross-desktop
  p95 remain Manual-required.
- Real-machine cross-device E2E (v1.8 `AdbSource`): cloud phone (Galaxy Fold 720x1280, adb via
  EasyTier overlay) → node A (`tetherly run --screen-adb 10.144.144.51:5555`) → zstd/TMF1/45720 →
  node B (`SystemSink` GDI): 1063 sent / 1060 received / 0 dropped (~20 min run), frames decompress
  to exactly 720×1280×4 BGRA, GDI window pixels 97.6% match within ±8 vs fresh device screenshot
  (residual = loading-animation phase + DWM color management), p95 ≈0.9–1.45s (adb screencap
  source-rate-bound, honestly measured), M5.2 refused=1 observed live. Two real gaps found+fixed:
  `/api/screen/start` auto-dials the screen channel; visitor rx counters (`screen_rx_stats`) now in Snapshot.

## Active Risks
- Physical Android M1.1 **p95 timing** and 8h soak are still Manual-required (need a real
  notification → visible-window timing harness). The `.so`-missing blocker is GONE (v1.10).
- Desktop UI is loopback HTTP, not Tauri.
- Host may already run EasyTier `10.144.144.0/24`; tests must isolate with a non-default CIDR (`10.199.199.0/24`).
- Cloud phones reach the host only via adb (overlay is host→phone); on-device E2E uses `adb reverse`.
- `snow` 0.9 has no `rekey()`; sessions **rebuild a fresh IK handshake in band** when the transport hits 2^16 msgs or 3600s (initiator-driven, cancellation-safe; `RekeyRequired` is retired).
- Screen `seq` must stay session-monotonic; `on_peer_gone` resets it and a reconnect must not replay old frames.
- Phase 4 real BLE transport does not exist yet; ANCS ingress is untested on Windows/macOS/Linux GATT drivers.
- AGPL/GPL screen-capture crates are banned by deny.toml; only permissive impls (windows, core-graphics, x11rb) are eligible.

## Active Blockers
- Physical Android M1.1 p95 + 8h soak (Manual-required; `.so` now built, need timing harness + long soak).
- True SendInput / macOS / Wayland portal+libei and 100 physical dual-desktop round-trips (Manual-required).
- Physical Android cellular + user EasyTier for true M2.1 p95 (Manual-required).
- Real iPhone ANCS p95 < 2s and Bluetooth-off-30s-and-on verification (Manual-required).
- Windows cross-desktop p95 measurement + OpenH264/rav1e + macOS/Linux screen loop + dual-desktop 100 physical input round-trips (Manual-required, Phases 3+5 narrowed). Cross-device (cloud-phone AdbSource) screen loop, single-machine key/mouse injection, and on-device Android pair/notify/resume are verified.

## Current Focus
- v1.16 (SSOT §13 proptest + fuzz layers + SSOT/memory sync) complete and fully green locally;
  awaiting an explicit user instruction before any commit/push.

## Next Milestone
- Phase 6 (single-item): Android controlled device, WinFsp mount, file persistent resume,
  Linux ANCS experience, embedded EasyTier FFI. (`notify.reply` closed v1.11; removed from backlog.)

## Key Artifacts
- `docs/DEVELOPMENT.md` (SSOT v1.16)
- `crates/tetherly-{core,crypto,net,node,android}` + `crates/tetherly-core/src/{ancs,screen}.rs`
  + `crates/tetherly-node/src/screen.rs` + `crates/tetherly-android/src/{store,client,jni}.rs`
- `crates/tetherly-core/tests/{decoders,fuzz_decoders}.rs`, `crates/tetherly-net/tests/decoders.rs`
- `bins/tetherly-cli`, `tests/{loopback,phase1,phase2,phase3,phase4,phase5}.rs`, `ui/`, `android/`
- GitHub: `dengyie/tetherly`
- Vault: `Note/Project/tetherly/tetherly 开发文档.md`
