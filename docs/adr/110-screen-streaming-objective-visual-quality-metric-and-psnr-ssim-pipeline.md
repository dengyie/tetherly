# ADR-110 屏幕串流实时客观画质度量、PSNR/SSIM 结构失真评估与自适应质检闭环流水线（Screen Streaming Real-Time Objective Visual Quality Metric, PSNR/SSIM Distortion Profiling & Closed-Loop Quality Assurance Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-29，v2.24 / v0.1.46）**——建立屏幕串流实时客观画质度量、PSNR/SSIM 结构失真评估与自适应质检闭环流水线：(1) **客观画质度量模型与核心算法**（`crates/tetherly-core/src/screen.rs`）：严格遵循 ADR-006 零重型外部依赖铁律，设计 `ScreenQualityMetricConfig`、`ScreenQualityMetricReport`；纯标准库数学运算实现均方误差 `ScreenFrame::compute_mse`、峰值信噪比 `ScreenFrame::compute_psnr`（基于对数逼近 $10 \cdot \log_{10}(255^2 / MSE)$，完全无损设为 100.0 dB 基准）与 16x16 宏块结构相似性 `ScreenFrame::compute_ssim`（计算宏块窗口亮度均值、方差与协方差，全帧取均值 $0.00 \sim 1.00$）；(2) **抽样评估与低开销质检探针**：可配置 `sample_interval`（默认每 10 帧抽验一次，范围 1..=120），维持评估帧计数、滚动累计平均与最新单帧指标；当 $PSNR < 30.0$ dB 或 $SSIM < 0.85$ 时触发 `quality_warning: true` 告警；(3) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 23（`ControlMsg::SetScreenQualityMetricMode { enabled: bool, sample_interval: u8 }`）与 Tag 24（`ControlMsg::ResetScreenQualityMetricMode`）；(4) **节点运行时流控与网络循环挂接**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_quality_metric_config` 与 `screen_quality_metric_state`，在串流接收呈现与参考帧流水线中按周期执行画质度量并更新遥测指标，双端网络事件循环实时同步 Tag 23/24；(5) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/metric`（GET / POST）、`/api/screen/metric/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充客观画质度量遥测指标，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen metric [status|set|reset]` 子命令族与格式化报表输出，`screen status` 扩充客观画质遥测；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_34_screen_objective_visual_quality_metric_and_psnr_ssim_pipeline` 全链路验证。 |
| 日期 | 2026-10-29 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-085「屏幕内容自适应画质模式与 QP 控制器」、ADR-086「屏幕串流4维流控矩阵协同调度器」、ADR-100「屏幕串流瓶颈驱动自主调优闭环」、ADR-108「屏幕串流自适应色彩深度与灰度经济模式流水线」、ADR-109「屏幕串流动态差分补丁与时域压缩流水线」 |

---

## 动机

1. **端到端画质盲区与自适应闭环缺口**：
   - 此前 Tetherly 屏幕串流已陆续引入自适应码率调节（ADR-082）、自适应帧率（ADR-083）、动态降采样（ADR-084）、自适应 QP（ADR-085）、色彩量化（ADR-108）与动态差分补丁（ADR-109）；
   - 然而，所有流控决策完全基于传输网络层指标（如 RTT、丢包率、TWCC 投递速率与在途字节），对画面“实际呈现质量”缺乏客观定量反馈；
   - 在高压缩或复杂网络场景下，容易出现“信道未丢包但画面文字发糊、宏块失真严重”或“码率过高造成无谓算力与带宽浪费”的双向失控。亟需引入端到端实时客观画质度量（QoE / Distortion Profiling）。
2. **纯标准库零外部重型图形依赖（ADR-006）**：
   - 传统图形学质量评估多依赖 OpenCV、ImageMagick 等数百兆 C/C++ 依赖，与 Tetherly 自包含工业级分发目标冲突；
   - 必须基于纯 Rust 标准库浮点运算，实现高吞吐、零动态内存分配的均方误差（MSE）、峰值信噪比（PSNR）与宏块结构相似性（SSIM）算法。
3. **轻量级抽样与无侵入监控**：
   - 全分辨率逐帧评估可能对 CPU 造成多余负担，因此设计可配置的抽样评估周期（`sample_interval`，默认 10 帧），在兼顾毫秒级极速响应的同时提供精准的滚动统计平均；
   - 设立质量下限阈值（$PSNR < 30.0$ dB 或 $SSIM < 0.85$ 时触发 `quality_warning`），可无缝反馈给四维矩阵协同调度器（ADR-086）与瓶颈调优闭环（ADR-100）。
