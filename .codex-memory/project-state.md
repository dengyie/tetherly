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
  4) / deny all green; `ci.yml` green on all five jobs; cross-VPS reconnect then two notifies both ingested
  (`has_otp:true` + `has_otp:false`). **Shipped as `v0.1.3`** (tag `1883377`; `release.yml` green, measured
  glibc floor **`GLIBC_2.30`**, six assets published). Two container fixes were required en route: the job's
  run shell pinned to `bash` (`debian:bullseye`'s `/bin/sh` is dash, which rejects `pipefail`) and
  `sources.list` pointed at `archive.debian.org` (bullseye is past LTS, so `deb.debian.org` no longer serves
  its pool and `apt-get update` 404s). Honest boundary: the container build path is CI-only here, so a real
  glibc 2.31 host should still confirm the binary starts once.
  RustDesk architecture borrow only, AGPL not linkable.
  **Inner-frame payload budget corrected (v1.22)** — while writing the v1.21 write-stall regression I hit
  `Crypto(Noise("input error"))` with a full-size inner frame and traced it to a **latent production
  defect, not a test bug**: `INNER_PAYLOAD_MAX` was a flat `64 * 1024`, ignoring the 12-byte inner header
  that `InnerFrame::encode` prepends and the 16-byte AEAD tag — both of which live inside the Noise
  transport's 65535-byte single-message cap. Measured (real IK handshake) the transport accepts exactly
  **65519** plaintext bytes, so an inner payload can be at most `65519 - 12 = 65507`; the old constant was
  **29 bytes too large**. Consequence: the screen fragmentation `SCREEN_FRAG_CHUNK_MAX = INNER_PAYLOAD_MAX
  - SCREEN_FRAG_HEADER` produced fragment payloads of `24 + 65512 = 65536` → 65548-byte plaintext, which
  the transport **refuses to encrypt** — so **any screen frame large enough to fragment could not be sent
  at all**. zstd keeps most real frames small, which is why the shipped end-to-end path never hit it (a
  "success the process cannot prove" sibling of the v1.18 `pin` class). The same root cause touched
  `to_frame()`'s single-frame path. The **file channel is unaffected** (it uses its own ChaCha20-Poly1305 +
  length prefix on the second TCP port, not the Noise transport). Fix: derive the budget instead of
  hardcoding it — `frame.rs` gains `INNER_HEADER = 12` and `NOISE_MESSAGE_MAX = 65_535`,
  `INNER_PAYLOAD_MAX = NOISE_MESSAGE_MAX - 16 - INNER_HEADER` (= **65507**), and `encode`/`decode` use
  `INNER_HEADER`. Fragment payload is now exactly 65507 → 65519 plaintext = the transport ceiling, exactly
  encryptable. Wire-compatible: no version ever successfully sent a payload in (65507, 65536] (encryption
  fails before the write). New regressions (275 → **280 green**): `tetherly-crypto/tests/limits.rs` (2 —
  measured ceiling 65519 pass / 65520 refuse, and a max inner frame is encryptable while one byte more is
  refused), `tetherly-net/tests/screen_wire.rs` (1 — a 4 MiB 1024x1024 BGRA frame fragments, crosses a
  real session fragment-by-fragment, and reassembles byte-identical), `tetherly-net/tests/backpressure.rs`
  (2 — the v1.21 write-stall session regression). fmt / clippy `-D warnings` / 280 tests / deny all green.
  **Windows ANCS BLE GATT transport landed (v1.23)** — §9.4 named "Windows: the `windows` crate GATT" but
  the repo only had the OS-free `tetherly-core::ancs` (codec + `AncsIngress` + `AncsTransport` port +
  `MemoryAncsTransport`); the real stack **and the inbound wiring** did not exist (`NodeConfig.ancs` is
  `None` on every production path), so ANCS could not start at all on a real machine — the port existed,
  the plug did not. New `crates/tetherly-node/src/ancs.rs` (assembly point: `AncsInbound`
  `Ready`/`NotificationSource`/`DataSource`/`Disconnected`, `open(address) -> Result<Option<AncsLink>, CoreError>`
  honestly `Ok(None)` off Windows) + `crates/tetherly-node/src/ancs/win_gatt.rs` (self-contained
  `#[cfg(windows)]`, absolute imports only, per the `screen/macos.rs` precedent). Thread model: a dedicated
  `tetherly-ancs` thread owns every WinRT interface and creates its own **MTA** apartment
  (`CoInitializeEx(COINIT_MULTITHREADED)`; `windows` 0.59 has no `initialize_mta` wrapper), while the
  `AncsTransport` front-end holds only a `Sender` — so the port is `Send + Sync` **by construction** and the
  node never touches a COM interface. Chain: `FromBluetoothAddressAsync` →
  `GetGattServicesForUuidAsync(ANCS)` → `GetCharacteristicsAsync` → find the three characteristics and
  **validate properties** (Notification Source / Data Source must be `Notify`, Control Point must be `Write`);
  CCCD writes `Notify`; `ValueChanged` callbacks **only copy bytes** into an unbounded channel (§9.4 forbids
  writing the Control Point from that thread); `write_control_point` goes through `DataWriter` →
  `WriteValueWithResultAsync` and checks `GattWriteResult::Status`, **blocking until the result is true** so
  "serial, one in flight" is a fact; the front-end bounds every ack wait at **10s** and reports
  `AncsSubscribe`/`AncsWrite` on timeout rather than a success it cannot prove. `ConnectionStatusChanged`
  raises a flag → the session ends → `ancs_backoff_delay` exponential backoff (1s→30s); `Session::drop`
  explicitly `RemoveValueChanged`/`RemoveConnectionStatusChanged`. The three **synchronous port methods use
  `windows` 0.59's blocking `IAsyncOperation::get()`** (an OS `Waiter`, no busy-wait), so **no new dependency
  and no hand-rolled executor**. Node side: `Node::spawn_ancs_pump` (own thread + current-thread runtime) feeds
  `AncsInbound` into `ancs_connect`/`ancs_notification_source`/`ancs_data_source`/`ancs_disconnected` and drives
  `ancs_tick` every 500ms; CLI `tetherly run --ancs <11:22:33:44:55:66>`. macOS (CoreBluetooth) / Linux (BlueZ)
  stay unimplemented (`open()` honestly `Ok(None)`). New regressions (280 → **285 green**): 4 pure-logic
  `win_gatt` unit tests (Windows address notation, 6 malformed inputs refused, the four ANCS UUIDs' LE decode
  ↔ canonical GUID field-for-field, malformed UUID reported) + 1 end-to-end `tests/phase4.rs::m4_5` driving the
  pump (subscribe order — Data Source first; callbacks only enqueue while the Control Point is written on `tick`;
  a Data Source response becomes a candidate; `Disconnected` → `Backoff`). `Cargo.lock` **unchanged** (the WinRT
  Bluetooth stack is a feature flag on the already-present `windows` crate). fmt / clippy `-D warnings` / 285
  tests / deny all green. **Honest boundary**: the real iPhone runtime (pairing, live notifications, M4.1 p95)
  stays Manual-required.

  **文件单文件上限成为真门 + 网卡分类可解释且单一真理源 (v1.24)** — 两个边界修复，同属「成功但不可证 / 静默跳过」缺陷族：
  (1) `tetherly_core::filexfer::FILE_MAX = 512 MiB` 此前是**死常量**；v1 文件路径两端整文件驻留内存，故它本质是**内存**上界，却没有任何一处真正把门。现在三处设卡：offer 校验（`offer_resumable` 里 `size > FILE_MAX → CoreError::FileTooLarge`，新错误变体）、45718 通道（`recv_chunks_after_header` 的 `max_size` 由 `u64::MAX` 收紧为 `FILE_MAX`，`accept_file` 的旧「死守卫」`total > size.max(32 MiB)` 换成真判据）、发送端入口（`offer_and_send_bytes`）。并且**拒收不再拆会话**：`SessionEvent::FileOffer` 臂的 `?` 冒泡改为 `warn!` + 继续（超限/坏名/坏哈希只拒这一单，不杀会话）。
  (2) 网卡分类此前散落在 `should_listen` 的 IP 区间判断里，读者看不到「为什么这个地址被跳过」。现在 `tetherly_core::bind::{BindClass, classify, classify_v4}` 是**单一真理源**：新增 `LoopbackAlias`（非 `127.0.0.1` 的 `127/8` 别名，如 systemd-resolved 的 `127.0.0.53`——`if_addrs` 会枚举它，旧 `lan_ips` 靠 `is_loopback()` 隐式跳过，与 `should_listen` 判据会漂移）与 `CgnatOverlay`（RFC 6598 `100.64.0.0/10` = Tailscale/NetBird 承载网，在 Rust 里既非 `is_private()` 也非 link-local，故 v1 默认静默跳过，需显式 `--overlay-cidr`）；`BindClass::reason()` 给每条被跳过的地址一句可读理由。node 侧 `lan::interface_report()` 做成只读出口，`uihttp::Snapshot.lan` 让 UI 不读日志也能解释「什么都没找到」，启动时 `log_skipped_interfaces()` 逐个打一行 info。**290 全绿**（285 → +5）。
