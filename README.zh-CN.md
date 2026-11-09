<!-- markdownlint-disable-file MD033 MD045 -->
<div align="center">

# Tetherly

**工业级跨终端设备互联引擎、硬件级虚拟键鼠漫游与多端协同运行时**

[![License](https://img.shields.io/badge/license-Apache_2.0_OR_MIT-blue.svg?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.80+-orange.svg?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Crates](https://img.shields.io/badge/Crates-6%20Modular%20Crates-purple.svg?style=flat-square)](Cargo.toml)
[![Security](https://img.shields.io/badge/Crypto-SPAKE2%20%7C%20Noise%20IK-success.svg?style=flat-square)](crates/tetherly-crypto)
[![Tests](https://img.shields.io/badge/tests-773%20passed-brightgreen.svg?style=flat-square)]()

[English](./README.md) · [简体中文](./README.zh-CN.md) · [核心特性](#核心特性) · [系统架构](#系统架构全景) · [Crate 拓扑](#工作区-crate-生态拓扑) · [快速上手](#快速上手) · [屏幕串流](#自适应屏幕串流流水线) · [安全模型](#密码学与安全架构)

<br>

<p align="center">
  <b>彻底打破跨系统生态壁垒与单机设备凭证孤岛的两难困局。</b><br>
  在纯粹硬核的 Rust 原生运行时中，将局域网零配置直连、Overlay 网格侧车、验证码硬件级隔离提取与桌面共享键鼠完美融为一体。<br>
  原生集成抗中间人攻击的 SPAKE2 与 Noise IK 前向安全会话，具备微秒级感知的空间瓦片去重与局部运动矢量复用屏幕串流，以及跨平台免 App 接入的 ANCS 蓝牙通知中枢。
</p>

</div>

---

## 痛点与架构对比

异构桌面与移动设备的协同工具常年陷入碎片化泥潭：商业套件强制绑定中心化云端账号与订阅收费；Synergy/Barrier 等开源 KVM 工具缺少移动端通知与验证码提取；KDE Connect 则深度耦合沉重的桌面环境框架。Tetherly 将底层设备协作解耦为无头自愈的独立后台守护进程：

| 评估维度 | Tetherly 互联运行时 | Barrier / Synergy | KDE Connect | 商业闭源远控套件 |
| :--- | :--- | :--- | :--- | :--- |
| **鉴权与信任链** | SPAKE2 零知识 PIN + Noise IK 前向安全通道 | 预共享对称密钥或明文 TLS | 自签名证书手动校验指纹 | 强制登录厂商账号与云端中继 |
| **键鼠共享 (KVM)** | 亚毫秒级平滑越界漫游，支持无感反向控制 | 仅限桌面端，需手动编写布局 | 基于 JSON 文本透传基础操作 | 经过云端回传，延迟高且易卡顿 |
| **移动端深度融合** | 原生 Android JNI 引擎 + iPhone 免 App 蓝牙接收 | 无移动端支持 | 基于庞大 Qt/C++ 构建手机端 | 闭源私有伴侣应用 |
| **低延迟屏幕串流** | 空间瓦片分解、Zstd 无损/H.264 自适应、运动矢量复用 | 无屏幕功能 | 仅有基础屏幕镜像插件 | 闭源有损压缩录屏 |
| **验证码安全防护** | 本地高置信度离线提取，绝不外发短信原文 | 无短信感知 | 全量通知明文向所有设备广播 | 无 |
| **运行时资源占用** | 纯 Rust 无头守护进程（<20MB RSS），回环 HTTP 控制 | 强依赖 Qt 桌面 GUI 框架 | 依赖庞大 Qt/KDE 运行时库 | 庞大的 Electron / Web 壳层 |
| **异地组网能力** | 原生 LAN 直连优先 + EasyTier 无状态侧车组网 | 仅限本地局域网 | 仅限本地局域网 | 强制走厂商中心化转发服务器 |

---

## 核心特性

| 模块 | 特性亮点 | 技术实现与工程优势 |
| :--- | :--- | :--- |
| **SPAKE2 与 Noise IK** | 强健的双向鉴权与前向安全 | 使用 8 位临时 PIN 码经 Argon2id 派生进行 SPAKE2 零知识协商，建立 256 位 Noise IK 会话，彻底消除中间人窃听风险。 |
| **虚拟外设越界漫游** | 硬件级无缝共享鼠标与键盘 | 四向屏幕边缘无缝穿越，支持按比例绝对坐标映射、防误触拖拽锁定以及对端断线时光标自动安全归位。 |
| **自适应屏幕串流** | 超低延迟与动态画质调节 | 4D 动态矩阵协调器自适应切换 Zstd 无损与 H.264 编码，集成选择性 NACK 重传恢复与空间瓦片缓存去重。 |
| **局部运动矢量复用** | 极限降低动态画面网络开销 | 毫秒级识别局部画面位移（`TILE_SUBCODEC_MOTION`），仅发送 5 字节位移矢量，压缩比高达 3276:1（节省 99.9% 带宽）。 |
| **原生 ANCS 蓝牙中枢** | 苹果手机免装 App 接收通知 | 电脑端直接监听 Apple Notification Center Service，原生覆盖 WinRT（Windows）、CoreBluetooth（macOS）与 BlueZ（Linux）。 |
| **本地验证码防泄密** | 智能离线正则与启发式过滤 | 毫秒级提取 4~8 位高置信度验证码候选，并在本机自动过期清理，绝不在网络中回传原始短信正文。 |
| **断点续传文件通道** | 高吞吐大文件点对点分片流 | 二进制数据流通道内置增量 SHA-256 校验、严格对齐的偏移量重开机制与多级流控令牌桶。 |
| **双树全同构 CLI** | 完备的无头控制与脚本接入 | 随二进制附带 `tetherly-cli`，经由带严格超时防护的回环 HTTP/SSE API 提供 485 条经过契约校验的命令路径。 |

---

## 系统架构全景

```text
┌────────────────────────────────────────────────────────────────────────┐
│                        本机客户端与运维操作端                          │
│   ├─ 回环管理 Web 控制台 (http://127.0.0.1:45716)                      │
│   └─ tetherly-cli (485 条全功能命令，走本机回环 HTTP/SSE)              │
└───────────────────────────────────┬────────────────────────────────────┘
                                    │ 本机严格回环 REST & SSE (端口 45716)
                                    ▼
┌────────────────────────────────────────────────────────────────────────┐
│                   Tetherly Node 核心守护进程 (端口 45717)              │
│   ┌─────────────────────┐ ┌──────────────────────┐ ┌────────────────┐   │
│   │ SPAKE2 / Noise IK   │ │ 虚拟外设控制中枢     │ │ 屏幕串流流水线 │   │
│   │ 会话协调与防重放    │ │ 边缘漫游与拖拽锁定   │ │ 4D矩阵 / FEC恢复│   │
│   └──────────┬──────────┘ └──────────┬───────────┘ └────────┬───────┘   │
│              │                       │                      │           │
│   ┌──────────┴───────────────────────┴──────────────────────┴───────┐   │
│   │              tetherly-core (底层通用协议与状态机调度)           │   │
│   │   验证码识别 · 文件通道 · 令牌桶限流 · 空间瓦片编解码           │   │
│   └──────────────────────────────────┬──────────────────────────────┘   │
└──────────────────────────────────────┼──────────────────────────────────┘
                                       │ 经 Noise IK 加密的端对端传输链路
        ┌──────────────────────────────┼──────────────────────────────┐
        ▼                              ▼                              ▼
┌───────────────┐              ┌───────────────┐              ┌───────────────┐
│ 已配对的电脑  │              │ 安卓智能手机  │              │ 苹果 iPhone   │
│ (Desktop 对端)│              │ (JNI 原生客户端)             │ (ANCS 蓝牙信标)│
└───────────────┘              └───────────────┘              └───────────────┘
```

---

## 工作区 Crate 生态拓扑

代码库由 6 个专职 Crate 与 3 个独立构建二进制组成，维持严格的分层边界：

| Crate / 二进制 | 物理目录 | 架构职责与技术细节 |
| :--- | :--- | :--- |
| **`tetherly-core`** | `crates/tetherly-core` | 零 OS 系统调用依赖的核心协议、帧定义、状态机、验证码启发式解析、令牌桶限流与空间瓦片算法。 |
| **`tetherly-crypto`** | `crates/tetherly-crypto` | Ed25519 身份密钥、X25519 静态协商、Argon2id 散列、SPAKE2 零知识配对与 Noise IK 状态转换。 |
| **`tetherly-net`** | `crates/tetherly-net` | TCP 监听套接字、SPAKE2 握手泵、Noise IK 加密会话流、局域网 mDNS 广播以及流式文件传输管道。 |
| **`tetherly-loopback`**| `crates/tetherly-loopback` | 共享回环 HTTP/SSE 客户端，具备严格的超时守护（连接 5s、请求 30s、握手 10s、空闲看门狗 60s）。 |
| **`tetherly-node`** | `crates/tetherly-node` | 宿主守护进程运行时、落盘持久化、本地回环 REST 接口，生成主二进制 `tetherly`。 |
| **`tetherly-android`** | `crates/tetherly-android` | Android JNI 原生绑定、客户端会话状态机（产出 `libtetherly_android.so`）与真机探针工具。 |
| **`tetherly-cli`** | `bins/tetherly-cli` | 拥有 485 条命令路径的命令行客户端，操作正在运行的本地后台节点。 |

---

## 快速上手

### 1. 源码编译

请确保本地已安装 Rust 1.80 或更高版本：

```bash
# 克隆代码仓库
git clone https://github.com/dengyie/tetherly.git
cd tetherly

# 以 Release 模式编译工作区内全部二进制
cargo build --release --workspace
```

编译生成的可执行文件位于 `./target/release/`：
- `tetherly`（桌面节点守护进程）
- `tetherly-cli`（全功能命令行工具）
- `android_probe`（真机诊断探针）

### 2. 启动宿主守护进程

```bash
# 启动首个节点（监听对端控制端口 45717，本机控制 UI 端口 45716）
./target/release/tetherly --data-dir ./host-data --name "主机-A" run
```

### 3. 生成临时配对 PIN 码

在同一台机器的另一个终端窗口中执行：

```bash
# 向正在运行的守护进程申请一组 8 位临时配对码（有效期 3 分钟）
./target/release/tetherly pin
```

```text
Tetherly Pairing PIN: 8492-0193
Valid for: 180 seconds
```

> **配对原则**：`tetherly pin` 必须在守护进程已启动的状态下执行。PIN 码仅由持有它的运行实例识别，连错 5 次将触发 15 分钟安全锁定。

### 4. 从第二台设备发起配对

在第二台电脑或设备上：

```bash
# 使用对端 IP 地址与生成的 PIN 码完成安全配对
./target/release/tetherly-cli pair --address 192.168.1.50:45717 --pin 84920193
```

配对完成后，两端会自动持久化身份公钥（`id_pk` 与 `n_pk`）。后续再次互联时，两端通过前向安全的 Noise IK 握手毫秒级自动重连，无需再次输入 PIN 码。

---

## 自适应屏幕串流流水线

屏幕引擎根据链路抖动与 CPU 负载动态调整画质与压缩率：

```text
画面捕获 (DXGI / ScreenCaptureKit / X11)
  │
  ▼
脏矩形分析器与损伤检测
  │
  ├─► 空间瓦片网格分解 (32x32px 瓦片)
  │     ├─ 瓦片哈希比对 (基于历史画布的毫秒去重)
  │     ├─ 局部运动矢量位移 (5 字节位移指令复用像素)
  │     └─ 显著性分级调度与调色板动态压缩
  │
  ├─► 无损通道: Zstandard 高速整帧打包 (TMV1)
  └─► 低带宽通道: H.264 关键帧/增量帧动态码率调节
        │
        ▼
FEC 前向纠错与平滑发包看门狗 (消减突发抖动与排队膨胀)
  │
  ▼
经 Noise IK 加密的 TCP 传输套接字 (端口 45717)
```

- **已交付帧单调水位线**：通过 `delivered_seq` 水位线杜绝陈旧分片因乱序晚到而复活已交付帧，彻底阻断参考画布污染。
- **选择性 NACK 快速恢复**：仅针对丢失的数据分片发起重传请求，避免全帧重传占用宝贵信道。

---

## 密码学与安全架构

1. **零知识口令双向认证**：设备首次配对执行 SPAKE2 协议，借助 Argon2id 派生群元。明文 PIN 绝不在信道传输，抗暴力破解并设 5 次错误锁定。
2. **Noise IK 前向安全通道**：配对后日常通信基于 `Noise_IK_25519_ChaChaPoly_BLAKE2s`，发起方在持有对端静态公钥的前提下完成带前向保密性的毫秒握手。
3. **回环地址硬隔离**：管理接口（`http://127.0.0.1:45716`）严格绑定本机回环地址，杜绝外网渗透请求。
4. **敏感机密零落地**：验证码在内存中提取并即时销毁，短信明文与私钥绝不以纯文本明文落盘或写入系统日志。

---

## 回环 REST 控制接口参考

正在运行的守护进程在 `http://127.0.0.1:45716` 提供纯粹的 JSON API 与 SSE 数据流：

| 接口端点 | 请求方法 | 功能说明 |
| :--- | :--- | :--- |
| `/api/status` | `GET` | 查询节点身份、开机时间、当前活动对端与连接健康度。 |
| `/api/pin` | `POST` | 申请生成或吊销当前 3 分钟有效期的临时配对 PIN。 |
| `/api/peers` | `GET` | 列出已发现和已连接的对端设备、往返延迟 (RTT) 与路由表。 |
| `/api/clip` | `GET`/`POST`| 读取剪贴板环形历史记录或主动同步一条剪贴板内容。 |
| `/api/files/offer` | `POST` | 向指定对端发起一条大文件断点续传邀请。 |
| `/api/screen/start` | `POST` | 申请对端授权并拉起低延迟屏幕串流服务。 |
| `/api/ancs/status` | `GET` | 检查 Apple Notification Center 蓝牙外设的连接状态。 |
| `/api/events` | `GET` | 订阅全量拓扑变更、验证码提取与剪贴板事件的 SSE 长连接。 |

---

## 参与贡献与许可证

欢迎提交 Issue 与 Pull Request！在提交代码前，请确保全量测试与代码静态检查通过：

```bash
# 运行工作区全量单元测试与集成测试
cargo test --workspace

# 执行最严格的 Clippy 静态检查
cargo clippy --workspace --all-targets -- -D warnings

# 校验 Rust 代码格式化规范
cargo fmt --all -- --check
```

本项目采用双重开源许可证授权（任选其一）：
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))
