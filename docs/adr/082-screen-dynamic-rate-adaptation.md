# ADR-082 屏幕流动态速率自适应与 AIMD 码率控制器（Screen Streaming Dynamic Rate Adaptation & AIMD Bitrate Controller）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v1.96）**——AIMD 速率自适应（拥塞退避乘法减 20% + 稳定通道加法增 150 kbps）、400ms 冷却抑震荡、编码器热重配置联动、回环 API 与 CLI 动态调优接口 |
| 日期 | 2026-10-25 |
| 关联 | §9.7「压缩」、ADR-078「链路感知画质自动分级」、ADR-079「屏幕有损编码器与线上协商」、ADR-080「双向帧确认与自适应拥塞背压」、ADR-081「按需关键帧请求与序列断洞自愈恢复」 |

## 动机

1. **静态码率与波动网络失配**：此前屏幕推流的码率仅在会话建立时依据初始测速简档（`screen_bitrate_auto`）设定为静态上限（默认 4 Mbps）。然而跨国弱网、蜂窝移动与公共 Wi-Fi 网络具有显著的时变抖动特征。固定码率在带宽骤降时会持续溢出信道承载能力，而在信道拓宽时又无法发挥高分辨率的画质潜力。
2. **缺乏速率闭环反馈引发恶性重传**：ADR-080 引入了拥塞丢帧，ADR-081 引入了关键帧恢复。但在没有码率自适应时，若网络持续劣化，高码率依然在持续生成大量碎片帧，导致发送端频繁触发丢帧背压和 IDR 重置，加剧链路排队并恶化 RTT。
3. **缺乏运行时热调优通道**：运行中的桌面共享会话缺乏轻量级干预接口，运维人员或上层智能编排无法在不中断看屏的前提下动态调整码率上限与画质倾向。

## 决定

1. **核心 AIMD 码率控制器（`ScreenRateController`）**：
   - 在 `tetherly-core::screen` 内部引入无锁/确定性的 AIMD 速率自适应控制器。
   - **乘法减（Multiplicative Decrease, MD）**：当检测到网络拥塞信号（在途帧数 $\ge 4$、单帧往返时延 $\text{RTT} \ge 250\text{ms}$、或发送端套接字/队列丢帧）时，立即将当前码率缩减至 80%（即 $-20\%$）：
     $$\text{bitrate}_{\text{new}} = \max\left(\lfloor\text{bitrate} \times 0.80\rfloor, \text{SCREEN\_BITRATE\_MIN\_BPS}\right)$$
   - **冷却抑震荡保护**：设置 400ms 冷却时间（`SCREEN_BITRATE_COOLDOWN_US = 400_000`）。在发生一次下调后 400ms 内，即使仍有高 RTT 确认帧到达，也不连续重复下调，防止单次突发抖动导致码率归底。
   - **加法增（Additive Increase, AI）**：当链路处于绝对健康状态（零在途排队 $\text{in\_flight} = 0$ 且 $\text{RTT} < 120\text{ms}$）时，计数器递增；每连续收获 5 帧健康确认，尝试向上探测可用带宽 $+150\text{ kbps}$（`SCREEN_BITRATE_STEP_INCREASE_BPS = 150_000`）：
     $$\text{bitrate}_{\text{new}} = \min\left(\text{bitrate} + 150\,000, \text{SCREEN\_BITRATE\_MAX\_BPS}\right)$$
   - **刚性上下界限制**：码率严格限制在 $[200\text{ kbps}, 20\text{ Mbps}]$ 之间（`SCREEN_BITRATE_MIN_BPS = 200_000`, `SCREEN_BITRATE_MAX_BPS = 20_000_000`）。

2. **跨层端口与运行时编码器热更新联动**：
   - `ScreenCodec` 与 `ScreenSource` 抽象端口扩展能力方法：`fn set_bitrate_bps(&self, bps: u32)` 和 `fn bitrate_bps(&self) -> u32`。
   - `EncodedSource<S>` 自动将上层下发的 `set_bitrate_bps` 委托给绑定的底层编码器。
   - `H264Codec` 与 `NegotiatedCodec` 将目标码率以原子值缓存（`AtomicU32`），并在下次 `encode_with` 帧捕获时平滑应用，重新下发 SPS/PPS 并生成首个适配码率的关键帧，防止历史宏块参考错位。
   - `tetherly-node` 发送泵：在帧确认（`on_control_with_time` 处理 `ControlMsg::Ack`）与主动丢帧（`record_drop_with_time`）时，立即同步控制器的最新码率至活跃编码器实例。

3. **回环 API 与 CLI 统一管理**：
   - 回环 HTTP API 路由扩充：
     - `GET /api/screen/bitrate`：查询当前推流码率、上下限及时间戳。
     - `POST /api/screen/bitrate`：热更新目标码率（接收 `kbps` 或 `bps` 参数），返回强类型 `ScreenBitrateReport`。
   - CLI 命令行同构化：
     - `tetherly screen bitrate`：以人类可读格式展示当前码率及可用调节范围。
     - `tetherly screen bitrate <kbps>`：即时下发目标码率。
     - `tetherly screen status`：实时状态中展示当前生效的推流码率。
   - 遥测 DTO 兼容：`ScreenStatusReport` 新增 `bitrate_bps` 字段。

## 落地记录（v1.96）

- `crates/tetherly-core/src/screen.rs`:
  - 码率常数定义：`SCREEN_BITRATE_MIN_BPS (200k)`, `SCREEN_BITRATE_MAX_BPS (20M)`, `SCREEN_BITRATE_DEFAULT_BPS (4M)`, `SCREEN_BITRATE_STEP_INCREASE_BPS (150k)`, `SCREEN_BITRATE_COOLDOWN_US (400ms)`。
  - `ScreenStats` 扩充 `bitrate_bps: u32` 遥测指标。
  - 核心状态机 `ScreenRateController`：实现 AIMD、冷却保护、连续健康帧判定与边界钳位。
  - `ScreenSender` 深度集成 `ScreenRateController`，在 `on_ack`、`record_drop` 及手动 `set_bitrate_bps` 时驱动自适应。
  - 完备单测 `screen::tests::rate_controller_aimd_adaptation`。
- `crates/tetherly-core/src/ports.rs` & `lib.rs`:
  - `ScreenSource` 与 `ScreenCodec` 扩展 `set_bitrate_bps` 与 `bitrate_bps` 端口方法。
  - 导出常数及 `ScreenRateController`。
- `crates/tetherly-node/src/screen.rs`:
  - `H264Codec` 与 `NegotiatedCodec`、`EncodedSource` 实现码率更新并与底座 `openh264` 编码器联动。
- `crates/tetherly-node/src/runtime.rs`:
  - 发送泵与信令接收主循环实时连接 `ScreenRateController` 与活跃编码器。
  - 新增 `ScreenBitrateReport` DTO，并在 `ScreenStatusReport` 中透出实时码率。
  - `Node` 增加 `screen_set_bitrate_bps`、`screen_bitrate_bps`、`screen_bitrate_report`、`screen_set_bitrate_reported`。
- `crates/tetherly-node/src/uihttp.rs` & `bin/tetherly.rs`:
  - 路由 `GET /api/screen/bitrate` 与 `POST /api/screen/bitrate`。
  - 子命令 `tetherly screen bitrate` 及 `status` 实时展示。
- `tests/phase5.rs`:
  - E2E 集成测试 `m5_6_screen_dynamic_rate_adaptation`，验证真实信道握手、API 动态调优、边界钳位与 H.264 解码呈现。