## Current Phase
- Spec `docs/DEVELOPMENT.md` **v1.25**. Phase 5 real-machine loop + Phase 3 real-machine chain +
  Phase 1 Android native lib + `notify.reply` quick reply + file cross-process resume + Windows credential-vault identity storage + macOS Keychain / Linux libsecret backends + **§13 proptest/fuzz layers** + **§13 stateful state-machine properties** + **real-device APK end-to-end + `tetherly pin` fix + on-device M1.1 p95** + **macOS/Linux screen backends** + **CLI control plane as live-node loopback clients + Linux glibc 2.35 release anchor** + **session liveness/backpressure + glibc 2.31 Linux release anchor (v1.21)** + **inner-frame payload budget corrected (v1.22)** + **Windows ANCS BLE GATT transport + inbound pump (v1.23)** + **FILE_MAX 成为真门 + 网卡分类单一真理源 (v1.24)** all landed.
- **GitHub CI-First 准则与生产文件发送闭环 (v1.25)**:
  (1) **GitHub CI-First 构建体系**：所有跨平台矩阵构建（Windows x86_64, macOS arm64, Linux glibc 2.31 容器, Android NDK JNI）、代码风格 (`cargo fmt`)、代码合规检测 (`cargo clippy -D warnings`)、全量测试套件 (`cargo test --workspace --locked`) 与许可审计 (`cargo deny check`) 全面以 GitHub Actions 为核心准绳；本地开发保持轻量代码编辑与单机逻辑快速验证，避免本机多目标交叉编译工具链漂移。
  (2) **生产文件发送入口闭环**：回环 API 扩充 `POST /api/file/send`；CLI 客户端扩充 `tetherly send <file_path> [--peer <device_id>] [--ui-port <port>]`；UI 呈现文件发送表单；端到端回归断言 `tests/phase1.rs::file_sent_via_live_node_api_reaches_the_paired_peer`。
