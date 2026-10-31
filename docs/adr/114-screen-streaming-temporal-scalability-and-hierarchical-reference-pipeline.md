# ADR-114 屏幕串流双层时域分级预测 (SVC-T) 层次化参考与抗拥塞优雅降级流水线 (Screen Streaming 2-Layer Temporal Scalability (SVC-T) Hierarchical Reference and Congestion-Resilient Graceful Degradation Pipeline)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-29，v2.28 / v0.1.50）**——建立屏幕串流双层时域分级预测 (SVC-T) 层次化参考与抗拥塞优雅降级流水线：(1) **双层层次化时域分级模型与参考不变式**（`crates/tetherly-core/src/screen.rs`）：定义 `ScreenSvcMode`、`ScreenSvcConfig` 与 `ScreenSvcReport`，新增专用 Codec 标记 `SCREEN_COMPRESS_SVC_T1 = 6`；将串流帧解耦为偶数帧的基础层 (Base Layer, T0) 与奇数帧的增强层 (Enhancement Layer, T1)；T0 帧仅参考前一个 T0 帧，T1 帧仅参考最近的 T0 帧，确立“没有任何后续帧依赖 T1”的根本参考不变式；(2) **抗拥塞瞬时优雅降级与零重协商跳帧**：在网络遭遇瞬时拥塞、队列延迟上升或低带宽节流时，发送端直接丢弃 T1 增强帧，接收端平滑降至 30fps 解码 T0 帧，实现零撕裂、零花屏、零残影，彻底避免传统降帧率引发的差分依赖链断裂与重型关键帧/LTR 自愈流量冲击；拥塞恢复后即刻恢复 60fps 满帧率；(3) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 33（`ControlMsg::SetScreenSvcMode { mode: u8, drop_t1: bool }`）、Tag 34（`ControlMsg::ResetScreenSvcMode`）与 Tag 35（`ControlMsg::SvcT1DropSync { dropped_count: u64 }`）；(4) **节点运行时状态维护与网络循环协同**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_svc_config`、基础层参考画布（`screen_sender_t0_canvas` 与 `screen_receiver_t0_canvas`）及流控计数器（`t0_frames_sent`、`t1_frames_sent`、`t1_frames_dropped`、`saved_bandwidth_bytes`），在 `screen_send_frame()` 阶段动态执行层次化差分编码与 T1 智能丢帧，接收端解码 T1 时严格隔离基础参考画布；(5) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/svc`（GET / POST）、`/api/screen/svc/drop-t1`（POST）、`/api/screen/svc/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充 SVC 遥测指标，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen svc [status|set|drop-t1|reset]` 子命令族与格式化报表输出，`screen status` 扩充 SVC 遥测；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_38_screen_temporal_scalability_svc_pipeline` 全链路验证。 |
| 日期 | 2026-10-29 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-083「屏幕串流动态帧率与自适应定步控制」、ADR-087「屏幕串流播放抖动缓冲与平滑控制器」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-091「屏幕串流报文定步与突发整形令牌桶」、ADR-109「屏幕串流动态差分补丁与宏块缓存流水线」、ADR-111「屏幕串流客观 QoE 自主闭环调优流水线」、ADR-112「屏幕串流渐进式帧内平滑刷新与去脉冲流控流水线」、ADR-113「屏幕串流长期参考帧 (LTR) 锚定与增量断洞自愈流水线」 |

---

## 动机

1. **平坦时间差分预测链的脆弱性与弱网拥塞困境**：
   - 在既有的增量差分补丁（ADR-109）中，串流帧采用平坦的时间差分预测模式（$F_t \leftarrow F_{t-1} + \Delta_t$），即每一帧都强依赖前一物理帧；
   - 在无线 Wi-Fi 竞争、4G/5G 弱网或 EasyTier 虚拟 Mesh 隧道中，网络带宽随信道干扰经常发生剧烈波动，引发瞬时拥塞（Micro-Congestion）；
   - 此时发送端面临严重的两难困境：
     - **若盲目跳帧降频**：直接丢弃某些帧会立刻打断接收端的差分依赖链，导致画面撕裂或迫使接收端派发重型刷新指令（IDR 关键帧 Tag 3 或 LTR 恢复请求 Tag 31）；
     - **若维持原频发送**：超额数据帧在网络队列中堆积，造成严重的网络缓冲膨胀（Bufferbloat，RTT 骤增至数百毫秒），产生滚雪球式端到端时延崩塌（Lag Snowball）。

