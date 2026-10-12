# Phase 3 — Desktop keyboard/mouse CI slice

- Closed: 2026-10-09
- Spec: `docs/DEVELOPMENT.md` v1.5
- Delivered: `tetherly-core::input` (binary `TIN1`, seq window, `InputServer` / `InputClient` /
  `MemorySink`, edge state machine); node listens on separate TCP 45719 (LAN + overlay, never
  `0.0.0.0`); `SessionConfig.resume_only` refuses pairing there; `caps.update inputport=`;
  input frames only on the 45719 channel, never control-plane JSON; `tests/phase3.rs`.
- Validation: local fmt/clippy `-D warnings`/`cargo test --workspace --locked`/deny green;
  GitHub Actions run `37997269866` all five jobs success.
- Manual-required: real OS cursor/key injection (Win SendInput / macOS / Wayland portal+libei),
  100 physical dual-desktop round-trips. M3.4 DeskFlow gateway stays experimental/off; no
  DeskFlow / Lan Mouse / Barrier / KDE source linked.
- Stop: Do not open Phase 4 without a new waiver.

## v1.9 (2026-10-11) — real-machine chain landed

- Delivered: `SystemInputSink` (SendInput injection: 1000-units → 65535 absolute, buttons, wheel, VK keys; `NodeConfig.input_sink`) + controlled-side mirror (observer trace → real sink) + controller `Node::spawn_input_capture` (`WH_MOUSE_LL` always for edge detection, `WH_KEYBOARD_LL` armed only on the remote seat, injected events filtered) + `drive_input_button`/`drive_input_wheel`.
- Probe-verified on this host (temp test, deleted): cursor crossed the right edge and really landed at (0,539); mid-screen move precise (959,539); Win/Esc keyboard injection through the same seat; the injected desktop change produced real 1920x1080 DXGI frames (M5.1 pixel gap closed as a side effect).
- Manual-required (narrowed): dual-desktop 100 physical round-trips (M3.1 device); macOS/Linux capture+injection.
- Validation: workspace 178 tests green, deny clean. SSOT v1.9 (§9.5 semantics pinned: VK codes / 1000 units / mask bits).
