# ADR-085 屏幕流自适应内容模式与量化参数（QP）质量控制器（Screen Streaming Adaptive Content Profile & Quantization Parameter Quality Controller）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v1.99）**——实现内容语义自适应画质简档（ScreenContentProfile: Text / Motion / Auto）、量化参数（Quantization Parameter, QP 0–51）上下界控制器（ScreenQualityController）、底层 OpenH264 编码模式联动（ScreenContentRealTime vs CameraVideoRealTime）、参数热重构与首帧 IDR 恢复、完成 4 维多媒体流控矩阵（码率、帧率、分辨率、语义画质/QP）、回环 API 与 CLI 动态调优接口 |
| 日期 | 2026-10-25 |
| 关联 | §9.7「压缩」、ADR-078「链路感知画质自动分级」、ADR-079「屏幕有损编码器与线上协商」、ADR-080「双向帧确认与自适应拥塞背压」、ADR-081「按需关键帧请求与序列断洞自愈恢复」、ADR-082「屏幕流动态速率自适应与 AIMD 码率控制器」、ADR-083「屏幕流动态帧率与自适应定步步频控制器」、ADR-084「屏幕流动态分辨率自适应降采样控制器」 |

## 动机

1. **桌面文本与动态画面的语义分歧**：
   - 传统视频流控通常只考虑带宽（Bitrate）、帧率（FPS）和分辨率（Resolution）三维参数。然而在桌面远程交互中，内容语义具有极端的二元分化：
     - **文本与代码桌面（Text/IDE Profile）**：高频边缘、细小字形（如终端与编辑器中的小号等宽字体）、大面积纯色或高反差背景。需要杜绝振铃效应（ringing artifacts）与字符模糊，允许在画面静态时以较低熵率输出，但在动态键入时必须保持极高锐度。
     - **动态画面与视频媒体（Motion Profile）**：包含视频播放、3D 渲染或快速滚动。宏块运动熵极高，需要平滑吸收瞬时码率突发，优先保证帧率连贯性与低传输延迟，容忍适度的高频细节平滑。
2. **OpenH264 编码器特性的语义映射**：
   - OpenH264 原生支持不同的使用场景模式（Usage Type）与码率控制模式（Rate Control Mode）：
     - `UsageType::ScreenContentRealTime` 针对桌面屏幕内容优化，具备调色板与字符边界保持机制。
     - `UsageType::CameraVideoRealTime` 针对连续摄影与自然运动优化，具备更强的运动矢量预测与时间域宏块抗抖动能力。
   - 现有的固定编码器配置无法满足多场景的自适应切换需求。
3. **量化参数（QP）边界刚性控制与保底防噪**：
   - H.264 标称 QP 取值范围为 $[0, 51]$，数值越低画面越逼真、但码率激增；数值越高画面压缩越狠、但易发生宏块马赛克。
   - 在缺乏 QP 边界钳制时，码率控制器在突发丢包或高运动时可能将 QP 拉至上限（如 48–51），导致文本完全糊成色块无法阅读；或在低码率下无法锁定下界，导致编码器在简单字符闪烁时产生不必要的过载。
   - 必须在协议与核心库中建立确定性的 QP 上下界控制器，提供自适应推荐默认值并允许精准覆盖。
4. **4 维多媒体流控矩阵闭环**：
   - 随着码率控制器（ADR-082）、帧率控制器（ADR-083）、分辨率控制器（ADR-084）相继落地，量化与语义画质控制器（ADR-085）构成了完整的 4 维流控控制闭环：
     $$\text{QoS} = f(\text{Bitrate}, \text{Framerate}, \text{Resolution}, \text{Semantic Profile / QP})$$
   - 赋予桌面推流在极弱网、代码编辑、高清演播等多场景下的全自动自适应与精细化人工干预能力。

## 决定

1. **核心数据结构与常数定义（`tetherly-core::screen`）**：
   - **`ScreenContentProfile` 内容画质简档**：
     ```rust
     #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
     #[serde(rename_all = "lowercase")]
     pub enum ScreenContentProfile {
         Text = 0,
         Motion = 1,
         Auto = 2,
     }
     ```
   - **QP 界限常数**：
     - 全局范围：`SCREEN_QP_MIN = 0`, `SCREEN_QP_MAX = 51`
     - 文本模式默认界限：`SCREEN_QP_TEXT_MIN = 12`, `SCREEN_QP_TEXT_MAX = 35`（杜绝高频振铃，保护字符可读性）
     - 动态模式默认界限：`SCREEN_QP_MOTION_MIN = 16`, `SCREEN_QP_MOTION_MAX = 46`（允许大运动量下 QP 上浮吸纳熵增）
     - 全局默认基准：`SCREEN_QP_DEFAULT_MIN = 12`, `SCREEN_QP_DEFAULT_MAX = 42`

