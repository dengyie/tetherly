# ADR-112 屏幕串流渐进式帧内平滑刷新、带状宏块去脉冲流控流水线与双端 CLI 全同构 (Screen Streaming Progressive Intra-Refresh, Anti-Burst Band Slice Refresh Pipeline with 100% Dual CLI Isomorphism)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-29，v2.26 / v0.1.48）**——建立屏幕串流渐进式帧内平滑刷新、带状宏块去脉冲流控流水线：(1) **渐进式帧内平滑刷新模型与状态机**（`crates/tetherly-core/src/screen.rs`）：定义 `ScreenPirMode`（Disabled/0, Continuous/1, OnDemand/2）、`ScreenPirConfig` 与 `ScreenPirReport`，实现 `ScreenPirCalculator` 带状切片计算算法；(2) **去脉冲平滑流控与带状宏块切片几何划分**：将屏幕垂直切分为 $N$ 个水平宏块切片（$2 \le N \le 32$，默认 $N = 8$），逐帧推进 Intra-Slice 帧内刷新带，消除传统全量 IDR 关键帧高达 15~50 倍的突发流量峰值（Traffic Burst），彻底消除网络缓冲膨胀（Bufferbloat）、瞬态丢包与客户端画面冻结（100~300ms Freeze）；(3) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 27（`ControlMsg::SetScreenPirMode { mode: u8, bands: u8 }`）与 Tag 28（`ControlMsg::ResetScreenPirMode`）；(4) **节点运行时切片推进与网络循环挂接**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_pir_config` 与切片进度计数器（`sweeps_completed`、`slices_sent`、`current_band`），网络事件循环双向同步 Tag 27/28，帧发送时协同差分与参考画布推进切片；(5) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/pir`（GET / POST）、`/api/screen/pir/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充 PIR 遥测指标，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen pir [status|set|reset]` 子命令族与格式化报表输出，`screen status` 扩充 PIR 遥测；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_36_screen_progressive_intra_refresh_pipeline` 全链路验证。 |
| 日期 | 2026-10-29 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-087「屏幕串流播放抖动缓冲与平滑控制器」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-091「屏幕串流报文定步与突发整形令牌桶」、ADR-109「屏幕串流动态差分补丁与宏块缓存流水线」、ADR-111「屏幕串流客观 QoE 自主闭环调优流水线」 |

---

## 动机

1. **全量关键帧脉冲冲击 (I-Frame Burst Spike) 痛点**：
   - 在已落地的增量差分补丁（ADR-109）、客观画质质检（ADR-110）与 QoE 自主闭环（ADR-111）中，当网络轻微丢包、新访客接入、参考画布漂移或 QoE 控制器触发 `ForceKeyframe` 时，系统必须派发 `ControlMsg::KeyframeRequest`（Tag 3）请求全量 IDR 关键帧刷新；
   - 然而，未经切片的全量 1080p/4K 画面数据量达到通常差分增量帧的 15~50 倍，瞬间向链路注入巨大脉冲流量（Traffic Burst）；
   - 该突发脉冲会瞬间击穿家用无线网络、4G/5G 弱网或 EasyTier 虚拟 Mesh 隧道缓冲队列，引发严重缓冲膨胀（Bufferbloat，RTT 骤增至数百毫秒）并导致尾部丢包，直接击穿接收端播放抖动缓冲（Dejitter Buffer，ADR-087），造成用户端肉眼可见的“关键帧卡顿 / 顿挫冻结（100~300ms Freeze）”。

2. **渐进式帧内平滑刷新 (Progressive Intra-Refresh, PIR)**：
   - 将整屏垂直方向分割为 $N$ 个水平宏块带状切片（$2 \le N \le 32$，默认 $N = 8$）；
   - 在每个 PIR 串流帧中，仅将第 $k$ 个切片（$k = \text{frame\_seq} \pmod N$）作为全量 Intra-Slice 高保真帧内刷新，其余 $N-1$ 个切片维持常规帧间差分或跳帧；
   - 经过连续 $N$ 帧后，整个屏幕在不产生任何带宽脉冲尖峰的情况下完成 100% 全局无缝刷新（Zero-Burst Smooth Refresh）。

3. **PIR 运行策略模式 (`ScreenPirMode`)**：
   - `Disabled` (0)：默认传统全量关键帧模式；
   - `Continuous` (1)：持续滚动渐进刷新（每一帧推进 1 个切片，消除周期性 I 帧波峰）；
   - `OnDemand` (2)：按需触发式渐进刷新（在收到刷新指令或 QoE 触发时，平滑执行一次完整的 $N$ 帧扫描后自动休眠）。

4. **切片几何运算与边界吸收 (Band Slice Geometry)**：
   - 设画面尺寸为 $(W, H)$，切片数 $N$：基准切片高度 $h_b = H / N$；
   - 前 $N-1$ 个切片覆盖 $y \in [k \cdot h_b, (k+1) \cdot h_b)$，第 $N-1$ 个切片自动吸收除不尽的余数行像素至 $H$；
   - 严格遵循 ADR-006 零重型外部依赖铁律，纯 Rust 标准库切片与内存索引运算。

