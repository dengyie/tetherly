# Tetherly desktop shell (Phase 1)

Loopback HTTP UI at `http://127.0.0.1:45716`. The node never binds `0.0.0.0`.

- Copy / Insert / Dismiss candidates (insert only on click)
- Confirm or reject incoming files
- Show PIN on this machine only; 3 minute TTL

Full Tauri/React packaging (`src-tauri`) is the next desktop-shell iteration. This page is the Phase 1 UX so notify/OTP/file confirm work without WebView2 in CI.