- **Windows ANCS BLE GATT transport + inbound pump (v1.23)**: `crates/tetherly-node/src/ancs.rs` (`AncsInbound` four-state channel; `open(address) -> Result<Option<AncsLink>, CoreError>`, non-Windows honestly `Ok(None)`) + `crates/tetherly-node/src/ancs/win_gatt.rs` (self-contained `#[cfg(windows)]`). Dedicated `tetherly-ancs` thread + MTA apartment owns all WinRT interfaces; front-end `AncsTransport` holds only a `Sender`. Property-validated characteristics; CCCD `Notify`; `ValueChanged` copies bytes only; Control Point written on that thread with a 10s ack bound; `ConnectionStatusChanged` → backoff (1s→30s); `Session::drop` deregisters. Blocking `IAsyncOperation::get()` → no new dep, no executor. `Node::spawn_ancs_pump` (own thread + current-thread runtime) wires `AncsInbound` into the `ancs_*` entries + `ancs_tick` (500ms); CLI `--ancs <addr>`. 280 → **285 green**; `Cargo.lock` unchanged.
- **FILE_MAX 成为真门 + 网卡分类单一真理源 (v1.24)**: `crates/tetherly-core/src/{bind.rs,filexfer.rs,error.rs,lib.rs}`、`crates/tetherly-node/src/{lan.rs,runtime.rs,uihttp.rs}`、`crates/tetherly-net/src/filechan.rs`、`tests/phase1.rs`。`BindClass{Listen,LoopbackAlias,Unspecified,Broadcast,Multicast,Overlay,CgnatOverlay,Ipv6,NotLan}` + `classify()`；`FileTooLarge(u64)`；`FILE_MAX` 三处设卡（offer / 45718 通道 / 发送端）；`SessionEvent::FileOffer` 拒收改为 `warn!` 不拆会话；`lan::IfaceReport` + `interface_report()` + `log_skipped_interfaces()`；`Snapshot.lan`。新增 5 条回归：`bind::classify_names_the_reason_and_stays_consistent`、`filexfer::an_oversized_offer_is_refused_before_it_becomes_a_candidate`、`filechan::a_declared_size_over_the_cap_is_refused_on_the_header_alone`、`lan::interface_report_matches_the_actual_listen_set`、`phase1::an_oversized_offer_is_refused_without_dropping_the_session`。SSOT v1.24。
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
  `cargo test --workspace` all green — **349 passed, 0 failed** (v1.67: ADR-060 cross-device peer direct secure messaging, notes, unread status flow, circular message engine, and zero-plaintext SSE test added; 100% green across all crates).
- Target cross-check: `cargo check --target aarch64-unknown-linux-gnu` clean.
- `cargo deny check` → advisories ok, bans ok, licenses ok, sources ok (easytier* still banned; AGPL crates strictly rejected).
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
- Phase 4 ANCS GATT 传输栈已在 Windows (WinRT v1.23)、macOS (CoreBluetooth v1.57 ADR-050) 与 Linux (BlueZ D-Bus v1.58 ADR-051) 三大主流桌面操作系统实现 100% 同构全闭环。真实物理 iPhone 运行时接入（配对及 M4.1 p95 时延）仍属 Manual-required。
- AGPL/GPL screen-capture crates are banned by deny.toml; only permissive impls (windows, core-graphics, x11rb) are eligible.

## Active Blockers
- Physical Android M1.1 p95 + 8h soak (Manual-required; `.so` now built, need timing harness + long soak).
- True SendInput / macOS / Wayland portal+libei and 100 physical dual-desktop round-trips (Manual-required).
- Physical Android cellular + user EasyTier for true M2.1 p95 (Manual-required).
- Real iPhone ANCS p95 < 2s and Bluetooth-off-30s-and-on verification (Manual-required).
- Windows cross-desktop p95 measurement + OpenH264/rav1e + macOS/Linux screen loop + dual-desktop 100 physical input round-trips (Manual-required, Phases 3+5 narrowed). Cross-device (cloud-phone AdbSource) screen loop, single-machine key/mouse injection, and on-device Android pair/notify/resume are verified.

