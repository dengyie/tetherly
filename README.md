# Tetherly

跨终端设备互联:局域网直连,跨网走 mesh,验证码本机提取,桌面共享一套键鼠。

仓库与 crate 名一律使用 `tetherly`。许可证 **Apache-2.0 OR MIT**。实现合同是 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)。

## 当前状态

五个 crate + 两个二进制(`tetherly` 桌面节点、`android_probe` 真机探针)。无 GUI(本机 UI 是 loopback HTTP);ANCS BLE 接收栈三平台齐备(Windows WinRT / macOS CoreBluetooth / Linux BlueZ,真机 Manual-required);屏幕串流默认 zstd 无损,H.264 有损可经 `--screen-codec h264` + 线上协商启用(ADR-079);EasyTier 仅侧车。

| crate | 职责 |
|---|---|
| `tetherly-core` | OTP、帧、Hello、状态机、去重、白名单 |
| `tetherly-crypto` | Ed25519 + X25519、Argon2id、SPAKE2、PairBind、Noise IK |
| `tetherly-net` | TCP Hello / SPAKE2 / Noise;可选 LAN mDNS |
| `tetherly-node` | LAN 节点、落盘、hubs、本机 HTTP UI;产出 `tetherly` 二进制 |
| `tetherly-android` | 真身份 + Noise 会话 → `libtetherly_android.so` + `android_probe` |
| `tetherly-cli` | Phase 0 配对夹具(`listen` / `dial` / `pin` / `identity`),不进生产路径 |

```text
# 终端 A:起节点(控制 45717,本机 UI 127.0.0.1:45716)
tetherly --data-dir ./host --name host run

# 终端 A:另开一个 shell,显示 8 位 PIN(3 分钟有效)
tetherly pin
```

`tetherly pin` **必须在节点已经运行时执行**:它通过本机 UI API 给**正在跑的那个进程**装 PIN。PIN 只对持有它的进程有意义——另起一个进程打印的码,没有任何监听者会接受。

本机 UI 在 http://127.0.0.1:45716 ,配对、候选验证码、剪贴板与远程屏幕都在这里操作。手机端装 APK 后填桌面地址 + PIN,只拨 45717。

不要把 PIN 发给任何人。错 PIN 5 次锁定 15 分钟。

```text
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Windows GNU 链接需要较新的 MinGW(Scoop `mingw` 16+)。不要用 MinGW.org 6.3:它的 `ld` 不认识 `--high-entropy-va`。

## 平台承诺(v1)

| 端 | 角色 |
|---|---|
| Windows / macOS | 全功能节点 |
| Linux | 全功能,Wayland 键鼠为 beta |
| Android | 通知/验证码源 + 文件/剪贴板 |
| iPhone | ANCS 把通知送到电脑;可选 App 只做局域网接收 |

iOS 被控、应用内 VPN、把通知 URL 当命令执行,都不是目标。EasyTier 仅侧车,不进 `Cargo.toml`。