5. **全链路协同与双端 CLI 100% 同构**：
   - 遵循 Tetherly 架构铁律，通过定长信令（Tag 27/28）、回环 REST API（`/api/screen/pir`）以及双端 CLI（`screen pir`）实现完整能力对称与可观测性。

---

## 决定

### 1. PIR 数据模型与切片算法（`crates/tetherly-core/src/screen.rs`）

- **数据模型**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "lowercase")]
  pub enum ScreenPirMode {
      #[default]
      Disabled = 0,
      Continuous = 1,
      OnDemand = 2,
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenPirConfig {
      pub mode: ScreenPirMode,
      pub bands: u8, // 2..=32, default: 8
  }

  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenPirReport {
      pub ok: bool,
      pub mode: String,
      pub bands: u8,
      pub sweeps_completed: u64,
      pub slices_sent: u64,
      pub current_band: u8,
      pub active_sweep: bool,
  }
  ```

- **切片几何运算**：
  ```rust
  pub fn compute_pir_band_rect(width: u32, height: u32, bands: u8, band_index: u8) -> (u32, u32, u32, u32) {
      let b = bands.clamp(2, 32) as u32;
      let idx = (band_index as u32) % b;
      let base_h = height / b;
      let y = idx * base_h;
      let h = if idx == b - 1 {
          height - y
      } else {
          base_h
      };
      (0, y, width, h)
  }
  ```

### 2. 定长线协议信令扩展（`TYPE_SCREEN_CONTROL = 0x0502`）

- **Tag 27 (`ControlMsg::SetScreenPirMode { mode: u8, bands: u8 }`)**：
  - `seq` 编码：`mode as u64 | ((bands as u64) << 8)`；
  - 解码时 `mode` 解析为 `ScreenPirMode`，`bands` 钳位于 `2..=32`（为 0 时恢复默认值 8）；
- **Tag 28 (`ControlMsg::ResetScreenPirMode`)**：
  - `seq` 编码：`0u64`；
  - 远端接收后将 PIR 模式重置为 `Disabled`，切片进度复位。

### 3. 节点运行时与切片流控流水线（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载 `screen_pir_config: Arc<RwLock<ScreenPirConfig>>` 与原子统计计数器：
  - `screen_pir_sweeps_completed: AtomicU64`
  - `screen_pir_slices_sent: AtomicU64`
  - `screen_pir_current_band: AtomicU8`
  - `screen_pir_active_sweep: AtomicBool`
- 帧发送循环（`screen_send_frame`）：
  - 当 PIR 模式处于 `Continuous` 或 `OnDemand` 且 `active_sweep` 为真时：
    - 读取当前 `current_band`，计算 `(x, y, w, h)`；
    - 递增 `slices_sent`，并将 `current_band` 推进至 `(current_band + 1) % bands`；
    - 若 `current_band` 归零，说明一轮完整扫描完成，递增 `sweeps_completed`；若处于 `OnDemand`，则重置 `active_sweep = false`。

### 4. 回环 REST 控制面 API（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/pir`：返回当前 PIR 配置与进度统计报表（`ScreenPirReport`）；
- `POST /api/screen/pir`：通过 JSON 体（`{"mode": "continuous", "bands": 8}`）或 URL 参数（`?mode=continuous&bands=8`）配置模式与切片数，并通过活动通道向对端发送 Tag 27 信令；
- `POST/DELETE /api/screen/pir/reset`：重置 PIR 状态为 `Disabled`，向对端发送 Tag 28 复位信令；
- `ScreenStatusReport` 扩充 `pir_enabled`、`pir_mode`、`pir_bands`、`pir_sweeps_completed` 字段；
- 安全防护：严格执行 127.0.0.1 物理绑定与 Host 请求头验证。

### 5. 双端 CLI 100% 全同构（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）

- 完整支持如下命令：
  ```bash
  tetherly screen pir status [--json] [--ui-port <port>]
  tetherly screen pir set --mode <continuous|on-demand|disabled> [--bands <count>] [--json] [--ui-port <port>]
  tetherly screen pir reset [--json] [--ui-port <port>]
  ```
- 格式化报表函数 `print_screen_pir_report` 对称呈现；
- `screen status` 对称扩充 PIR 遥测输出。

---

## 验证与安全边界

1. **零重型依赖约束 (ADR-006)**：
   - 切片几何算法由纯标准库数学运算完成，零第三方图像处理库引入；
2. **回环安全约束**：
   - 保持 127.0.0.1 绑定与 Host 请求头检查，拒绝任何非本地未授权请求；
3. **集成与端到端测试**：
   - core 单测：`screen_progressive_intra_refresh_algorithms_and_signaling` 验证切片坐标与余数吸收；
   - REST 单测：`test_uihttp_screen_pir_endpoints` 验证回环 REST 接口全生命周期与 403 阻断；
   - Phase 5 集成测试：`m5_36_screen_progressive_intra_refresh_pipeline` 验证真实屏幕流转下的 Tag 27/28 信令传播、PIR 激活、切片统计递增、完整轮次完成与复位清零。