## Current Focus
- **v1.60 (ADR-053)**: 跨端收件箱生命周期治理、已接收文件多维检索与安全审计清除全闭环已落地：
  1. **收件箱生命周期治理与强类型 DTO 抽象 (`crates/tetherly-node/src/runtime.rs` & `lib.rs`)**：解决收件箱落盘后细粒度画像盲区与缺乏删除原语的痛点，设计 `InboxFileDto`、`InboxReport`、`InboxDeleteReport` 与 `InboxClearReport`，支持单遍扫描 `<data_dir>/inbox/` 目录，单文件尺寸提取、UNIX 时间戳、扩展名解析与合规安全性校验；
  2. **安全单文件删除与批量审计清空原语**：实现 `Node::delete_inbox_file` 与 `Node::clear_inbox`，严格执行路径穿越红线守卫（坚决拦截 `..`、`/`、`\`、NUL、隐藏文件等恶意路径，返回 `CoreError::UnsafeFileName`），并支持按 `before_secs` 截断安全清除过期文件；
  3. **事件总线生命周期闭环与零明文审计**：在 `UiEvent` 中扩展 `InboxFileDeleted` 与 `InboxCleared` 事件变体，严格践行 Zero-Plaintext 铁律（仅传输安全文件名、释放字节数与时间戳，严禁携带敏感正文与路径），实现 SSE 实时推送；
  4. **回环控制面端点群全闭环 (`crates/tetherly-node/src/uihttp.rs`)**：新增 `GET /api/file/inbox`（别名 `/api/inbox`）、`POST /api/file/inbox/delete`（别名 `DELETE /api/file/inbox`）与 `POST /api/file/inbox/clear`（别名 `DELETE /api/file/inbox/all`），全量执行 127.0.0.1 回环绑定与 `Host: 127.0.0.1|localhost` 403 Forbidden 强阻断校验；
  5. **双端 CLI 完整同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**：增设 `inbox` 子命令族（`inbox list [--search] [--limit] [--json]`、`inbox delete <name>`、`inbox clear [--yes] [--before-secs]`），并增强 SSE 事件监听器高亮输出；
  6. **Web UI 已收文件治理交互面板 (`ui/index.html`)**：新增「已接收文件与收件箱治理 (Inbox)」卡片，支持实时搜索过滤、单项删除、一键清空与 SSE 联动即时局部重绘；
  7. **端到端集成测试与全量安全基线全绿**：`tests/phase1.rs` 增设完整集成测试覆盖空收件箱查询、多文件排序、关键字过滤与 limit 截断、路径穿越攻击阻断、单项物理删除与 SSE 事件、批量清空与保留分块侧车、Host 403 恶意头阻断，全工作区 342+ 项测试、Linux 跨端交叉编译、Clippy 零告警与 Cargo Deny 100% 通过。
- **v1.59 (ADR-052)**: 跨端多文件批量发送提议、断点续传可观测性画像与传输治理全闭环已落地：
  1. **批量文件提议管道与原子批次治理 (`crates/tetherly-node/src/runtime.rs`)**：解决此前传输仅能单文件逐个提议导致的对端弹窗洪泛与缺乏批次聚合统计的痛点，设计 `BatchSendRequest`、`BatchSendItem`、`BatchSendReport` 与 `BatchTransferItemReport`，引入全局唯一批次标识 `tbt_<hex12>`，支持物理路径与 Base64 内存缓冲混合提议，严格执行 $O(1)$ 内存流式分块哈希（64 KiB 增量 SHA-256 计算）与总字节数、成功/失败计数原子回报；
  2. **深度断点续传可观测性画像 (`ResumeMarkerDto` & `ResumeMarkersReport`)**：打破分块断点续传标记此前仅在内存与磁盘 `.resume.json` 侧车文件中的黑盒盲区，实现 `Node::resume_markers_report`，双重检索内存 `ResumeTable` 与 `inbox/` 目录中的 `.<transfer_id>.resume.json` 与 `.part` 实体，计算块对齐特征、精确进度百分比（`progress_percent`）并反解对端友好别名，并入 `StorageReport`；
  3. **存储治理与互斥锁安全解耦**：精准定位并修复 `storage_report` 中 `hubs` Mutex 跨函数调用引发的不可重入自死锁，通过严格局部作用域收敛锁生命周期，确保垃圾回收与断点画像零死锁、零竞态；
  4. **回环控制面端点群全闭环 (`crates/tetherly-node/src/uihttp.rs`)**：新增 `POST /api/file/send-batch`（与别名 `/api/file/batch`）与 `GET /api/file/resume`（与别名 `/api/storage/resume`），全量执行 127.0.0.1 回环绑定与 `Host: 127.0.0.1|localhost` 严格白名单校验（不匹配时强行阻断并返回 403 Forbidden）；
  5. **双端 CLI 完整同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**：升级 `send` 子命令原生接收多个文件路径（`paths: Vec<PathBuf>`），以格式化表格展示批次发送成果与各子项 Transfer ID；新增 `resume` 命令支持 ASCII 进度表格排版与 `--json` 管道透传；
  6. **Web UI 多文件提议与续传状态画像 (`ui/index.html`)**：`<input id="sendfile" multiple>` 升级原生支持批量文件选择与多任务 Base64 聚合提议，传输卡片增设断点续传绿色徽标（`Resumed`）画像；
  7. **端到端集成测试与全量安全基线全绿**：`tests/phase1.rs` 增设 `batch_file_offer_pipeline_and_resume_observability_via_loopback_api` 完备验证单测/集成测试，全部 341+ 项测试 100% 全绿，Clippy 零警告，Cargo Deny 审计通过。
- **v1.58 (ADR-051)**: Linux BlueZ D-Bus GATT 传输栈架构与跨平台三端 ANCS 全闭环已落地：
  1. **零 Lockfile 扰动纯 Rust D-Bus 依赖架构 (`crates/tetherly-node/Cargo.toml`)**：在 `[target.'cfg(unix)'.dependencies]` 中配置 `zbus 4`，彻底复用 `Cargo.lock` 已锁定的 `zbus 4.4.0`，无额外 C/C++ 依赖与 AGPL 风险；
  2. **前端契约与线程隔离架构 (`crates/tetherly-node/src/ancs/bluez.rs`)**：前端 `BluezAncsTransport` 满足 `Send + Sync` 契约，单次响应通道并在 10s 内同步等待，后台独立专用线程 `"tetherly-ancs-bluez"` 持有单线程 Tokio Runtime，避免与全局运行时死锁；
  3. **BlueZ D-Bus 对象树拓扑与 48 位 MAC 寻址 (`dev_path_from_mac`)**：通过 `dev_path_from_mac` 将 48 位 MAC 映射为 `/org/bluez/hci0/dev_XX_XX_XX_XX_XX_XX`，检索 `ObjectManager` 解析 ANCS 服务与 3 个特征；
  4. **严格恪守 SSOT §9.4 硬约束**：先订阅 Data Source 再订阅 Notification Source，监听 `PropertiesChanged` 信号单向流式推送解包入 `AncsInbound`（严禁回调反向写入 Control Point），`WriteValue` 携带 `{"type": "request"}` 严格串行写入并等待 ATT 确认，监听 `Connected == false` 推入 `AncsInbound::Disconnected` 触发指数退避；
  5. **跨平台三端 ANCS 全闭环 (`crates/tetherly-node/src/ancs.rs`)**：Linux 平台 (`#[cfg(all(unix, not(target_os = "macos")))]`) 正式路由至 `bluez::open`，达成 Windows WinRT、macOS CoreBluetooth 与 Linux BlueZ 三端 100% 同构全闭环。
