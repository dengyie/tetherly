# ADR-111 屏幕串流实时客观 QoE 自主闭环调优、画质感知驱动的自适应流控协调流水线与双端 CLI 全同构 (Screen Streaming Autonomous Closed-Loop QoE Adaptation, Perceptual-Driven Stream Coordination Pipeline with 100% Dual CLI Isomorphism)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-29，v2.25 / v0.1.47）**——建立屏幕串流实时客观 QoE 自主闭环调优、画质感知驱动的自适应流控协调流水线：(1) **客观 QoE 自主闭环模型与状态机**（`crates/tetherly-core/src/screen.rs`）：定义 `ScreenQoeMode`（Off/0, Conservative/1, Balanced/2, Economy/3）、`ScreenQoeAction`（Hold/0, BoostBitrate/1, UpgradeScale/2, LowerQp/3, ForceKeyframe/4, FallbackThrottle/5）、`ScreenQoeConfig` 与 `ScreenQoeReport`，实现 `ScreenQoeController`；(2) **画质感知驱动闭环调优决策**：接收 ADR-110 抽样评估指标（PSNR、SSIM、MSE 与 `quality_warning`），对比模式目标阈值（Conservative: 34dB/90%, Balanced: 30dB/85%, Economy: 26dB/78%），结合 500ms 冷却窗与迟滞机制自主输出流控恢复动作，消除高压缩降采样下的画质盲区；(3) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 25（`ControlMsg::SetScreenQoeMode { mode: u8, target_psnr_db: u8, target_ssim_pct: u8 }`）与 Tag 26（`ControlMsg::ResetScreenQoeMode`）；(4) **节点运行时流控与网络循环挂接**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_qoe_config` 与 `screen_qoe_controller`，在质检采样更新后驱动闭环调优，联动四维协同调度器与关键帧请求，网络事件循环双向同步 Tag 25/26；(5) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/qoe`（GET / POST）、`/api/screen/qoe/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充 QoE 自主闭环遥测指标，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen qoe [status|set|reset]` 子命令族与格式化报表输出，`screen status` 扩充 QoE 闭环遥测；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_35_screen_autonomous_qoe_closed_loop_adaptation_pipeline` 全链路验证。 |
| 日期 | 2026-10-29 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-084「屏幕串流动态分辨率降采样控制器」、ADR-085「屏幕内容自适应画质模式与 QP 控制器」、ADR-086「屏幕串流4维流控矩阵协同调度器」、ADR-100「屏幕串流瓶颈驱动自主调优闭环」、ADR-110「屏幕串流实时客观画质度量与 PSNR/SSIM 质检流水线」 |

---

## 动机

1. **从被动画质观测到主动感知闭环**：
   - 在 ADR-110 中，系统已成功引入 MSE、PSNR 与 16x16 宏块 SSIM 客观画质度量管道，并通过抽样探针输出 `quality_warning` 告警；
   - 然而，该告警此前仅作为被动观测遥测展示，缺乏将画质劣化信号回传至流控执行器的有效闭环通路；
   - 当网络轻载但前期降采样（ADR-084）或色彩量化（ADR-108）过度激进时，底层网络指标（RTT、丢包率）表现良好，但用户端呈现的画面文字严重模糊、宏块伪影显著，系统必须具备**由画质客观失真驱动的主动自愈调优回路**。

2. **多阶策略模式与感知阈值划分**：
   - 针对不同应用场景（精细代码阅读、办公文档协作、弱网极端保活），设计多阶 QoE 保障策略：
     - `Conservative`（保真优先）：目标 $PSNR \ge 34\text{ dB}, SSIM \ge 0.90$，轻微失真即刻触发恢复；
     - `Balanced`（平衡模式，默认）：目标 $PSNR \ge 30\text{ dB}, SSIM \ge 0.85$，在清晰度与带宽之间取得良好平衡；
     - `Economy`（经济省流）：目标 $PSNR \ge 26\text{ dB}, SSIM \ge 0.78$，允许低码率高压缩，避免无谓带宽溢出；
     - `Off`：关闭闭环干预，仅保留度量遥测。

3. **分级自适应调优动作与抗震荡防护**：
   - 当检测到画质低于目标阈值时，QoE 控制器按层级输出针对性动作：
     - `BoostBitrate`：量化误差高（MSE 大）时，阶梯式提升底线码率；
     - `UpgradeScale`：结构边缘模糊（SSIM 低）时，提升分辨率缩放比率；
     - `LowerQp`：降低压缩量化参数以提升空间细节；
     - `ForceKeyframe`：严重画面漂移时触发 `ControlMsg::KeyframeRequest`（Tag 3）强制 I 帧刷新；
     - `FallbackThrottle`：极端双重劣质时平滑定步。
   - 设定动作冷却时间（500ms），避免逐帧微小波动引起流控频繁震荡。

