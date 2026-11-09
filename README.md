<!-- markdownlint-disable-file MD033 MD045 -->
<div align="center">

# Tetherly

**Production-Grade Cross-Terminal Interconnection Engine, Hardware-Bound Peripherals & Multi-Platform Convergence Runtime**

[![License](https://img.shields.io/badge/license-Apache_2.0_OR_MIT-blue.svg?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.80+-orange.svg?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Crates](https://img.shields.io/badge/Crates-6%20Modular%20Crates-purple.svg?style=flat-square)](Cargo.toml)
[![Security](https://img.shields.io/badge/Crypto-SPAKE2%20%7C%20Noise%20IK-success.svg?style=flat-square)](crates/tetherly-crypto)
[![Tests](https://img.shields.io/badge/tests-773%20passed-brightgreen.svg?style=flat-square)]()

[English](./README.md) · [简体中文](./README.zh-CN.md) · [Core Features](#core-features) · [Architecture](#system-architecture) · [Crate Ecosystem](#workspace-crate-topology) · [Quick Start](#quick-start) · [Screen Streaming](#adaptive-screen-streaming-pipeline) · [Security Model](#cryptographic-security-architecture)

<br>

<p align="center">
  <b>Terminates ecosystem vendor lock-in and fragmented utility silos across heterogeneous desktop and mobile devices.</b><br>
  Unifies zero-configuration direct LAN links, mesh overlay sidecars, hardware-isolated OTP token extraction, and cross-desktop virtual input sharing within a hardened Rust runtime.<br>
  Features authenticated SPAKE2/Noise IK channels, sub-frame latency screen streaming with spatial tile deduplication and motion vector reuse, and cross-platform native ANCS BLE notification hubs.
</p>

</div>

---

## Motivation & Architecture Comparison

Heterogeneous desktop and mobile setups frequently suffer from protocol fragmentation: commercial suites mandate proprietary cloud accounts, open-source utilities like Synergy lack mobile notification and OTP integration, and tools like KDE Connect depend on heavy desktop environment frameworks. Tetherly decouples low-level device coordination into a headless, secure daemon:

| Evaluation Dimension | Tetherly Convergence Runtime | Barrier / Synergy | KDE Connect | Commercial Remote Tools |
| :--- | :--- | :--- | :--- | :--- |
| **Authentication & Trust** | SPAKE2 zero-knowledge PIN + Noise IK forward secrecy | Pre-shared key or plaintext TLS | TLS with manual certificate fingerprint | Centralized vendor identity & cloud relay |
| **Input Sharing (KVM)** | Sub-millisecond cursor roaming with borderless auto-reverse | Desktop only, manual config | Keyboard/mouse input over raw JSON | Cloud-relayed, high latency |
| **Mobile Integration** | Android JNI engine (`libtetherly_android`) + iOS ANCS BLE | None | Android app via bulky Qt/C++ | Proprietary mobile companion apps |
| **Screen Streaming** | Tile-grid decomposition, Zstd lossless / H.264, motion vector reuse | None | Screen mirror plugin only | Proprietary lossy capture |
| **OTP Security** | Local heuristic token extraction, never forwards raw SMS | None | Full notification body broadcast | None |
| **Runtime Footprint** | Pure headless Rust daemon (<20MB RSS), loopback HTTP UI | Qt desktop GUI dependency | Qt / KDE framework dependencies | Electron / Chromium wrappers |
| **Overlay Networking** | First-class LAN direct + unmanaged EasyTier sidecar mesh | LAN only | LAN only | Mandatory cloud servers |

---

## Core Features

| Module | Feature Highlight | Technical Implementation & Advantage |
| :--- | :--- | :--- |
| **SPAKE2 & Noise IK** | Mutual authentication & forward secrecy | Mitigates MITM attacks using 8-digit short-lived PINs via Argon2id + SPAKE2, establishing 256-bit Noise IK encryption sessions. |
| **Virtual Input Roaming** | Hardware-grade boundary mouse & keyboard sharing | Four-directional edge traversal with proportional coordinate mapping, drag protection, and automatic input release on peer disconnection. |
| **Adaptive Screen Streaming** | High-framerate low-latency visual delivery | Dynamic 4D matrix coordinator switching between Zstd lossless and H.264, selective NACK recovery, and spatial tile cache deduplication. |
| **Motion Vector Reuse** | Ultra-efficient bandwidth conservation | Detects block-level translations (`TILE_SUBCODEC_MOTION`), compressing frame deltas down to 5-byte movement signals (up to 3276:1 ratio). |
| **Native ANCS Hub** | iPhone BLE notification ingestion without app | Directly communicates via Apple Notification Center Service over WinRT (Windows), CoreBluetooth (macOS), and BlueZ (Linux). |
| **Local OTP Safeguard** | Isolated one-time password extraction | High-confidence heuristic extractor captures 4-to-8 digit auth codes locally without propagating raw sensitive message payloads. |
| **Resumable File Pipeline** | Reliable high-throughput data transfer | Streaming binary channel with incremental SHA-256 verification, resume offset alignment, and rate-limited chunk scheduling. |
| **Dual-Tree CLI** | Comprehensive node governance | Headless daemon accompanied by `tetherly-cli` providing 485 validated command paths through hardened loopback HTTP/SSE APIs. |

---

## System Architecture

```text
┌────────────────────────────────────────────────────────────────────────┐
│                        Local Clients & Operator                        │
│   ├─ Loopback Web UI (http://127.0.0.1:45716)                          │
│   └─ tetherly-cli (485 commands over authenticated loopback HTTP/SSE)  │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ Loopback REST & SSE (Port 45716)
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                    Tetherly Node Daemon (Port 45717)                   │
│   ┌─────────────────────┐ ┌──────────────────────┐ ┌────────────────┐   │
│   │ SPAKE2 / Noise IK   │ │ Virtual KVM Engine   │ │ Screen Pipeline│   │
│   │ Session Coordinator │ │ Edge Roaming & Drag  │ │ 4D Matrix / FEC│   │
│   └──────────┬──────────┘ └──────────┬───────────┘ └────────┬───────┘   │
│              │                       │                      │           │
│   ┌──────────┴───────────────────────┴──────────────────────┴───────┐   │
│   │              tetherly-core (Protocol Frame Dispatcher)          │   │
│   │   OTP Filter · File Channel · Dedup Limiter · State Machines    │   │
│   └──────────────────────────────────┬──────────────────────────────┘   │
└──────────────────────────────────────┼──────────────────────────────────┘
                                       │ Encrypted Noise IK Transport
        ┌──────────────────────────────┼──────────────────────────────┐
        ▼                              ▼                              ▼
┌───────────────┐              ┌───────────────┐              ┌───────────────┐
│ Paired Node   │              │ Android Phone │              │  Apple iPhone │
│ Desktop Peer  │              │ (JNI Client)  │              │  (ANCS BLE)   │
└───────────────┘              └───────────────┘              └───────────────┘
```

---

## Workspace Crate Topology

The workspace maintains a modular separation of concerns across 6 specialized crates and 3 standalone binaries:

| Crate / Binary | Path | Architectural Responsibility |
| :--- | :--- | :--- |
| **`tetherly-core`** | `crates/tetherly-core` | Protocol frames, state machines, OTP heuristic parsing, rate limiters, screen tile codecs, zero OS dependencies. |
| **`tetherly-crypto`** | `crates/tetherly-crypto` | Ed25519 identity, X25519 static exchange, Argon2id KDF, SPAKE2 password-authenticated exchange, Noise IK state. |
| **`tetherly-net`** | `crates/tetherly-net` | TCP listener, SPAKE2 handshake pumps, Noise IK encrypted streams, mDNS discovery, and streaming file channels. |
| **`tetherly-loopback`** | `crates/tetherly-loopback` | Shared HTTP/SSE client with bounded timeouts (connect 5s, request 30s, SSE handshake 10s, idle watchdog 60s). |
| **`tetherly-node`** | `crates/tetherly-node` | Daemon host runtime, local state persistence, loopback REST engine, and the primary `tetherly` binary. |
| **`tetherly-android`** | `crates/tetherly-android` | Android JNI native bindings, client session runtime (`libtetherly_android.so`), and real-device test harnesses. |
| **`tetherly-cli`** | `bins/tetherly-cli` | Full-featured command-line interface with 485 verified paths interacting with active daemon instances. |

---

## Quick Start

### 1. Build from Source

Ensure Rust 1.80+ is installed:

```bash
# Clone repository
git clone https://github.com/dengyie/tetherly.git
cd tetherly

# Compile workspace binaries in release mode
cargo build --release --workspace
```

The compiled binaries will be available in `./target/release/`:
- `tetherly` (Host node daemon)
- `tetherly-cli` (Command-line tool)
- `android_probe` (Diagnostic probe)

### 2. Start Host Node Daemon

```bash
# Launch primary host node (Listen on control port 45717, loopback UI on 45716)
./target/release/tetherly --data-dir ./host-data --name "Workstation-A" run
```

### 3. Issue Short-Lived Pairing PIN

In a secondary shell on the same host:

```bash
# Request an 8-digit temporary PIN from the running daemon (valid for 3 minutes)
./target/release/tetherly pin
```

```text
Tetherly Pairing PIN: 8492-0193
Valid for: 180 seconds
```

### 4. Pair Peer Device

On the second computer or device:

```bash
# Pair using host address and generated PIN
./target/release/tetherly-cli pair --address 192.168.1.50:45717 --pin 84920193
```

Once paired, cryptographic identities (`id_pk` and `n_pk`) are saved locally. Subsequent connections reconnect automatically via forward-secret Noise IK handshakes without re-entering PINs.

---

## Adaptive Screen Streaming Pipeline

The screen pipeline dynamically adapts to varying network throughput, packet jitter, and CPU budgets:

```text
Display Capture (DXGI / ScreenCaptureKit / X11)
  │
  ▼
Dirty Rect Evaluator & Damage Tracker
  │
  ├─► Spatial Tile Grid Decomposition (32x32px Tiles)
  │     ├─ Tile Cache Hash Lookup (Deduplication against frame buffer)
  │     ├─ Motion Vector Displacement (5-byte translation vectors)
  │     └─ Saliency Tiering & Palette Compression
  │
  ├─► Lossless Stream: Zstandard frame assembly (TMV1)
  └─► Low-Bandwidth Stream: H.264 intra/inter codec with dynamic bitrate
        │
        ▼
FEC Encoder & Packet Pacer (Burst smoothing & Trendline bufferbloat guard)
  │
  ▼
Noise IK Encrypted Socket (Port 45717)
```

- **Monotonic Frame Watermark**: Enforces strict delivery sequence (`delivered_seq`), completely preventing out-of-order straggler packets from resurrecting stale frames or corrupting reference canvases.
- **Selective NACK Recovery**: Re-requests missing packet shards individually rather than triggering expensive full-frame keyframe bursts.

---

## Cryptographic Security Architecture

1. **Zero-Knowledge Mutual Pairing**: The pairing handshake executes SPAKE2 parameterized by Argon2id-derived points. The cleartext PIN is never transmitted across the network, and brute-force guessing is locked out after 5 consecutive failures for 15 minutes.
2. **Noise IK Forward Secrecy**: Once paired, regular sessions initiate via the `Noise_IK_25519_ChaChaPoly_BLAKE2s` protocol, where the initiator encrypts against the responder's known static key while providing ephemeral forward secrecy.
3. **Strict Loopback Isolation**: Management endpoints (`http://127.0.0.1:45716`) bind strictly to loopback interfaces, rejecting non-local requests.
4. **Zero Raw Secret Storage**: Passwords and secrets are never logged or stored in plaintext; pairing states are cryptographically isolated in authenticated key stores.

---

## Loopback REST API & Control Endpoints

Running daemons expose a rich set of programmatic endpoints on `http://127.0.0.1:45716`:

| Endpoint | Method | Functional Description |
| :--- | :--- | :--- |
| `/api/status` | `GET` | Node identity, uptime, active peers, and transport health. |
| `/api/pin` | `POST` | Generate or revoke a 3-minute ephemeral pairing PIN. |
| `/api/peers` | `GET` | List connected peers, link latency (RTT), and route states. |
| `/api/clip` | `GET`/`POST`| Inspect clipboard history ring buffer or synchronize content. |
| `/api/files/offer` | `POST` | Initiate an outbound streaming file transfer offer. |
| `/api/screen/start` | `POST` | Request peer permission and begin low-latency screen streaming. |
| `/api/ancs/status` | `GET` | Query Apple Notification Center BLE peripheral connection. |
| `/api/events` | `GET` | Server-Sent Events (SSE) feed of real-time topology updates. |

---

## Contributing & License

Contributions are welcome. Before opening a pull request, verify that the complete test suite and code linters pass cleanly:

```bash
# Run all workspace unit and integration tests
cargo test --workspace

# Validate strict clippy rules
cargo clippy --workspace --all-targets -- -D warnings

# Check code formatting
cargo fmt --all -- --check
```

Licensed under either of:
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
