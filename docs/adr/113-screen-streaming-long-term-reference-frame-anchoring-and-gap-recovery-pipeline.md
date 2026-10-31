# ADR-113 屏幕串流长期参考帧 (LTR) 锚定、增量断洞自愈与抗丢包参考帧选择流水线 (Screen Streaming Long-Term Reference (LTR) Frame Anchoring, Gap Invalidation Recovery and Reference Picture Selection Pipeline)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-29，v2.27 / v0.1.49）**——建立屏幕串流长期参考帧 (LTR) 锚定、增量断洞自愈与抗丢包参考帧选择流水线：(1) **长期黄金参考画布与参考帧选择模型**（`crates/tetherly-core/src/screen.rs`）：定义 `ScreenLtrConfig` 与 `ScreenLtrReport`，新增专用 Codec 标记 `SCREEN_COMPRESS_LTR = 5`，支持黄金参考帧锚定与跨序列自愈补丁；(2) **增量断洞自愈与抗丢包 1-RTT 瞬时恢复**：在动态差分补丁（ADR-109）与 PIR 平滑刷新（ADR-112）基础上，解决因中间差分帧丢包造成的连续预测链断裂与画布撕裂；通过保存受确认的黄金画布 $F_{\text{ltr}}$，接收端遭遇断洞时仅需派发 LTR 自愈请求，发送端即刻生成基于黄金画布的增量补丁（$F_{\text{live}} - F_{\text{ltr}}$），1 个 RTT 内瞬时无缝恢复，彻底消除全量 IDR 关键帧高达 15~50 倍的突发流量峰值（Traffic Burst）与多帧 PIR 等待延迟；(3) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 29（`ControlMsg::SetScreenLtrMode { enabled: bool, auto_interval: u16 }`）、Tag 30（`ControlMsg::ResetScreenLtrMode`）、Tag 31（`ControlMsg::LtrRecoveryRequest { ltr_seq: u64 }`）与 Tag 32（`ControlMsg::LtrAnchorSync { ltr_seq: u64 }`）；(4) **节点运行时状态维护与网络循环协同**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_ltr_config`、双端黄金参考画布（`screen_sender_ltr_canvas` 与 `screen_receiver_ltr_canvas`）及自愈统计计数器（`anchors_established`、`recoveries_requested`、`recoveries_completed`、`saved_burst_bytes`），帧发送阶段自动执行周期性锚定与 pending 状态自愈补丁派发；(5) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/ltr`（GET / POST）、`/api/screen/ltr/anchor`（POST）、`/api/screen/ltr/recover`（POST）、`/api/screen/ltr/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充 LTR 遥测指标，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen ltr [status|set|anchor|recover|reset]` 子命令族与格式化报表输出，`screen status` 扩充 LTR 遥测；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_37_screen_ltr_long_term_reference_pipeline` 全链路验证。 |
| 日期 | 2026-10-29 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-087「屏幕串流播放抖动缓冲与平滑控制器」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-091「屏幕串流报文定步与突发整形令牌桶」、ADR-109「屏幕串流动态差分补丁与宏块缓存流水线」、ADR-111「屏幕串流客观 QoE 自主闭环调优流水线」、ADR-112「屏幕串流渐进式帧内平滑刷新与去脉冲流控流水线」 |

---

## 动机

1. **增量差分串流中的预测链断裂 (Prediction Chain Invalidation) 痛点**：
   - 在已落地的动态差分补丁（ADR-109）中，串流帧采用高压缩比的时间差分预测（$F_t \leftarrow F_{t-1} + \Delta_t$）；
   - 在真实网络环境（如家用无线 Wi-Fi、4G/5G 弱网、跨国跨城网络或 EasyTier 虚拟 Mesh 隧道）中，报文乱序、网络抖动或尾部丢包难以彻底避免；
   - 一旦中间某个差分帧 $\Delta_k$ 发生丢包或校验损坏，接收端的参考画布（Reference Canvas）即发生永久性基准漂移，导致后续所有差分帧解码呈现严重花屏、撕裂与伪影幽灵残影（Artifact Ghosting）。

