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
  **stateful/model-based property tests landed (v1.17)** — §13's property layer extended from single decoders/messages to sequence-driven state machines: `tetherly-core/tests/state_machines.rs` (11 proptest + 7 deterministic witnesses) drives `ReplayGuard`/`TokenBucket`/`NotifyLimiter`/`ClipLimiter`/`NotifyHub`/`FileHub`/`ResumeTable`/`ScreenSender`/`ScreenReceiver`/`ScreenAssembler`/`AncsIngress` with arbitrary byte op-programs, asserting the safety contract after every step; no new bug found (v1.15 fixes hold at the state level), 269 green;
  **real-device APK end-to-end closed (v1.18)** — the shipped `app.tetherly` front-end `NotificationListenerService` → JNI → `libtetherly_android.so` live Noise session → desktop candidate `{app_id:"com.android.shell", has_otp:true, source:tdev_...}` (body never in snapshot/logs), closing v1.10's "APK Kotlin→JNI→session UI path still manual" gap; on-device M1.1 notify round-trip **p95 measured** (`p50≈17.9ms / p95≈56.9ms / p99≈361.8ms` over the `adb reverse` cloud-phone tunnel; LAN same-model ~15–16ms), inside the <1.0s budget; **`tetherly pin` defect fixed** (it used to generate a PIN in a throwaway `Node::start` process, so no listener could ever accept it — now a loopback client of the running daemon via `tetherly_node::loopback_post("/api/pin")`, honest non-zero exit when no daemon is up) + regression test, 270 green;
  **macOS/Linux screen-capture backends landed (v1.19)** — the gap §9.7/§14 had long marked
  "next iteration" is closed: `screen/macos.rs` (`MacSource`, CoreGraphics `CGDisplayCreateImage`
  into an explicit BGRA8 `CGBitmapContext`, 16-byte-aligned stride requested then the framework's
  actual stride de-padded; Screen-Recording denied → honest `ScreenRefused`, not a blank frame;
  present needs AppKit `NSWindow` and is honestly refused) and `screen/x11.rs` (`X11Source`, MIT-SHM
  fast path + plain `GetImage` fallback, `shm_query_version >= 1.2` gate, reads exactly `reply.size`;
  present = override-redirect window + `PutImage` with the depth read from the root, not hardcoded).
  Each is a self-contained file (absolute imports only) per the `cred.rs` precedent; target-deps added
  (macOS `objc2-core-*` default-features=false, Linux `x11rb` shm + `libc`) with **no new packages**
  (all already in the lockfile via `arboard`). Verified only by per-target `clippy -D warnings`
  type-check on a throwaway probe (whole `screen.rs` + a stub `zstd`); 7+7 backend unit tests are
  pure logic and run natively in CI. Real capture (needs a display/permission), macOS present,
  Wayland portal, and the open encoder remain Manual-required.
  **CLI control plane moved to live-node loopback clients (v1.20)** — the cross-VPS test exposed two
  more instances of the v1.18 `pin` defect class: `tetherly dial` started a second `Node::start` that
  collided with the running daemon on the same LAN ports ("no LAN address bound"), and
  `tetherly inject-notify` only fed a fake push into its own process (never over the Noise session,
  so it could never reach a peer). Both are now loopback clients of the running node: `dial` →
  `POST /api/dial` (`--ui-port`, default 45716), `InjectNotify` deleted and replaced by `Cmd::Notify`
  → `POST /api/notify` (single live peer implied; multiple peers need an explicit `--peer`; unknown/
  offline peer → 400; body delivered, never executed, never logged; `.truncate()` gate shared with the
  inbound path). The loopback API refuses non-loopback callers via its existing `Host` check. The
  release workflow anchors Linux binaries to **glibc 2.35** (build matrix `ubuntu-latest` →
  `ubuntu-22.04` + an `objdump` guard that fails the build on `GLIBC_2.36+`), fixing the 24.04-built
  binary that refused to start on glibc 2.31 VPS hosts. Verification: fmt / clippy `-D warnings` /
  **271 tests** (phase1 9→10, adding `notify_posted_to_the_live_node_api_reaches_the_paired_peer`) /
  deny all green.
