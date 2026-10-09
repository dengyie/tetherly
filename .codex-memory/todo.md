# TODO
## In Progress
- (none)

## Next
- [ ] Phase 3 only after Phase 2 device exit or waiver: native input 45719, no DeskFlow/Lan Mouse/KDE source
- [ ] Physical Android cellular M2.1 p95 and Wi-Fi→overlay failover
- [ ] `libtetherly_android.so` (cargo-ndk) + real Ed25519 JNI identity
- [ ] Physical Android M1.1 p95 and Win+Android 8h soak
- [ ] True Notepad UIA insert (M1.3 device)
- [ ] Tauri/React `src-tauri` desktop shell (next UI iteration)
- [ ] Credential Manager / Keychain instead of identity.bin fallback
- [ ] macOS M1.1（mac 下一迭代）

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