2. **传统补救手段的两难困境**：
   - **方案 A：全量 IDR 关键帧刷新 (`KeyframeRequest`, Tag 3 / ADR-081)**：
     - 接收端发现丢包后请求全量关键帧刷新；
     - 全量关键帧体积是差分帧的 15~50 倍，瞬间向拥塞或弱网链路注入巨大脉冲流量（Traffic Burst），直接引发严重缓冲膨胀（Bufferbloat）与二次丢包，并在端侧造成 100~300ms 冻结顿挫；
   - **方案 B：渐进式帧内平滑刷新 (`PIR`, ADR-112)**：
     - 将刷新切片分散到连续 $N$ 帧中推进，消除了带宽尖峰，但在高动态操作或连续丢包场景下，需要等待 $N$ 个帧间隔（通常 250~500ms）才能彻底自愈全部画面。

3. **长期参考帧 (Long-Term Reference, LTR) 与参考帧选择 (Reference Picture Selection, RPS) 解法**：
   - 发送端与接收端协同保留一个已确认无损送达的“黄金参考画布”（Golden LTR Canvas, $F_{\text{ltr}}$），并同步其帧序号 `ltr_seq`；
   - 当接收端检测到差分帧断洞或画布漂移时，不再请求重型 IDR 全量关键帧，而是向发送端派发 **LTR 恢复请求 (`LtrRecoveryRequest`, Tag 31)**；
   - 发送端收到请求后，直接比对当前最新屏幕帧与本地保存的黄金参考画布，生成 **LTR 增量恢复补丁**（$\Delta_{\text{recovery}} = F_{\text{live}} - F_{\text{ltr}}$，使用专用 Codec Tag `SCREEN_COMPRESS_LTR = 5`）；
   - 接收端接收到该补丁后，将其直接叠加到本地黄金参考画布上，**在 1 个 RTT 内瞬时无缝恢复 100% 正确画面，既无 IDR 关键帧突发脉冲，也无需等待多帧 PIR 周期**；
   - 周期性（如每 60 帧）或按需执行黄金参考帧锚定（`LtrAnchorSync`, Tag 32），确保持续高效的增量恢复基准。

4. **零依赖与线协议定长信令**：
   - 严格遵守 ADR-006 铁律，纯 Rust 标准库内存索引与切片计算，无任何外部重型图像或视频依赖；
   - 扩充 14 字节定长信令信封（`TYPE_SCREEN_CONTROL = 0x0502`）中 Tag 29、30、31、32；
   - 提供回环 REST API 与双端 CLI 100% 全同构支持。

---

## 决定

### 1. LTR 数据模型与 Codec 标记（`crates/tetherly-core/src/screen.rs`）

- **新增 Codec Tag 与常量**：
  ```rust
  pub const SCREEN_COMPRESS_LTR: u8 = 5;
  pub const SCREEN_LTR_DEFAULT_AUTO_INTERVAL: u16 = 60;
  pub const SCREEN_LTR_MIN_INTERVAL: u16 = 5;
  pub const SCREEN_LTR_MAX_INTERVAL: u16 = 600;
  ```