- **v1.57 (ADR-050)**: macOS ANCS CoreBluetooth 传输栈架构、跨平台外设地址抽象与守护进程链路修复已落地：
  1. **跨平台外设地址抽象与通用解析器 (`tetherly-core/src/ancs.rs` & `lib.rs`)**：确立强类型 `BleAddress` 枚举，桥接 Windows 48 位物理 MAC (`Mac([u8; 6])`) 与 macOS CoreBluetooth 128 位 RFC 4122 `NSUUID` (`Uuid([u8; 16])`)，实现通用解析器 `parse_ble_address` 与规范化字符串序列化 `to_canonical_string`；
  2. **守护进程启动链路与状态机所有权修复 (`crates/tetherly-node/src/bin/tetherly.rs`)**：彻底修复 CLI `--ancs <addr>` 参数解析后静默丢弃传输层的致命缺陷，在 `Node::start` 前将 `cfg.ancs = Some(transport)` 与 `cfg.ancs_peripheral = addr` 注入配置，确保内部状态机持有传输层所有权，打通 `spawn_ancs_pump` 与 `ancs_connect` 链路；
  3. **macOS 原生 CoreBluetooth GATT 传输栈构建 (`crates/tetherly-node/src/ancs/macos.rs` & `ancs.rs`)**：基于 `objc2-core-bluetooth` 与 GCD 串行调度队列构建自包含 Objective-C 代理 `AncsDelegate`，在独立后台线程运行 `CBCentralManager`，依据 128 位 UUID 检索并连接外设，解析 ANCS 服务与 3 个特征（Notification Source、Data Source、Control Point），非阻塞流式推送到 `AncsInbound`，串行化带响应写入 Control Point，支持断线自动触发退避重连；
  4. **运行时可观测性与控制面透视 (`crates/tetherly-node/src/runtime.rs` & `bin/tetherly.rs`)**：`AncsReport` 扩充 `pub peripheral: Option<String>` 字段，节点状态打印清晰指示外设硬件 MAC 或 CoreBluetooth UUID；
  5. **端到端集成与回归验证 (`tests/phase4.rs` & `tetherly-core`)**：新增 `test_ble_address_and_daemon_ancs_wiring_regression` 验证跨平台外设地址解析、配置注入与守护进程状态机初始化。
- **v1.56 (ADR-049)**: ANCS 苹果通知服务全生命周期治理、白名单规则动态增删查、状态机事件总线闭环与双端 CLI 全同构已落地：
  1. **核心白名单原语扩展与 URL 注入守卫 (`allowlist.rs` & `error.rs`)**：`OpenAllowlist` 扩展 `remove` 与 `clear` 原语，`OpenRule` 派生 `Serialize`；严格校验 `is_local_scheme` 阻止外部恶意 URL scheme 劫持注入；核心错误确立 `CoreError::NoAncsTransport` 变体；
  2. **事件总线扩展与安全红线守卫 (`events.rs`)**：新增 `UiEvent::AncsStateChanged` 与 `UiEvent::AncsRuleChanged`，严格遵守 Zero-Plaintext 铁律，不携带任何苹果通知消息正文、标题或敏感属性，仅传递状态、附着标记、退避时间戳、动作与规则计数；
  3. **结构化 DTO 抽象与运行时控制原语 (`runtime.rs`, `lib.rs`)**：定义并导出 `OpenRuleDto`、`AncsConnectReport`、`AncsDisconnectReport`、`AncsRuleReport`、`AncsClearRulesReport`，升级 `AncsReport` 支持 `rules: Vec<OpenRuleDto>`，实现 `ancs_connect_reported`、`ancs_disconnect_reported`、`add_open_rule_reported`、`remove_open_rule_reported`、`clear_open_rules_reported`，在发生状态流转或白名单变更时全生命周期广播 `UiEvent`；
  4. **轻量级回环控制面端点升级 (`uihttp.rs`)**：重构 `GET /api/ancs` 返回强类型 `AncsReport`，重构 `POST /api/ancs/connect` 与 `POST /api/ancs/disconnect` 返回强类型报告，扩展 `POST /api/ancs/allow` (及 `POST /api/ancs/rules/add`)、`DELETE /api/ancs/allow` (及 `POST /api/ancs/rules/remove` 与 `DELETE /api/ancs/rules`)、`POST /api/ancs/rules/clear` (及 `DELETE /api/ancs/rules/clear`)；严格执行 127.0.0.1 回环绑定与 Host 403 校验；
  5. **CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs`, `crates/tetherly-node/src/bin/tetherly.rs`)**：`ancs` 命令族扩展 `status`、`connect`、`disconnect`、`allow`、`remove`、`clear` 子命令，提供整洁对齐列表与 `--json` 原生管道输出，SSE `events` 监听器高亮 `[ancs_state_changed]` 与 `[ancs_rule_changed]`；
  6. **Web UI 实时动态刷新与状态徽标 (`ui/index.html`)**：新增「断开 ANCS」操作、规则列表行内即时删除标签 (`×`) 与「清空白名单」按钮，在 SSE 分发循环中挂载状态与规则事件监听，实现毫秒级局部刷新；
  7. **端到端集成测试全覆盖 (`tests/phase4.rs`)**：新增 `ancs_lifecycle_governance_rule_management_and_events_via_loopback_api` 完备验证。
- **v1.55 (ADR-048 / Release v0.1.5)**: 网络拓扑全景透视、网卡物理分类动态重探治理、EasyTier 侧车穿透可观测性与事件总线闭环已完成开发、全矩阵 CI 验证、Release 四端构建并在线上生产 VPS 验证通过：
  1. **事件总线扩展与安全红线守卫 (`events.rs`)**：新增 `UiEvent::NetworkRefreshed` 与 `UiEvent::OverlayStateChanged`，严格遵守 Zero-Plaintext 铁律，不泄露私网敏感信息，仅携带分类网卡统计、监听计数、侧车状态及对端数量；
  2. **结构化 DTO 抽象与运行时控制原语 (`runtime.rs`, `lib.rs`)**：定义并导出 `IfaceReportDto`、`OverlayStatusReport`、`NetworkReport`、`NetworkRefreshReport`，实现 `Node::network_report_dto` 与 `Node::refresh_network`，在运行时主动执行物理网卡重新枚举与 EasyTier 侧车连接探测，在侧车状态演进或主动重探时全生命周期广播 `UiEvent`；后台 `overlay_scan_loop` 同步在侧车状态演进时派发 `UiEvent::OverlayStateChanged`；
  3. **轻量级回环控制面端点升级 (`uihttp.rs`)**：重构 `GET /api/net` 返回强类型 `NetworkReport`，新增 `POST /api/net/refresh` 与 `POST /api/net/probe` 返回强类型 `NetworkRefreshReport`，严格 127.0.0.1 回环绑定与 Host 403 校验；
  4. **CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs`, `crates/tetherly-node/src/bin/tetherly.rs`)**：`net` 子命令增设 `--refresh` (`-r`) 参数，支持主动重探与 `--json` 管道输出，SSE `events` 监听器高亮 `[network_refreshed]` 与 `[overlay_state_changed]`；
  5. **Web UI 实时动态刷新与状态徽标 (`ui/index.html`)**：在「网络与接口诊断」面板新增「探测刷新」按钮与即时反馈，在 SSE 分发循环中挂载网络探测与侧车状态变更监听，实现局部解耦渲染；
  6. **端到端集成测试全覆盖 (`tests/phase2.rs`)**：新增 `dynamic_network_rescan_and_overlay_events_via_loopback_api` 完备验证，并通过 `wait_for_network_refreshed` 过滤消除 Windows CI 广播竞争条件；
  7. **GitHub Actions 5 端 CI 全绿 + 4 端 Release 构建发布 (`v0.1.5`)**：Linux (bullseye glibc 2.31 容器)、Windows (x86_64)、macOS (Apple Silicon arm64)、Android (NDK arm64-v8a + APK) 产物全部就绪并发布到 GitHub Release；
  8. **生产 VPS 真实环境验证与清理**：在 `aws-tokyo` 线上 VPS 成功下载 `v0.1.5` Linux 二进制，全面验证守护进程启动、健康检查、非回环 Host 403 阻断、网卡探测重探与双端 CLI 交互，并完成现场全量干净清理。
