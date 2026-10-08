# Tetherly

跨终端设备互联：局域网直连，跨网走 mesh，验证码本机提取，桌面共享一套键鼠。

仓库与 crate 名一律使用 `tetherly`。许可证 **Apache-2.0 OR MIT**。实现合同是 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)。

## Phase 0（当前）

四个 crate + CLI。无 GUI、无 BLE、无 EasyTier。

| crate | 职责 |
|---|---|
| `tetherly-core` | OTP、帧、Hello、状态机、去重、白名单 |
| `tetherly-crypto` | Ed25519 + X25519、Argon2id、SPAKE2、PairBind、Noise IK |
| `tetherly-net` | TCP Hello / SPAKE2 / Noise；可选 LAN mDNS |
| `tetherly-cli` | `listen` / `dial` / `pin` / `identity` |

```text
# 终端 A
tetherly-cli listen --port 45717 --pin 25170394 --name host

# 终端 B
tetherly-cli dial --port 45717 --pin 25170394 --name peer
```

不要把 PIN 发给任何人。错 PIN 5 次锁定 15 分钟。

```text
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Windows GNU 链接需要较新的 MinGW（Scoop `mingw` 16+）。不要用 MinGW.org 6.3：它的 `ld` 不认识 `--high-entropy-va`。

## 平台承诺（v1）

| 端 | 角色 |
|---|---|
| Windows / macOS | 全功能节点 |
| Linux | 全功能，Wayland 键鼠为 beta |
| Android | 通知/验证码源 + 文件/剪贴板 |
| iPhone | ANCS 把通知送到电脑；可选 App 只做局域网接收 |

iOS 被控、应用内 VPN、把通知 URL 当命令执行，都不是目标。EasyTier 仅侧车，不进 `Cargo.toml`。