2. **双层时域分级预测 (Temporal Scalability, SVC-T) 解法**：
   - 借鉴现代实时视频编码（H.264 SVC / VP9 SVC / AV1）的时域分级架构，在无重型外部编码器依赖（ADR-006）的前提下，构建轻量高效的双层时域分级结构：
     - **基础层 (Base Layer, T0)**：包含偶数帧（$seq \pmod 2 == 0$），代表 30fps 基准刷新。每个 T0 差分帧**严格且仅参考前一个 T0 基础帧**；
     - **增强层 (Enhancement Layer, T1)**：包含奇数帧（$seq \pmod 2 == 1$），负责将刷新率提升至满血 60fps。每个 T1 差分帧**严格参考最近的一个 T0 基础帧**；
   - **核心参考不变式 (Fundamental Invariant)**：
     $$\forall t, \quad F_{t} \text{ 不依赖任何 } F_{\tau} \text{ 其中 } \tau \in T_1$$
     **没有任何后续帧（无论是未来的 T0 还是 T1）会将 T1 帧作为参考基准**！

3. **抗拥塞瞬时优雅降级 (Zero-Renegotiation Graceful Degradation)**：
   - 当探测到网络拥塞、队列排队增加或触发省流模式时，发送端只需**直接静默丢弃 T1 增强帧**（Drop T1）；
   - 接收端依然按 30fps 节奏顺畅接收并解码 T0 帧。由于 T0 的差分依赖链完全独立且自洽，画面**零伪影、零撕裂、零花屏，且绝对不需要重新协商编码参数或请求重型关键帧刷新**；
   - 网络恢复后，发送端立即恢复推送 T1 帧，接收端瞬时回归 60fps 平滑满帧体验。

4. **系统约束与安全红线**：
   - 严格遵循 ADR-006 零重型图像/外部依赖铁律，全算法采用标准库切片与内存运算；
   - 遵循 ADR-004 对端主权与安全强隔离原则，所有回环控制面端点严格绑定 127.0.0.1 并强制 Host 请求头校验，非法访问返回 403 Forbidden；
   - 严格保持双端 CLI（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）100% 同构。

---

## 决定

### 1. SVC-T 数据模型与 Codec 标记（`crates/tetherly-core/src/screen.rs`）

- **专用 Codec 标记**：
  ```rust
  pub const SCREEN_COMPRESS_SVC_T1: u8 = 6;
  ```

