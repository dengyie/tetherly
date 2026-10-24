# ADR-083 屏幕流动态帧率与自适应定步步频控制器（Screen Streaming Dynamic FPS & Adaptive Frame Pacing Controller）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v1.97）**——动态 FPS 分级阶梯自适应（基于测速简档 15/24/30/60 FPS 自动阶梯）、微秒级前置定步控制（Pre-Grab Pacing Check 避免冗余捕获开销）、编码器帧率热重建联动、回环 API 与 CLI 动态调优接口 |
| 日期 | 2026-10-25 |
| 关联 | §9.7「压缩」、ADR-078「链路感知画质自动分级」、ADR-079「屏幕有损编码器与线上协商」、ADR-080「双向帧确认与自适应拥塞背压」、ADR-081「按需关键帧请求与序列断洞自愈恢复」、ADR-082「屏幕流动态速率自适应与 AIMD 码率控制器」 |

## 动机

1. **盲目高频捕获带来算力与功耗空转**：此前屏幕推流发送泵（Pump）以固定的 16ms 定时器驱动捕获（`src.grab()`）。在弱网或低帧率配置下，操作系统级桌面图像截取（Windows DXGI 显存映射、Android adb 管道抓流、Linux X11 SHM 拷贝）依然全速运行，随后才在网络排队或拥塞检测中被丢弃，造成了严重的 CPU/GPU 算力和电池电量浪费。
2. **缺乏链路感知的动态帧率阶梯分级**：尽管 ADR-078 实现了基于实测带宽的码率初值分级，但推流帧率却始终固定为默认值。在跨国高延迟或低带宽（< 5 Mbps）环境下，继续强推 30 FPS 会导致关键帧和在途分片挤压队列，而高带宽（$\ge$ 40 Mbps）局域网内又无法自发提升至 60 FPS 流畅视感。
3. **缺乏微秒级精准帧定步机制（Frame Pacing）**：由于系统定时器与调度抖动，若无统一的微秒级定步控制器（`ScreenFpsPacer`），帧发送间隔容易呈现密集突发（Burst）与间隙交替，破坏下层传输协议的平滑性并引发不必要的网络抖动与拥塞误判。
4. **缺乏运行时 FPS 热调优通道**：运行中的桌面共享会话缺乏轻量级干预接口，运维人员或桌面用户无法根据当前业务需求（如静止 PPT 演示切换到高帧率动态演示）热调整帧率上限与定步节奏。

## 决定

1. **核心自适应定步状态机（`ScreenFpsPacer`）**：
   - 在 `tetherly-core::screen` 内部引入确定性无锁的微秒级帧定步控制器。
   - **刚性上下界限制**：支持的目标帧率严格限制在 $[5\text{ FPS}, 120\text{ FPS}]$ 之间（`SCREEN_FPS_MIN = 5`, `SCREEN_FPS_MAX = 120`），默认帧率为 30 FPS（`SCREEN_FPS_DEFAULT = 30`）。
   - **定步时间窗计算**：定步时间窗严格根据目标帧率微秒精确计算：
     $$\text{interval\_us} = \left\lfloor \frac{1\,000\,000}{\text{fps}} \right\rfloor$$
     例如：60 FPS 对应 $16\,666\,\mu\text{s}$，30 FPS 对应 $33\,333\,\mu\text{s}$，5 FPS 对应 $200\,000\,\mu\text{s}$。
   - **前置定步判定（Pre-Grab Pacing Check）**：在调用昂贵捕获操作（`src.grab()`）之前，发送泵先执行定步就绪判定：
     $$\Delta t = \text{now\_us} - \text{last\_frame\_us} \ge \text{interval\_us}$$
     若未达到定步周期，立即跳过该 Tick 并返回 `Ok(false)`，彻底杜绝无谓的显存与管道抓取开销。
   - **定步槽位消费（Pacing Consumption）**：当帧被成功抓取并入队（或因网络严重拥塞主动丢弃）后，记录本次推流时间戳 $\text{last\_frame\_us} = \text{now\_us}$，进入下一周期的定步等待。

2. **实测带宽简档自动分级阶梯（Bandwidth Profile Auto-Tiering）**：
   - 实现 `screen_fps_from_profile(measured_mbps: Option<f64>) -> u32`：
     - **Minimal (< 5 Mbps)**: 15 FPS（低频稳推，保障跨国极弱网下文字可辨且不堆叠延迟）
     - **Low (5–15 Mbps)**: 24 FPS（影视级基础流畅度）
     - **Medium (15–40 Mbps)**: 30 FPS（标准交互桌面体验）
     - **High ($\ge$ 40 Mbps)**: 60 FPS（电竞/高刷极速流畅视感）
   - 在推流会话建立（`screen_send_frame` 首次激活）时，若配置了 `screen_fps_auto: true`，根据对端节点的测速结果（`peer_link_quality` / `measured_mbps`）自动选定目标 FPS 阶梯。