- **数据模型**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenLtrConfig {
      pub enabled: bool,
      pub auto_interval: u16, // 5..=600, default: 60 (1s at 60fps), 0 = manual only
  }

  #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenLtrReport {
      pub ok: bool,
      pub enabled: bool,
      pub auto_interval: u16,
      pub current_ltr_seq: u64,
      pub anchors_established: u64,
      pub recoveries_requested: u64,
      pub recoveries_completed: u64,
      pub saved_burst_bytes: u64,
  }
  ```

### 2. 14 字节定长线协议信令定义（`TYPE_SCREEN_CONTROL = 0x0502`）

| 信令枚举 | Tag | 载荷字段 | Wire `seq` (8 字节大端) 编码规则 |
|---|---|---|---|
| `ControlMsg::SetScreenLtrMode { enabled, auto_interval }` | **29** | `enabled: bool`, `auto_interval: u16` | `(enabled as u64) \| ((auto_interval as u64) << 8)` |
| `ControlMsg::ResetScreenLtrMode` | **30** | 无 | `0u64` |
| `ControlMsg::LtrRecoveryRequest { ltr_seq }` | **31** | `ltr_seq: u64` | `ltr_seq` |
| `ControlMsg::LtrAnchorSync { ltr_seq }` | **32** | `ltr_seq: u64` | `ltr_seq` |

### 3. 节点运行时管线与闭环执行（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载状态：
  - `screen_ltr_config: Arc<RwLock<ScreenLtrConfig>>`
  - `screen_ltr_current_seq: AtomicU64`
  - `screen_ltr_anchors_established: AtomicU64`
  - `screen_ltr_recoveries_requested: AtomicU64`
  - `screen_ltr_recoveries_completed: AtomicU64`
  - `screen_ltr_saved_burst_bytes: AtomicU64`
  - `screen_ltr_recovery_pending: AtomicBool`
  - `screen_sender_ltr_canvas: Arc<RwLock<Option<ScreenFrame>>>`
  - `screen_receiver_ltr_canvas: Arc<RwLock<Option<ScreenFrame>>>`
- 发送端逻辑：
  - 若 `screen_ltr_recovery_pending` 为 true，从 `screen_sender_ltr_canvas` 生成 LTR 恢复差分帧（`SCREEN_COMPRESS_LTR`），清空 pending 标志并累加 `recoveries_completed` 与 `saved_burst_bytes`；
  - 若达到 `auto_interval` 或初次关键帧，自动保存当前帧副本至 `screen_sender_ltr_canvas`，递增 `anchors_established`，并向接收端派发 `ControlMsg::LtrAnchorSync`；
- 接收端逻辑：
  - 若收到 `SCREEN_COMPRESS_LTR` 帧，直接应用至 `screen_receiver_ltr_canvas` 并同步刷新 `screen_receiver_canvas`；
  - 若收到 `ControlMsg::LtrAnchorSync`，将接收端当前画布固化存入 `screen_receiver_ltr_canvas`；
- 暴露运行时方法：`screen_ltr_report()`、`screen_set_ltr_mode()`、`screen_anchor_ltr()`、`screen_request_ltr_recovery()`、`screen_reset_ltr_mode()`。

### 4. 回环 REST 控制面接口（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/ltr`：查询当前 LTR 运行状态、黄金参考序号与自愈统计；
- `POST /api/screen/ltr`：动态配置启用状态与自动锚定周期；
- `POST /api/screen/ltr/anchor`：手动触发一次黄金参考帧锚定；
- `POST /api/screen/ltr/recover`：手动派发 LTR 增量恢复请求；
- `POST/DELETE /api/screen/ltr/reset`：重置 LTR 配置与自愈计数器；
- 严格执行 127.0.0.1 物理绑定与 Host 头 403 阻断。

### 5. 双端 CLI 100% 全同构（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）

- 对称扩展 `screen ltr` 命令族：
  - `screen ltr status [--json] [--ui-port <port>]`
  - `screen ltr set [--enabled <bool>] [--interval <n>] [--json] [--ui-port <port>]`
  - `screen ltr anchor [--json] [--ui-port <port>]`
  - `screen ltr recover [--json] [--ui-port <port>]`
  - `screen ltr reset [--json] [--ui-port <port>]`
- 格式化输出函数 `print_screen_ltr_report`；
- `screen status` 扩充显示 LTR 状态与指标。

### 6. 端到端集成测试（`tests/phase5.rs`）

- 编写 `m5_37_screen_ltr_long_term_reference_pipeline`，验证：
  1. 初始 LTR 禁用状态；
  2. 启用 LTR 并验证初始关键帧建立首个黄金参考帧锚点；
  3. 推流多个中间差分帧；
  4. 触发 LTR 自愈恢复，验证 `SCREEN_COMPRESS_LTR` 增量补丁派发与 1-RTT 瞬时恢复；
  5. 校验 `saved_burst_bytes > 0` 且无 IDR 关键帧突发；
  6. 重置并验证双端同步；
  7. 安全关闭会话。

---

## 效果与影响

1. **瞬时 0 脉冲丢包恢复**：消除中间差分丢包时的全量 IDR 关键帧突发脉冲，避免网络缓冲膨胀与端侧冻结；
2. **极低开销与高鲁棒性**：仅在双端维护单个黄金参考画布，内存增量微乎其微（1080p RGBA 仅 ~8MB），在弱网环境下提供极高容错性；
3. **100% 双端全同构**：CLI、REST API、信令与遥测完全对称，无任何私有旁路。