- **v1.54 (ADR-047)**: 远程屏幕与键鼠共享全生命周期治理、实时遥测可观测性与事件总线闭环已落地：
  1. **事件总线扩展与安全红线守卫 (`events.rs`)**：新增 `UiEvent::ScreenStateChanged` 与 `UiEvent::InputSeatChanged`，严格遵守 Zero-Plaintext 铁律，不携带像素缓冲区、按键扫描码或光标敏感轨迹；
  2. **结构化 DTO 抽象与运行时控制原语 (`runtime.rs`, `lib.rs`)**：定义并导出 `ScreenStatusReport`、`ScreenControlReport`、`InputStatusReport`、`InputControlReport`，在 `Shared` 中维护 `screen_peer` 与 `input_peer` 互斥追踪，新增 `screen_status_report`、`screen_allow_reported`、`screen_revoke_reported`、`screen_start_reported`、`screen_stop_reported`、`input_status_report`、`input_start_reported`、`input_stop_reported`；
  3. **轻量级回环控制面端点升级 (`uihttp.rs`)**：重构 `/api/screen` 与 `/api/input` 响应，完全由强类型 DTO 驱动，严格 127.0.0.1 回环绑定与 Host 403 校验；
  4. **CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs`, `crates/tetherly-node/src/bin/tetherly.rs`)**：`screen status` 与 `input status` 支持高亮排版与 `--json` 原生管道输出，SSE `events` 监听器高亮 `[screen_state_changed]` 与 `[input_seat_changed]`；
  5. **Web UI 实时动态刷新与状态徽标 (`ui/index.html`)**：在 SSE 分发循环中挂载屏幕与键鼠状态变更监听，动态展示活动对端与遥测统计；
  6. **端到端集成测试全覆盖 (`tests/phase5.rs`)**：新增 `screen_and_input_lifecycle_governance_and_events_via_loopback_api` 完备验证。
- **v1.61 (ADR-054)**: 局域网设备自发现治理、信标可观测性与一键拨号配对闭环已落地：
  1. **网络层 mDNS 浏览与注销原语 (`crates/tetherly-net/src/mdns.rs`)**：封装 `browse` 与 `stop_browse` 原语，解决此前节点仅单向广播而从未监听对端信标的“监听黑盒”；在非 `lan` 特性下提供安全 safe stub；mDNS 广播 TXT 记录增加 `("name", hostname)` 便于直接解析设备名；
  2. **局域网自发现强类型 DTO 抽象与运行时画像 (`crates/tetherly-node/src/runtime.rs`, `lib.rs`)**：定义 `DiscoveredPeerDto`、`DiscoveryReport` 与 `DiscoveryRescanReport`，在 `Shared` 中维护线程安全自发现表 `discovered`，自动过滤自身设备 ID，单遍汇总在线与受信任状态并按活跃时间倒序排布；
  3. **主动重新探测与智能拨号寻址原语**：实现 `Node::rescan_discovery` 主动触发信标组播重探；实现 `Node::dial_peer` 智能寻址引擎（按物理套接字地址、自发现设备名/ID、路由表对端标识优先级链式解析，并配备 2s 单目标快速超时，避免无响应 IP 阻塞）；
  4. **事件总线零明文审计与即时分发 (`crates/tetherly-node/src/events.rs`)**：扩展 `UiEvent::PeerDiscovered` 与 `UiEvent::PeerExpired` 变体，严格践行 Zero-Plaintext 铁律（仅传输公开元数据与能力列表，严禁传输私钥），通过 SSE 实时分发；
  5. **回环控制面端点全闭环 (`crates/tetherly-node/src/uihttp.rs`)**：新增 `GET /api/discovery`（别名 `/api/peers/discovered`）、`POST /api/discovery/rescan`（别名 `/api/peers/discover`），增强 `POST /api/dial` 原生支持 `peer` 智能寻址，全量端点严格执行 127.0.0.1 回环绑定与 `Host: 127.0.0.1|localhost` 403 Forbidden 强阻断；
  6. **双端 CLI 全同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**：增设 `discover [--rescan] [--json]` 子命令、升级 `dial [--addr] [--peer <name|id>]` 智能拨号，并在 SSE `events` 监听器中提供高亮格式化输出；
  7. **Web UI 局域网自发现交互面板 (`ui/index.html`)**：新增「局域网自动发现设备 (Discovered LAN Devices)」卡片，支持实时设备列表、能力特性徽标、一键地址填入、一键快捷拨号与 SSE 联动无感局部重绘；
  8. **端到端集成测试全覆盖 (`tests/phase1.rs`)**：`lan_discovery_and_beacon_governance_via_loopback_api` 全面覆盖自发现初态、主动刷新探测、模拟对端广播、智能寻址解析与拨号、信标过期与 SSE 验证、Host 403 恶意阻断，全工作区 273+ 项测试、Linux 跨端交叉编译、Clippy 零告警与 Cargo Deny 100% 通过。
- **v1.60 (ADR-053)**: 跨端收件箱生命周期治理、已接收文件多维检索与安全审计清除全闭环已落地：
  1. **收件箱生命周期治理与强类型 DTO 抽象 (`crates/tetherly-node/src/runtime.rs`, `lib.rs`)**：定义 `InboxFileDto`、`InboxReport`、`InboxDeleteReport` 与 `InboxClearReport`；
  2. **安全单文件删除与批量审计清空原语**：实现 `Node::delete_inbox_file` 与 `Node::clear_inbox`，严格执行路径穿越红线守卫（拦截 `..`、`/`、`\`、NUL 等）；
  3. **事件总线零明文审计**：在 `UiEvent` 中扩展 `InboxFileDeleted` 与 `InboxCleared`，践行 Zero-Plaintext 铁律；
  4. **回环控制面端点全闭环 (`crates/tetherly-node/src/uihttp.rs`)**：新增 `GET /api/file/inbox`、`POST /api/file/inbox/delete` 与 `POST /api/file/inbox/clear`，严格 127.0.0.1 与 Host 403 校验；
  5. **双端 CLI 完整同构升级**：增设 `inbox` 子命令族 (`list`, `delete`, `clear`)，增强 SSE 事件监听；
  6. **Web UI 已收文件治理交互面板**：新增「已接收文件与收件箱治理 (Inbox)」卡片，支持实时搜索过滤、单项删除、一键清空与 SSE 联动；
  7. **端到端集成测试全覆盖 (`tests/phase1.rs`)**：新增 `inbox_file_lifecycle_query_deletion_and_safe_purge_via_loopback_api` 完备验证。
- **v1.59 (ADR-052)**: 跨端多文件批量发送提议、断点续传可观测性画像与传输治理全闭环已落地：
  1. 解耦重量级全节点快照 `GET /api/state`，新增专用的轻量级 `GET /api/file/offers`，支持按 `transfer_id` 的 $O(1)$ 精准透视与按 `peer` 的轻量过滤；
  2. 提议决策动作零明文结构化报告：定义 `FileOfferDto`、`FileOffersReport`、`FileDecisionReport`、`FileBatchDecisionReport` 与 `AutoAcceptReport`；
  3. 批量提议治理原语：支持面向单端或全域对端的一键接收（`--all`）与一键拒绝（`--all`）；
  4. 受信任对端自动接收无人值守策略（`auto_accept_trusted`）：在 `Shared` 状态维护原子开关与 `auto_accepted_count` 计数器，严格受限于 Noise 握手认证受信状态（`is_trusted(peer)`）；
  5. 事件总线生命周期补完：新增 `UiEvent::FileRejected` 与 `UiEvent::FileAutoAcceptChanged`；
  6. 轻量级回环控制面端点升级：`GET /api/file/offers`、`POST /api/file/accept`、`POST /api/file/reject`、`GET/POST /api/file/auto-accept`（严格校验 127.0.0.1 回环绑定与 Host 403）；
  7. CLI 双端全同构升级：`offers`、`accept`、`reject`、`auto-accept` 子命令，并在 `events` 监听器中高亮展示；
  8. Web UI 提议治理卡片与自动接收开关；
  9. 网络层取消安全与多路复用健壮性强化：设计实现状态化 `TaggedReader` 环形缓冲状态机，消除 Tokio `select!` 竞态丢弃分支时的 partial-read 网络帧解包失步，在 `tests/phase1.rs` 增设步骤 4/5 对端拒绝观测同步，5 平台 CI 100% 全绿；
	  10. 生产级代码审查修复、原子锁治理与多文件提议尺寸纠偏：
	     - **修复 P1 多文件累积字节统计失真**：`file_offers` 中 `total_bytes` 与 `size` 改为全文件求和 `t.files.iter().map(|f| f.size).sum()`；
	     - **修复 P2 拒绝状态流转清理竞态与单点原子锁收敛**：`user_accept_file_reported` 与 `user_reject_file_reported` 将状态转换与元数据提取收敛至单次持锁临界区，加锁由 3 次降为 1 次且消灭并发 prune 竞态；
	     - **代码去重与接口收敛**：`user_accept_file` 与 `user_reject_file` 直接委托 `*_reported` 变体；
	     - **端到端集成测试增强**：`tests/phase1.rs` 增设包含 2 个子文件的多文件提议注入，验证累积 4000 字节及原子拒绝上报。
		  11. **v1.62 (ADR-055) 跨端收件箱文件内容提取、流式下载、系统级直开与多媒体预览全闭环**：
		     - **HTTP 响应协议头与流式结构升级**：`UiResponse` 替代旧三元组，支持任意状态码、标准 MIME 类型映射（`mime_type_from_name` 智能识别 20+ 种格式）与 `Content-Disposition` 协议头；
		     - **二进制安全字节流防失真提取**：重构 `parse_http_response` 为滑动窗口字节切分（`b"\r\n\r\n"`），彻底消除 `String::from_utf8_lossy` 导致的非 UTF-8 字节损毁；
		     - **收件箱文件内容直读与系统原生调起原语**：`Node::read_inbox_file` 与 `Node::open_inbox_file`，对接系统默认打开器（macOS `open`、Windows `rundll32 url.dll,FileProtocolHandler`、Linux `xdg-open`），支持测试注入 `MemoryOpener`；
		     - **严格多层路径穿越红线拦截**：拦截任何包含 `..`、`/`、`\`、`\0` 或以 `.` 开头的文件名（`CoreError::UnsafeFileName`）；
		     - **事件总线零明文审计**：新增 `UiEvent::InboxFileOpened` 变体，仅透出脱敏文件名与时间戳，严禁携带私钥或文件敏感正文；
		     - **回环端点与隔离铁律**：`GET /api/file/inbox/download`、`GET /api/file/inbox/raw`、`POST /api/file/inbox/open`，127.0.0.1 物理绑定与 Host 403 强阻断；
		     - **双端 CLI 全同构升级**：`inbox get <name> [--dest <path>]`、`inbox open [<name>]`，SSE `events` 监听器高亮格式化；
		     - **Web UI 收件箱提取与交互联动**：新增「打开目录」按钮，卡片行新增「打开」、「下载」及「预览」操作，实时联动 SSE；
		     - **端到端集成测试**：`tests/phase1.rs` 增设 `inbox_file_download_raw_preview_and_native_open_via_loopback_api` 全绿。
			  12. **v1.63 (ADR-056) 跨端桌面原生系统级通知推送、横幅弹窗治理与免打扰闭环**：
			     - **消除遗留悬空特质存根**：重构 `crates/tetherly-core/src/ports.rs` 的 `Notifier` trait 与强类型 `NotificationPayload`、`NotificationUrgency`（Low / Normal / Critical），补齐六大平台端口全闭环；
			     - **跨平台原生横幅调度与测试适配器**：`SystemNotifier` 驱动（macOS `osascript` 唤起通知中心横幅与声音、Windows PowerShell 调起气泡/Toast、Linux `notify-send`）与 `MemoryNotifier` 内存捕获容器；
			     - **集中式免打扰 (DND) 与调度策略引擎**：`NotifyBannerConfig` 支持总开关（`banners_enabled`）、三级免打扰模式（`dnd_mode`: `off` / `silent` / `all`）、音效开关（`sound_enabled`）、验证码隐私防窥掩码（`mask_otp`）与紧急验证码免打扰穿透（`allow_otp_in_dnd`）；
			     - **核心通信与传输事件驱动**：手机推送通知摄入（`Node::ingest_local_push`）与大文件接收完成（`Node::receive_file_loop`）自动调度原生横幅；
			     - **Zero-Plaintext 零明文安全红线**：扩展 `UiEvent::NotificationBannerSent` 与 `UiEvent::NotifyConfigChanged`，坚守严禁广播正文与明文验证码铁律；
			     - **回环控制面端点全闭环**：新增 `GET/POST /api/notify/config`（别名 `/api/notify/banner`）与 `POST /api/notify/test`，坚守 127.0.0.1 物理绑定与 Host 403 强隔离；
			     - **双端 CLI 全同构升级**：增设 `banner config` 与 `banner test` 子命令，SSE 事件监听高亮格式化；
			     - **Web UI 桌面通知与免打扰控制台**：新增独立治理卡片、实时开关、DND 下拉选择与一键测试横幅；
			     - **端到端集成测试**：`tests/phase2.rs` 增设 `desktop_notification_banners_dnd_governance_and_otp_masking_via_loopback_api` 全绿。
			  13. **v1.64 (ADR-057) 节点系统资源深度遥测、进程级性能画像与运行时健康度量闭环**：
			     - **平台探测抽象与跨平台原生驱动**：实现 `SysProbe`、`PlatformSysProbe`（macOS Mach 内核系统调用、Linux `/proc/self/statm`、POSIX `getrusage` 与 `/dev/fd` 打开句柄统计，Zero-New-Crate 与 Zero-AGPL 铁律）与 `MemorySysProbe` 测试适配器；
			     - **强类型度量模型与运行时采样原语**：`ProcessMetrics`、`HostMetrics` 与 `SystemReport`，提供 `Node::system_report()` 与探针注入原语；
			     - **Zero-Plaintext 零明文安全红线**：新增 `UiEvent::SystemMetricsRefreshed`，仅广播资源指标，绝不泄露进程环境、命令行参数或私钥；
			     - **回环端点与隔离铁律**：`GET /api/sys`（别名 `GET /api/system`、`GET /api/process`），127.0.0.1 物理绑定与 Host 403 强隔离；
			     - **双端 CLI 全同构升级**：`sys [--watch] [--interval <secs>] [--json]`，SSE 事件监听高亮格式化；
			     - **Web UI 节点系统资源与运行时画像专区**：新增性能画像卡片、实时可视化呈现硬件拓扑、RSS 物理内存、虚拟内存、活跃线程与 FDs，联动 SSE 与周期性自动刷新；
			     - **端到端集成测试**：`tests/phase1.rs` 增设 `node_system_telemetry_probe_injection_and_metrics_via_loopback_api` 全绿。

## Current Branch
- `main` @ `https://github.com/dengyie/tetherly`