- **Session liveness + backpressure + glibc 2.31 release anchor (v1.21)**: the cross-VPS test proved
  v1.20 notify works (an OTP push reached the peer's candidate list) and then found that a
  **silently black-holed** TCP path makes every later `notify` report success while the peer logs
  nothing — the control session `await`ed a write inside `select!`, `write_tagged`'s `flush()` parks
  forever on a full send buffer, the outbound queue was **unbounded**, and no ping was ever sent.
  Fix: `tetherly-core::liveness` (clock-injected heartbeat/deadline policy, "no inbound for 45s ⇒ dead",
  +4 deterministic tests); `tetherly-net` `SESSION_WRITE_TIMEOUT` + `NetError::WriteStalled` +
  `ActiveSession::send_bounded()`; `tetherly-node` bounded channels (`mpsc::channel` 256/32) with
  `try_send` backpressure, a 15s keepalive ping on the control session, the liveness teardown, and
  `send_bounded` at all 8 write sites (screen drops whole frames under pressure). The Linux release
  build moved into a **`debian:bullseye` (glibc 2.31) container** so the artifact's floor is ≤ 2.31
  (the v1.20 `ubuntu-22.04` artifact required `GLIBC_2.34` and could not run on the glibc 2.31 hosts
  §14 promises); the guard now refuses `GLIBC_2.32+` and prints the floor. 271 → **275 green**.
  **Session liveness + backpressure, and a glibc 2.31 Linux release anchor (v1.21)** — the cross-VPS
  test (two VPS hosts, real internet) first showed the v1.20 notify feature works end-to-end (an OTP push
  landed as a candidate), then exposed a genuine defect: after the first push every later `notify`
  returned `notified tdev_...` while the peer logged nothing, both directions, with the sockets still
  `ESTAB` and unacknowledged bytes stuck in Send-Q (a silently black-holed path). Root cause: the
  control-session task `await`ed the write inside its `select!` body, and `write_tagged`'s trailing
  `flush()` parks forever on a full kernel send buffer (zero window) — so the task went deaf while the
  **unbounded** queue kept accepting, and nothing ever sent a ping so `ESTABLISHED` was treated as alive.
  Fix, three layers: `tetherly-core::liveness` (clock-injected, no `now()`; `SESSION_HEARTBEAT=15s`,
  `SESSION_LIVENESS_DEADLINE=45s`, "45s with no inbound frame is dead", saturating so a backwards clock
  never misreads; +4 deterministic tests), `tetherly-net` (`SESSION_WRITE_TIMEOUT=15s`,
  `NetError::WriteStalled`, `ActiveSession::send_bounded()` turning a silent hang into a hard error), and
  `tetherly-node` (all four outbound channels bounded — `mpsc::channel`, 256/32 — with `try_send`
  backpressure errors; the control session gains a 15s keepalive ping + the liveness check → tear down,
  `PeerDown`, so later sends fail fast; all 8 session writes use `send_bounded`; the screen queue drops
  whole frames when full). Also fixed the doc-vs-artifact mismatch: the v1.20 `ubuntu-22.04` artifact
  demanded `GLIBC_2.34` (fine on every reachable host but unable to run on the glibc 2.31 hosts §14
  claims), so the Linux build now runs in a **`debian:bullseye` (glibc 2.31) container** (`build-linux`
  job; `build-desktop` matrix separate; `publish` needs both) and the guard refuses `GLIBC_2.32+` while
  printing the actual floor. Verification: fmt / clippy `-D warnings` / **275 tests** (=271 + `liveness`
  4) / deny all green; cross-VPS reconnect then two notifies both ingested (`has_otp:true` +
  `has_otp:false`). Honest boundary: the container build path is CI-only here.
  RustDesk architecture borrow only, AGPL not linkable.

## Current Phase
- Spec `docs/DEVELOPMENT.md` **v1.21**. Phase 5 real-machine loop + Phase 3 real-machine chain +
  Phase 1 Android native lib + `notify.reply` quick reply + file cross-process resume + Windows credential-vault identity storage + macOS Keychain / Linux libsecret backends + **§13 proptest/fuzz layers** + **§13 stateful state-machine properties** + **real-device APK end-to-end + `tetherly pin` fix + on-device M1.1 p95** + **macOS/Linux screen backends** + **CLI control plane as live-node loopback clients + Linux glibc 2.35 release anchor** + **session liveness/backpressure + glibc 2.31 Linux release anchor (v1.21)** all landed.
- **CLI control plane as live-node loopback clients + glibc 2.35 release anchor (v1.20)**: `tetherly dial` used to `Node::start(cfg)?` a second node — with the daemon already running it re-bound the same LAN ports and died with `no LAN address bound`; `tetherly inject-notify` built a `NotifyPush` in its own process and only called `ingest_local_push`, so it never crossed the Noise session and never reached a peer. Both were the v1.18 `pin` defect class (only the process holding the in-process state can arm a PIN / hold the session / deliver a push). Fix: `Cmd::Dial { addr, pin, ui_port }` → `loopback_post(ui_port, "/api/dial", {addr,pin})`; `InjectNotify` removed, replaced by `Cmd::Notify { title, body, app_id, peer, ui_port }` → `loopback_post(ui_port, "/api/notify", …)` backed by a new `POST /api/notify` route that resolves the target (one live peer implied, multiple need explicit `--peer` parsed as a `DeviceId` and checked against the live set, unknown/offline → 400 `peer is not connected`, zero → 400 `no connected peer`), builds a `.truncate()`d push (`app_name="tetherly"`, actions `["copy"]`, uid from the wall clock), and calls `Node::send_notify()`. `.github/workflows/release.yml`: Linux build row `ubuntu-latest` → `ubuntu-22.04` (glibc 2.35) plus a guard step `objdump -T target/release/tetherly | grep -E 'GLIBC_2\.(3[6-9]|[4-9][0-9])'` that fails the build rather than shipping a binary that cannot start on glibc 2.31 hosts. Not committed: the local Windows-GNU MinGW.org 6.3.0 is a 32-bit toolchain whose `ld` rejects rustc 1.99's `--high-entropy-va`, so the local test run used rustup's bundled 64-bit self-contained MinGW (gcc 14.2) as linker plus NDK clang 20 for the missing `as`/C frontend (`zstd-sys`); `.cargo/config.toml` stays machine-agnostic and CI's MSVC host is unaffected. Regression: `tests/phase1.rs::notify_posted_to_the_live_node_api_reaches_the_paired_peer` (the posted text reaches the peer's candidate list, and an unconnected `--peer` is refused, so a 200 cannot be a blanket accept). 270 → **271 green**.
- **Real-device APK E2E + `tetherly pin` fix + M1.1 p95 (v1.18)**: cloud-phone Galaxy Fold (SM-F900F / arm64-v8a / Android 13, over `adb reverse`) run of the **shipped APK** — front-end `NotificationListenerService` → JNI `Java_app_tetherly_NativeBridge_*` → `libtetherly_android.so` live Noise session → desktop node extracts candidate `{app_id:"com.android.shell", title:"Your verification code", has_otp:true, source:tdev_...}` (**body/OTP never in snapshot or logs**). On-device M1.1 notify round-trip measured `rtt_min≈17.4ms / p50≈17.9ms / mean≈37.5ms / p95≈56.9ms / p99≈361.8ms / max≈361.8ms` — inside the <1.0s budget (≈18× margin); **honest boundary**: this is the cloud phone over the `adb reverse` tunnel, NOT a LAN-direct pair (same model on LAN ≈15–16ms), the p99 tail is adb-tunnel jitter, 8h soak still Manual-required. Two-process live proof: `tetherly pin --ui-port 45726` printed a PIN, `POST /api/dial` with it returned `{"ok":true}`, host logged `path lan peer=tdev_...`; off-port (`45999`) fails honestly with exit 1.
- **`tetherly pin` defect (found by the real-device run, cost hours of false diagnosis)**: `Cmd::Pin` used to do `Node::start(cfg)?; println!(node.generate_pin()?)` — a **throwaway process** whose PIN (`pin_created_ms` is pure in-process state, never persisted) **no listener can ever accept**, so the live daemon always logged `pin expired`. The working route was always the live daemon's loopback API `POST 127.0.0.1:45716/api/pin` (the UI 「显示 8 位 PIN」 button). Fix: `Cmd::Pin { ui_port }` (default 45716) now calls the new `tetherly_node::loopback_post(port, "/api/pin", b"{}")` to arm the **live** node, and fails honestly (non-zero, actionable message) when nothing is listening. Support: `uihttp::spawn` → `bind_and_spawn` (sync bind, returns the real port; a taken port is now a hard error, not a silent `warn!`), `Node::spawn_ui()`/`Node::ui_port()` (`Shared.bound_ui: AtomicU16`), `run()` binds the UI before returning so "node up" ⇒ port really reachable. Regression: `tests/phase1.rs::pin_shown_by_the_live_node_api_is_the_one_that_pairs` (asserts a third node armed with a hardcoded PIN cannot be dialed, so the claim cannot pass by accident). 269 → **270 green**.
- **§13 proptest + fuzz layers (v1.16)**: `tetherly-core/tests/decoders.rs` (27 props) — every inbound decoder never panics on arbitrary input, every encodable message round-trips (notify push/dismiss/reply, clip set, file offer/decision/done, caps update, input event, screen frame, control msg, hello, JSON control-plane); plus sanitize_file_name stays a basename (non-empty, != "."/"..", no '/', no NUL), DataAssembler bounded to 8 KiB, any-order reassembly. `tetherly-net/tests/decoders.rs` (7 props) — `read_len_prefixed`/`read_tagged`/`read_header` never panic, oversized length prefix refused (anti-DoS), valid headers round-trip, `TFL1` reads as `start=0`, wrong magic refused. `tetherly-core/tests/fuzz_decoders.rs` (2) — deterministic xorshift mutator over a valid corpus (inner frames, all 7 control JSON types, all 7 input kinds, whole+fragmented screen frames, control msgs, ANCS source events + data-source tuples, canonical Hello) applying 6 mutation kinds (bit flip, extreme byte, truncate, extend, multi-byte overwrite, splice), feeding every decoder under `catch_unwind`; fixed seed `0x9E3779B97F4A7C15`, 512 mutations/seed, every 4th stacked. `proptest = "1"` (MIT) added as a workspace dev-dependency.
- **§13 stateful state-machine properties (v1.17)**: `tetherly-core/tests/state_machines.rs` (18 tests = 11 proptest + 7 deterministic witnesses). A `program(max_ops)` strategy yields an arbitrary byte op-program (`Vec<u8>` as an instruction stream: one byte selects the action, more bytes supply parameters); a `Cur` cursor **saturates** on out-of-range reads so the program itself can never panic, attributing every panic to the machine under test. Covered machines and the contract asserted after each step: `ReplayGuard` (an accepted id is never re-accepted; ancient ids below the high-water mark are always refused; no overflow panic — pins v1.15's `saturating_add`), `TokenBucket` (a frozen clock releases exactly `floor(burst)` and refuses the next), `NotifyLimiter` (releases never exceed the leaky bound `10 + 2t/1000 + 1`), `ClipLimiter` (a backwards clock grants no extra budget — monotonicity), `FileHub` (every name is sanitized to a safe basename; never self-accepted before a user click; the incoming table stays bounded at `INCOMING_MAX = 256` under a flood), `ResumeTable` (`claim` offsets are `0 < off < total`, `FILE_CHUNK`-aligned, and source-bound), `NotifyHub` (dedup / reply-correlation / expiry invariants hold under any push-reply-tick sequence), `ScreenSender` (`is_streaming()` implies local consent — never streams without it), `ScreenReceiver` (cursor monotone; `stats().received` equals actual receipts), `ScreenAssembler` (any-order reassembly, incomplete stream refused, never panics — pins v1.15's `checked_mul`), `AncsIngress` (no panic on any event sequence; callbacks are inert unless `Ready`, so a `PRE_EXISTING` uid never leaks into `drain_events()` — M4.3).
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
  `cargo test --workspace --locked` all green — **269 passed, 0 failed** (v1.17: +18 state-machine
  tests over the prior 251; the 18 = 11 proptest over arbitrary op-programs + 7 deterministic
  non-vacuity witnesses).
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
- v1.21 (session liveness/backpressure + glibc 2.31 Linux release anchor + SSOT/memory sync) complete
  and fully green locally (275 tests); committed and pushed to trigger CI, then cross-VPS re-verified.

## Next Milestone
- Phase 6 (single-item): Android controlled device, WinFsp mount, file persistent resume,
  Linux ANCS experience, embedded EasyTier FFI. (`notify.reply` closed v1.11; removed from backlog.)

## Key Artifacts
- `docs/DEVELOPMENT.md` (SSOT v1.21)
- `crates/tetherly-{core,crypto,net,node,android}` + `crates/tetherly-core/src/{ancs,screen}.rs`
  + `crates/tetherly-node/src/screen.rs` + `crates/tetherly-android/src/{store,client,jni}.rs`
- `crates/tetherly-core/tests/{decoders,fuzz_decoders,state_machines}.rs`, `crates/tetherly-net/tests/decoders.rs`
- `bins/tetherly-cli`, `tests/{loopback,phase1,phase2,phase3,phase4,phase5}.rs`, `ui/`, `android/`
- GitHub: `dengyie/tetherly`
- Vault: `Note/Project/tetherly/tetherly 开发文档.md`
