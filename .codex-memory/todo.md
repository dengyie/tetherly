# TODO
## In Progress
- (none)

## Next
- [ ] Real ANCS BLE transport on Windows (`windows` GATT), macOS (CoreBluetooth), Linux (BlueZ) — Manual-required
- [ ] Physical iPhone ANCS p95 < 2s + Bluetooth off-30s-on re-pop check (M4.1/M4.3 device)
- [ ] M4.4 / M4.5 optional iOS App (LAN 50MB sha256, no VPN / Network Extension)
- [ ] Physical Android cellular M2.1 p95 and Wi-Fi→overlay failover
- [ ] Real OS cursor/key injection (Win SendInput / macOS / Wayland portal+libei) + 100 dual-desktop round-trips (M3.1 device)
- [ ] `libtetherly_android.so` (cargo-ndk) + real Ed25519 JNI identity
- [ ] Physical Android M1.1 p95 and Win+Android 8h soak
- [ ] True Notepad UIA insert (M1.3 device)
- [ ] Tauri/React `src-tauri` desktop shell (next UI iteration)
- [ ] Credential Manager / Keychain instead of identity.bin fallback
- [ ] macOS M1.1（mac 下一迭代）
- [ ] Phase 5 backlog: Android controlled device, notify.reply, WinFsp mount, persistent file resume, embedded EasyTier FFI

## Done
- [x] Phase 0 crates + CLI + loopback
- [x] PairBind direction nonces
- [x] Trust-after-Noise + Clock + handshake timeout + TCP limiter
- [x] UTF-8 truncate, RNG fail-closed, inner msg_id / replay / rekey signal
- [x] DEVELOPMENT.md v1.2
- [x] Public GitHub + Obsidian mount
- [x] Phase 1 node persist + LAN bind + notify/clip/file
- [x] Loopback HTTP UI 45716
- [x] Android NLService + gradle CI skeleton
- [x] tests/phase1.rs M1.2–M1.7 + simulated M1.1/M1.6
- [x] copy_otp returns CandidateExpired for present-but-expired ids
- [x] deny.toml allow BSL-1.0 (clipboard-win)
- [x] DEVELOPMENT.md v1.3
- [x] Phase 2 sidecar discovery (RPC/CLI/iface) + unicast 45717
- [x] Overlay bind separate from LAN/mDNS; LAN wins over overlay
- [x] tests/phase2.rs M2.2–M2.5 loopback; UI「仅局域网」
- [x] DEVELOPMENT.md v1.4
- [x] Phase 3 `tetherly-core::input` binary TIN1 engine + edge state machine
- [x] Native Noise input port 45719 (resume-only, MemorySink); caps `inputport=`
- [x] tests/phase3.rs 未配对拒绝 / 过边按键 / 断线回光标 / 聚焦时剪贴板
- [x] Windows parallel-test identity temp collision fix; Android unit org.json stub
- [x] DEVELOPMENT.md v1.5; GitHub CI all five jobs green
- [x] Phase 4 `tetherly-core::ancs` codec + `AncsIngress` + `AncsTransport`/`MemoryAncsTransport`
- [x] §9.4 constraints in structure: subscribe order, no CP write on callback, serial CP, byte reassembly, retry, backoff, uid dedup, PreExisting drop
- [x] Node `ancs_*` surface + `Opener` port (`SystemOpener`/`MemoryOpener`) + `open_candidate`
- [x] UI two-step wizard (ANCS then SPAKE2) + `/api/ancs/connect`, `/api/ancs/allow`, `/api/open`
- [x] tests/phase4.rs M4.1/M4.2/M4.3 + serial/reassembly/Removed/inert/backoff (10 tests)
- [x] DEVELOPMENT.md v1.6