## Last Verified
- 本地全工作区验证 100% 通过：
  - `cargo fmt --all --check`：PASSED (0 diff)
  - `cargo clippy --workspace --all-targets -- -D warnings`：PASSED (0 warnings)
  - `cargo check --target aarch64-unknown-linux-gnu`：PASSED (0 errors)
  - `cargo test --workspace`：PASSED (全量 283+ 项测试全部通过)
  - `cargo deny check`（advisories, bans, licenses, sources）：PASSED (0 security/license violations)

## Key Artifacts
- `docs/DEVELOPMENT.md` (SSOT v1.63)
- `crates/tetherly-{core,crypto,net,node,android}`
- `bins/tetherly-cli`, `crates/tetherly-node/src/bin/tetherly.rs`
- `crates/tetherly-net/src/codec.rs` (`TaggedReader`), `crates/tetherly-net/src/session.rs`
- `crates/tetherly-node/src/{runtime.rs,uihttp.rs,events.rs,lan.rs,sidecar.rs,ancs.rs,platform.rs}`
- `ui/index.html`, `tests/phase1.rs`, `tests/phase2.rs`, `tests/phase4.rs`, `tests/phase5.rs`
- GitHub: `dengyie/tetherly`
- Vault: `Note/Project/tetherly/tetherly 开发文档.md`