- **运行模式与配置数据结构**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub enum ScreenSvcMode {
      Disabled = 0,
      Adaptive = 1,
      BaseOnly = 2,
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenSvcConfig {
      pub mode: ScreenSvcMode,
      pub drop_t1: bool,
  }

  #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenSvcReport {
      pub ok: bool,
      pub enabled: bool,
      pub mode: String,
      pub drop_t1: bool,
      pub t0_frames_sent: u64,
      pub t1_frames_sent: u64,
      pub t1_frames_dropped: u64,
      pub saved_bandwidth_bytes: u64,
  }
  ```

### 2. 14 字节定长线协议信令定义（`TYPE_SCREEN_CONTROL = 0x0502`）

信令沿用 14 字节定长信封（`magic: b"TSCR"`，`proto: 1`，`tag: u8`，`seq: 8 字节大端整数`）：

| 信令枚举 | Tag | 载荷字段 | Wire `seq` (8 字节大端) 编码规则 |
|---|---|---|---|
| `ControlMsg::SetScreenSvcMode { mode, drop_t1 }` | **33** | `mode: u8`, `drop_t1: bool` | `(mode as u64) \| (if drop_t1 { 1u64 << 8 } else { 0 })` |
| `ControlMsg::ResetScreenSvcMode` | **34** | 无 | `0u64` |
| `ControlMsg::SvcT1DropSync { dropped_count }` | **35** | `dropped_count: u64` | `dropped_count` |

### 3. 节点运行时状态与网络循环协同（`crates/tetherly-node/src/runtime.rs`）

1. **`NodeShared` 状态挂载**：
   - `screen_svc_config: Arc<RwLock<ScreenSvcConfig>>`
   - `screen_svc_t0_frames_sent: AtomicU64`
   - `screen_svc_t1_frames_sent: AtomicU64`
   - `screen_svc_t1_frames_dropped: AtomicU64`
   - `screen_svc_saved_bytes: AtomicU64`
   - `screen_sender_t0_canvas: Arc<RwLock<Option<ScreenFrame>>>`
   - `screen_receiver_t0_canvas: Arc<RwLock<Option<ScreenFrame>>>`

2. **帧发送调度流水线 (`screen_send_frame`)**：
   - 当 `svc_config.mode != Disabled`：
     - 若当前物理帧序号为奇数（$seq \pmod 2 == 1$），该帧判定为 **T1 增强帧**：
       - 若 `drop_t1` 为真或 `mode == BaseOnly`：发送端累加 `screen_svc_t1_frames_dropped` 与预估节省字节 `screen_svc_saved_bytes`，直接略过本帧发送；
       - 若正常发送：发送端基于 `screen_sender_t0_canvas` 提取差分增量补丁，打包为 `SCREEN_COMPRESS_SVC_T1 (6)` 格式派发，递增 `screen_svc_t1_frames_sent`；
     - 若当前物理帧序号为偶数（$seq \pmod 2 == 0$），该帧判定为 **T0 基础帧**：
       - 基于上一个 `screen_sender_t0_canvas` 生成差分补丁（或基准关键帧），更新本地 `screen_sender_t0_canvas`，递增 `screen_svc_t0_frames_sent`；

3. **接收端分层解码与画布隔离 (`screen_client_loop`)**：
   - 当接收到 `SCREEN_COMPRESS_SVC_T1`：
     - 将差分增量补丁直接叠加到前台展示画布 `screen_receiver_canvas` 并渲染；
     - **坚决不修改 `screen_receiver_t0_canvas`**，确保基础参考画布纯净锁定在最近的 T0 帧；
   - 当接收到 T0 帧（如全量帧或标准差分帧）：
     - 将其解码并保存至 `screen_receiver_t0_canvas`，同步刷新前台展示画布 `screen_receiver_canvas`。

### 4. 回环 REST 控制面接口规范（`crates/tetherly-node/src/uihttp.rs`）

| 端点路径 | HTTP 方法 | 请求体 / 查询参数 | 返回载荷 | 说明 |
|---|---|---|---|---|
| `/api/screen/svc` | `GET` | 无 | `ScreenSvcReport` | 查询当前 SVC-T 运行模式、分层发送与丢帧统计 |
| `/api/screen/svc` | `POST` | `{"mode": "adaptive", "drop_t1": false}` 或 URL 参数 | `ScreenSvcReport` | 动态切换模式与降级策略 |
| `/api/screen/svc/drop-t1` | `POST` | `{"drop": true}` 或 URL 参数 `?drop=true` | `ScreenSvcReport` | 显式触发或恢复 T1 帧丢弃降级 |
| `/api/screen/svc/reset` | `POST` / `DELETE` | 无 | `ScreenSvcReport` | 重置 SVC-T 状态至默认关闭态并清空统计 |

安全红线：严格执行 127.0.0.1 物理绑定与 `Host: 127.0.0.1|localhost` 校验，未通过直接阻断并返回 403 Forbidden。

### 5. 双端 CLI 100% 全同构（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）

对称提供 `screen svc` 子命令：
```text
tetherly screen svc status [--json] [--ui-port <port>]
tetherly screen svc set [--mode <disabled|adaptive|base-only>] [--drop-t1 <bool>] [--json] [--ui-port <port>]
tetherly screen svc drop-t1 [--drop <bool>] [--json] [--ui-port <port>]
tetherly screen svc reset [--json] [--ui-port <port>]
```

---

## 验证与测试策略

1. **核心算法与协议单元测试**（`crates/tetherly-core/src/screen.rs`）：
   - `screen_temporal_scalability_svc_algorithms_and_signaling`：验证模式枚举、Tag 33/34/35 编解码往返及 `SCREEN_COMPRESS_SVC_T1` 帧往返序列化。
2. **回环 REST 控制面测试**（`crates/tetherly-node/src/uihttp.rs`）：
   - `test_uihttp_screen_svc_endpoints`：验证 `/api/screen/svc/*` 各端点正常流转、参数解析、403 恶意 Host 防御。
3. **端到端集成测试**（`tests/phase5.rs`）：
   - `m5_38_screen_temporal_scalability_svc_pipeline`：验证串流建立、启用 SVC Adaptive、T0/T1 交替传输、主动触发 `drop_t1` 验证 T1 弃帧与带宽节省、T0 无缝解码、恢复 T1 满帧率及全链路复位。