4. **全链路协同与双端 CLI 100% 同构**：
   - 线协议定长信令（Tag 23 / Tag 24）、回环 REST 控制面（`/api/screen/metric`）以及双端 CLI（`screen metric`）具备完全对等的能力与格式化展示。

---

## 决定

### 1. 客观画质数据模型与核心算法（`crates/tetherly-core/src/screen.rs`）

- **数据模型**：
  ```rust
  pub const SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL: u8 = 10;

  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenQualityMetricConfig {
      pub enabled: bool,
      pub sample_interval: u8, // 1..=120, default 10
  }

  impl Default for ScreenQualityMetricConfig {
      fn default() -> Self {
          Self {
              enabled: false,
              sample_interval: SCREEN_METRIC_DEFAULT_SAMPLE_INTERVAL,
          }
      }
  }

  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenQualityMetricReport {
      pub ok: bool,
      pub enabled: bool,
      pub sample_interval: u8,
      pub evaluated_frames: u64,
      pub avg_mse: f64,
      pub avg_psnr_db: f64,
      pub avg_ssim: f64,
      pub last_psnr_db: f64,
      pub last_ssim: f64,
      pub quality_warning: bool,
  }
  ```
- **核心算法实现**：
  - `ScreenFrame::compute_mse(&self, reference: &ScreenFrame) -> Result<f64, CoreError>`：
    校验尺寸与格式匹配，计算 RGB 三通道像素差方均值；
  - `ScreenFrame::compute_psnr(&self, reference: &ScreenFrame) -> Result<f64, CoreError>`：
    若 $MSE \le 10^{-10}$，直接返回 $100.0$ dB；否则计算 $10.0 \cdot \log_{10}(255.0^2 / MSE)$；
  - `ScreenFrame::compute_ssim(&self, reference: &ScreenFrame) -> Result<f64, CoreError>`：
    以 16x16 宏块为窗口，提取亮度分量 $Y = (77R + 150G + 29B) >> 8$，计算局部均值、方差与协方差，得出宏块得分并求取全屏平均值。

### 2. 14 字节定长信封线协议扩充（`TYPE_SCREEN_CONTROL = 0x0502`）

- **Tag 23 (`ControlMsg::SetScreenQualityMetricMode { enabled: bool, sample_interval: u8 }`)**：
  - 64 位紧凑载荷：byte 0 存 `enabled as u8`，byte 1 存 `sample_interval`，其余高位清零；
- **Tag 24 (`ControlMsg::ResetScreenQualityMetricMode`)**：
  - 64 位载荷置 0，恢复至默认禁用质检。

### 3. 节点运行时管线与生命周期挂接（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载 `screen_quality_metric_config` 与 `screen_quality_metric_state`；
- 在接收呈现/抓帧管线中依据 `sample_interval` 抽样评估，更新统计指标；
- 网络事件循环在双端实时同步 Tag 23/24。

### 4. 回环 REST 控制面端点闭环（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/metric`：查询当前画质度量配置与遥测报告；
- `POST /api/screen/metric`：配置质检开关与抽样周期；
- `POST/DELETE /api/screen/metric/reset`：复位至默认并清零指标；
- 继承 `127.0.0.1` 物理绑定与 `Host` 头 403 强校验。

### 5. 双端 CLI 100% 全同构

- `bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly` 新增：
  - `screen metric status [--json] [--ui-port <port>]`
  - `screen metric set --enabled <true|false> [--interval <num>] [--json] [--ui-port <port>]`
  - `screen metric reset [--json] [--ui-port <port>]`
- 格式化输出函数 `print_screen_quality_metric_report`；
- `screen status` 扩充画质指标输出。

---

## 验证

1. **核心单测**：`crates/tetherly-core/src/screen.rs` 中 `screen_objective_visual_quality_metric_and_psnr_ssim_algorithms`；
2. **REST 单测**：`crates/tetherly-node/src/uihttp.rs` 中 `test_uihttp_screen_quality_metric_endpoints`；
3. **集成测试**：`tests/phase5.rs` 中 `m5_34_screen_objective_visual_quality_metric_and_psnr_ssim_pipeline`。