3. **跨层端口与编码器帧率热重建联动**：
   - `ScreenCodec` 与 `ScreenSource` 抽象端口扩充能力方法：`fn set_fps(&self, fps: u32)` 和 `fn fps(&self) -> u32`。
   - `H264Codec` 引入原子帧率管理并在 `encode_with` 中检测目标帧率变更。若目标帧率发生变化（`built_fps != target_fps`），触发底层 `openh264` 编码器平滑重建，配置最新最大帧率 `openh264::encoder::FrameRate::from_hz(fps as f32)` 并强制触发首帧 IDR 关键帧，确保时间轴与宏块预测完全同步。

4. **回环 API 与 CLI 统一管理**：
   - 回环 HTTP API 路由扩充：
     - `GET /api/screen/fps`：查询当前生效的目标 FPS、上下限（5–120）、定步周期微秒数、自动分级标识及时间戳，返回 `ScreenFpsReport`。
     - `POST /api/screen/fps`：热更新目标帧率（接收 JSON `{"fps": number}` 或 Query 参数 `?fps=...`），返回强类型 `ScreenFpsReport`。
   - CLI 命令行同构化：
     - `tetherly screen fps`：以人类可读格式展示当前生效的推流帧率、定步周期及调节范围。
     - `tetherly screen fps <fps>`：即时热调整目标推流帧率。
     - `tetherly run --screen-fps <auto|N>`：支持节点启动时预设帧率策略。
     - `tetherly screen status`：实时状态透出当前推流目标 `fps_target`。
   - 遥测 DTO 兼容：`ScreenStatusReport` 新增 `fps_target` 字段。

## 落地记录（v1.97）

- `crates/tetherly-core/src/screen.rs`:
  - 帧率常数定义：`SCREEN_FPS_DEFAULT (30)`, `SCREEN_FPS_MIN (5)`, `SCREEN_FPS_MAX (120)`。
  - 核心状态机 `ScreenFpsPacer`：实现微秒定步周期计算、前置条件判定、时间消费与上下限钳位。
  - `ScreenStats` 扩充 `fps_target: u32` 遥测指标。
  - `ScreenSender` 深度集成 `ScreenFpsPacer`，提供 `set_fps`、`fps_pacer`、`is_pacing_ready`、`consume_pacing`。
  - 完备单测 `screen::tests::fps_pacer_and_frame_rate_throttling`。
- `crates/tetherly-core/src/ports.rs` & `lib.rs`:
  - `ScreenSource` 与 `ScreenCodec` 扩展 `set_fps` 与 `fps` 端口方法。
  - 导出常数及 `ScreenFpsPacer`。
- `crates/tetherly-node/src/screen.rs`:
  - `H264Codec`、`NegotiatedCodec`、`EncodedSource` 实现 `set_fps` 与 `fps` 并与底层 OpenH264 编码器重构及 IDR 强发联动。
  - 实现链路带宽分级阶梯算法 `screen_fps_from_profile`。
  - 完备单测 `fps_tier_boundaries` 与 `fps_control_and_forwarding`。
- `crates/tetherly-node/src/runtime.rs`:
  - `NodeConfig` 新增 `screen_fps: u32` 与 `screen_fps_auto: bool`。
  - 发送泵 `screen_send_frame()` 在抓帧前执行微秒前置定步判定，初次推流依据实测带宽自动分级，入队后消费定步时间槽。
  - 新增 `ScreenFpsReport` DTO，并在 `ScreenStatusReport` 中透出实时 `fps_target`。
  - `Node` 增加 `screen_set_fps`、`screen_fps`、`screen_fps_report`、`screen_set_fps_reported`、`screen_is_pacing_ready`。
  - 完备单测 `test_screen_fps_pacing_and_reporting`。
- `crates/tetherly-node/src/uihttp.rs` & `bin/tetherly.rs`:
  - 路由 `GET /api/screen/fps` 与 `POST /api/screen/fps`。
  - `Cmd::Run` 支持 `--screen-fps <auto|N>` 参数。
  - 子命令 `tetherly screen fps` 及 `status` 实时展示。
  - 守护进程推流泵循环集成 `screen_is_pacing_ready` 前置短路判定。
- `tests/phase5.rs`:
  - E2E 集成测试 `m5_7_screen_dynamic_fps_pacing_and_telemetry`，验证微秒定步阻断、运行时 60 FPS / 5 FPS 热调优、上下限边界钳位与 H.264 解码呈现。
