# Phase 1 — LAN desktop + Android

- Closed: 2026-10-09 (CI slice, v1.3) / 2026-10-11 (Android native library, v1.10) / 2026-10-11 (notify.reply, v1.11)
- Spec: `docs/DEVELOPMENT.md` v1.11 (§9.6 rewritten; §7.2 notify.reply now real; M1.1/M1.5 rows updated)
- Delivered (CI slice, v1.3):
  - `tetherly-node`: identity/trust persistence (temp+rename, 0600), LAN bind (never `0.0.0.0`, skips the
    EasyTier `10.144.144.0/24`), mDNS `_tetherly._tcp`, session loop, caps advertise `fileport=`.
  - NotifyHub local extract; `insert()` only on user click; candidate TTL 120s; clipboard OTP cleared
    after 60s.
  - File transfer: control-plane offer/accept; data plane on **45718** with `TFL1` + HKDF token; receive
    only after explicit confirmation.
  - Local UI: `ui/index.html` + `uihttp`, Host must be 127.0.0.1/localhost.
  - Android: `NotificationListenerService` + foreground service + Gradle CI skeleton; Wire JSON carries
    no `url` action. `tests/phase1.rs` covers M1.2–M1.7 + simulated M1.1/M1.6.
- Delivered (Android native library, v1.10):
  - **New crate `crates/tetherly-android`** (lib `tetherly_android`, crate-type `["cdylib","rlib"]`):
    - `cdylib` → `libtetherly_android.so` (arm64-v8a, cargo-ndk); `rlib` → host/CI integration test.
    - `store.rs`: real Ed25519+X25519 `Identity::from_secrets`, `identity.bin` (64B, temp+rename, 0600),
      `trusted.json` (SAME schema as the desktop node — files interchangeable), `atomic_write`,
      `load_or_create_identity`, `load_trust`, `save_trust`.
    - `client.rs`: `AndroidClient::load_or_create` + `start(addr, pin, once) -> SessionHandle` on a
      "tetherly-android" thread with a `current_thread` tokio runtime; dials 45717 through the desktop's
      own `dial_session` (SPAKE2 8-digit PIN first run → Noise IK resume). `SessionHandle`:
      `notify()`/`state_code()`/`connected_peer()`/`last_error()`/`stop()`. `PHONE_CAPS=["notify","clip","file"]`
      (never input/screen). Backoff 250ms→5s; `TYPE_PING`→pong.
    - `jni.rs`: `Java_app_tetherly_NativeBridge_nativeAttach|SendNotify|DeviceId|IdPk|State|LastError|Stop`
      with `<'local>` JNIEnv lifetimes; `#![allow(unsafe_code)]` here only (crate root is `#![deny(unsafe_code)]`);
      `jni` dependency gated to `cfg(target_os="android")`.
    - `src/bin/android_probe.rs`: pushed to a phone and run from `adb shell` — real pair+notify proof.
    - `tests/client_e2e.rs`: spawns a real `accept_session` server (host, no emulator) and asserts
      connection + notify delivery + `trusted.json` persistence.
  - `tetherly-net`: added `SessionConfig.caps: Option<Vec<String>>` (explicit Hello cap override; default
    `None` keeps the existing inference); phone sets `Some(PHONE_CAPS)`. All 8 SessionConfig literal sites
    updated.
  - Kotlin: `NativeBridge.kt` (object; attach/sendNotify/deviceId/idPk/state/lastError/stop/splitAddr),
    `NodeSession.kt` (identity = real device_id or "unavailable"; start/stop/state/lastError/enqueueNotify),
    `MainActivity.kt` (status line shows the real device_id + live connection state via a 1s Runnable).
  - CI `android` job: rust `aarch64-linux-android` target + `cargo-ndk` → build the `.so` into
    `android/app/src/main/jniLibs` → Gradle `testDebugUnitTest` + `assembleDebug`. `jniLibs/` is gitignored
    (build artifact). `android/README.md` documents the native build + probe workflow.
- Verified (real hardware, v1.10): two Galaxy Fold (SM-F900F, arm64-v8a, Android 13) run `android_probe`
  over `adb reverse tcp:45717 tcp:45717` (phones have no route to the host overlay — adb is host→phone
  only) against a desktop node on `10.144.144.10:45717`. Each phone generated a real identity, paired with
  a PIN, and the desktop received the `notify.push` (candidate `has_otp:true`; body not in snapshot/logs);
  both `trusted.json` written. A re-run with a deliberately WRONG PIN still connected → proves Noise IK
  resume, not re-pairing. All 7 JNI symbols confirmed exported in the `.so`.
- Delivered (notify.reply, v1.11): the desktop→phone quick reply path on the notify channel.
  - core: `NotifyReply{uid,app_id,text}` (type `0x0103`, ≤`NOTIFY_TEXT_MAX`=4 KiB, multibyte-safe);
    `NotifyHub.replies` inbox keyed by `reply_id = sha256(source\0app_id\0uid)`, `ingest_reply`
    (trim/reject-blank/last-write-wins), single-use `take_reply_id`/`take_reply`, `reply_views()`;
    `ReplyView`/`ReplyViewDto` carry no text; `QueuedReply::Debug` redacts; `CoreError::EmptyReply`.
  - net/node: `SessionEvent::Reply(NotifyReply)` (replaces `ReplyStub`); `Node::reply_to_candidate`,
    `send_reply`, `take_reply`, `reply_views`; `UiEvent::ReplyReceived`; UI routes
    `POST /api/reply` + `POST /api/reply/take`.
  - Android: `SessionHandle.take_replies()` + `serve` loop `TYPE_NOTIFY_REPLY`; JNI
    `nativeTakeReplies`; `NativeBridge.takeReplies()`/`PhoneReply`; `NotifyListener.drainReplies()` +
    `fireRemoteReply()` (fires the notification's OWN `RemoteInput` action; never synthesizes one).
  - CI: `tests/phase2::m2_6_...` and `android/tests/client_e2e::android_client_receives_a_quick_reply_once`.
  - Manual-required: real on-device `RemoteInput` firing (needs a foreground notification with a reply
    action + notification-listener grant).
- Manual-required (narrowed, v1.10):
  - ~~Physical Android M1.1 **p95 timing**~~ **CLOSED v1.18**: on-device notify round-trip
    `rtt_p50≈17.9ms / p95≈56.9ms / p99≈361.8ms` (cloud phone over `adb reverse`, not LAN;
    LAN same-model ≈15–16ms), inside the <1.0s budget. **8h soak still Manual-required.**
  - ~~Real APK end-to-end (Kotlin NLService → JNI → live session) on a physical phone~~ **CLOSED
    v1.18**: the shipped `app.tetherly` front-end NLService → JNI → `libtetherly_android.so` live
    Noise session → desktop candidate `has_otp:true` (body/OTP never in snapshot/logs).
  - macOS M1.1 (mac next iteration).
  - LAN-direct dual-physical round-trip (the v1.18 p95 is over the adb-reverse tunnel, not LAN).
- Red lines honored: no EasyTier/DeskFlow/RustDesk/AGPL/GPL/LGPL linked; core stays OS-free
  (`cfg(target_os)`) with `#![forbid(unsafe_code)]`; the JNI `unsafe` is exempted in exactly one file and
  gated to Android; no `0.0.0.0` binds; pairing stays on 45717; OTP scoring untouched; no OTP/SMS body/
  clipboard content in logs or the snapshot.