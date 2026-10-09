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