2. **画质与 QP 控制器（`ScreenQualityController`）**：
   - 在 `tetherly-core` 中实现高内聚状态机：
     - 维护当前的 `profile: ScreenContentProfile`，QP 上下界 `(min_qp, max_qp)` 及自定义覆盖标记 `custom_qp: bool`。
     - **QP 钳位与保序算法**：对任意输入的 `(min, max)`，强制执行：
       $$min' = \mathrm{clamp}(min, 0, 51)$$
       $$max' = \mathrm{clamp}(max, min', 51)$$
     - 切换 Profile 时，若未开启自定义覆盖（`custom_qp == false`），则自动同步更新为对应 Profile 的推荐 QP 范围；若存在显式自定义 QP，则保持自定义范围并标记 `is_custom() == true`。
   - `ScreenSender` 内嵌 `ScreenQualityController`，在状态遥测与流控帧中实时透出当前的画质配置。

3. **底层 OpenH264 编码器无锁感知与热重构（`H264Codec`）**：
   - 维护三个原子字段：`profile_tag: AtomicU8`, `min_qp: AtomicU8`, `max_qp: AtomicU8`，确保在推流并发泵中实现无锁读取与零争用配置。
   - 在每帧编码前检测 Profile 或 QP 是否发生变更：
     - 若发生变更，立即将内部编码状态标记为脏，平滑重构底层 OpenH264 编码实例：
       - `Text` / `Auto` 模式：配置 `UsageType::ScreenContentRealTime`，锁定文本与 IDE 模式；
       - `Motion` 模式：配置 `UsageType::CameraVideoRealTime` 并开启 `RateControlMode::Quality`；
       - 设置 `iMinQp` 与 `iMaxQp` 范围；
     - 重建编码器后强制发出首帧 IDR 恢复关键帧，并在码流中注入全新 SPS/PPS 头部信息，保证对端解码器平滑同步。

4. **实测链路带宽自动分级联动（Auto-Tiering）**：
   - 实现 `screen_profile_from_profile(measured_mbps: Option<f64>) -> ScreenContentProfile`。
   - 在 `screen_profile_auto: true` 模式下，推流建立首帧自动根据对端历史测速进行最佳 Profile 仲裁（桌面远程控制默认优先 Text 模式以保证文字清晰度）。

5. **回环 API 与 CLI 动态调优接口**：
   - **HTTP API**：
     - `GET /api/screen/profile`：查询当前画质简档、QP 范围、自定义标记及自动分级状态（`ScreenProfileReport`）。
     - `POST /api/screen/profile`：支持 JSON 请求体 `{"profile": "text", "min_qp": 10, "max_qp": 30}` 或 Query 参数 `?profile=motion&min_qp=15` 动态热生效。
   - **CLI 命令行**：
     - `tetherly screen profile [profile] [--min-qp <qp>] [--max-qp <qp>]`：查看或覆盖推流画质简档与 QP 范围。
     - `tetherly run --screen-profile <auto|text|motion> --screen-qp <min..max>`：启动参数指定初始画质模式与 QP 限制。
     - `tetherly screen status`：实时状态透出 `Profile: text (QP: 12..35)`。

## 落地记录（v1.99）

- `crates/tetherly-core/src/screen.rs`:
  - 定义 `ScreenContentProfile` 枚举及序列化适配器。
  - 定义 QP 常数界限及推荐阈值（`SCREEN_QP_MIN`, `SCREEN_QP_MAX`, `SCREEN_QP_TEXT_MIN`, `SCREEN_QP_TEXT_MAX` 等）。
  - 实现核心控制器 `ScreenQualityController`，包含 `is_custom()`, `set_profile()`, `set_qp_range()`, `qp_range()`。
  - `ScreenStats` 扩充 `profile: ScreenContentProfile`, `min_qp: u8`, `max_qp: u8`。
  - `ScreenSender` 集成 `quality_controller`。
  - 单元测试：`screen_content_profile_and_quality_controller`。
- `crates/tetherly-core/src/ports.rs` & `lib.rs`:
  - `ScreenCodec` 与 `ScreenSource` 接口扩充 `set_content_profile()`, `content_profile()`, `set_qp_range()`, `qp_range()`。
  - 导出常数、枚举与控制器。
- `crates/tetherly-node/src/screen.rs`:
  - `H264Codec`、`NegotiatedCodec` 与 `EncodedSource` 实现原子画质控制与底层 OpenH264 参数热配置。
  - 实现带宽简档映射辅助函数 `screen_profile_from_profile`。
- `crates/tetherly-node/src/runtime.rs`:
  - `NodeConfig` 新增 `screen_profile`, `screen_profile_auto`, `screen_qp_min`, `screen_qp_max`。
  - 新增 `ScreenProfileReport` 结构体，`ScreenStatusReport` 透出画质简档与 QP 范围。
  - `Node` 提供 `screen_set_profile()`, `screen_set_qp_range()`, `screen_profile_report()` 等控制方法。
  - 首帧推流泵支持画质模式自适应对齐。
- `crates/tetherly-node/src/uihttp.rs` & `bin/tetherly.rs`:
  - 注册 `GET /api/screen/profile` 与 `POST /api/screen/profile` 回环端点。
  - CLI `ScreenCmd::Profile` 与 `Cmd::Run --screen-profile / --screen-qp`。
  - `tetherly screen status` 格式化透出画质简档与 QP 范围。
- `tests/phase5.rs`:
  - 新增 Phase 5 端到端集成测试 `m5_9_screen_dynamic_content_profile`，全面验证 API 热调优、QP 范围钳位、自定义 QP 保留、帧推流传输与远端呈现。