4. **全链路协同与双端 CLI 100% 同构**：
   - 遵循 Tetherly 架构铁律，通过定长信令（Tag 25/26）、回环 REST API（`/api/screen/qoe`）以及双端 CLI（`screen qoe`）实现完整能力对称与可观测性。

---

## 决定

### 1. 客观 QoE 数据模型与核心算法（`crates/tetherly-core/src/screen.rs`）

- **数据模型**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "lowercase")]
  pub enum ScreenQoeMode {
      #[default]
      Off = 0,
      Conservative = 1,
      Balanced = 2,
      Economy = 3,
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "snake_case")]
  pub enum ScreenQoeAction {
      #[default]
      Hold = 0,
      BoostBitrate = 1,
      UpgradeScale = 2,
      LowerQp = 3,
      ForceKeyframe = 4,
      FallbackThrottle = 5,
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenQoeConfig {
      pub mode: ScreenQoeMode,
      pub target_psnr_db: u8,
      pub target_ssim_pct: u8,
  }

  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenQoeReport {
      pub ok: bool,
      pub mode: String,
      pub target_psnr_db: u8,
      pub target_ssim_pct: u8,
      pub actions_total: u64,
      pub last_action: String,
      pub last_action_reason: String,
      pub in_cooldown: bool,
      pub active_boosts: u32,
  }
  ```

- **核心决策控制器 (`ScreenQoeController`)**：
  - 维护最后动作时间戳 `last_action_us`、冷却窗口（500,000 微秒）与累计动作计数；
  - 接收最新 `ScreenQualityMetricReport`，在非冷却期内对比 `last_psnr_db` 与 `last_ssim`：
    - 若 $PSNR < \text{target\_psnr}$ 或 $SSIM < \text{target\_ssim}$，按失真特征优先输出 `UpgradeScale`、`LowerQp`、`BoostBitrate` 或 `ForceKeyframe`；
    - 若画质达到标准，输出 `Hold`。

### 2. 14 字节定长信封线协议扩充（`TYPE_SCREEN_CONTROL = 0x0502`）

- **Tag 25 (`ControlMsg::SetScreenQoeMode { mode: u8, target_psnr_db: u8, target_ssim_pct: u8 }`)**：
  - 64 位紧凑载荷：byte 0 存 `mode`，byte 1 存 `target_psnr_db`，byte 2 存 `target_ssim_pct`，其余高位清零；
- **Tag 26 (`ControlMsg::ResetScreenQoeMode`)**：
  - 64 位载荷置 0，重置 QoE 模式至默认关闭与初始阈值。

### 3. 节点运行时管线与闭环决策挂接（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载 `screen_qoe_config: Arc<RwLock<ScreenQoeConfig>>` 与 `screen_qoe_controller: Arc<RwLock<ScreenQoeController>>`；
- 暴露 `screen_qoe_report`、`screen_set_qoe_mode`、`screen_reset_qoe_mode` 方法；
- 在接收呈现/抓帧管线中与抽样质检探针联动求值，并在事件循环中双向同步 Tag 25/26。

### 4. 回环 REST 控制面端点闭环（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/qoe`：查询当前 QoE 自主闭环配置与遥测报告；
- `POST /api/screen/qoe`：动态设置模式与目标 PSNR/SSIM 阈值；
- `POST/DELETE /api/screen/qoe/reset`：重置 QoE 配置并清空统计；
- `ScreenStatusReport` 扩充 `qoe_enabled`、`qoe_mode`、`qoe_target_psnr_db`、`qoe_target_ssim_pct`、`qoe_actions_total`、`qoe_last_action`；
- 继承 `127.0.0.1` 物理绑定与 `Host` 头 403 强校验。

### 5. 双端 CLI 100% 全同构

- `bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly` 新增：
  - `screen qoe status [--json] [--ui-port <port>]`
  - `screen qoe set --mode <conservative|balanced|economy|off> [--target-psnr <db>] [--target-ssim <pct>] [--json] [--ui-port <port>]`
  - `screen qoe reset [--json] [--ui-port <port>]`
- 格式化输出函数 `print_screen_qoe_report`；
- `screen status` 扩充显示 QoE 闭环遥测。

---

## 验证与测试

1. **核心单元测试**：`screen_autonomous_qoe_closed_loop_adaptation_algorithms_and_signaling` 覆盖各策略阈值触发、冷却窗防抖、动作序列输出与 Tag 25/26 往返编解码；
2. **回环 REST 端点单测**：`test_uihttp_screen_qoe_endpoints` 覆盖 API 全生命周期、配置校验与 Host 403 阻断；
3. **端到端集成测试**：`tests/phase5.rs::m5_35_screen_autonomous_qoe_closed_loop_adaptation_pipeline` 验证双节点全链路协商、画质劣化驱动动作触发、无损画质恢复与复位信令同步。
