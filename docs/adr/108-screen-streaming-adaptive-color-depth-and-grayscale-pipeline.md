# ADR-108 屏幕串流自适应色彩深度、极低带宽灰度经济模式与色彩量化流水线（Screen Streaming Adaptive Color Depth, Grayscale Economy & Dynamic Monochrome Mode Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-28，v2.22 / v0.1.44）**——建立屏幕串流自适应色彩深度、极低带宽灰度经济模式与色彩量化流水线：(1) **色彩模式数据模型与高效整数色彩变换算法**（`crates/tetherly-core/src/screen.rs`）：严格遵循 ADR-006 零重型外部依赖铁律，设计 `ScreenColorMode`、`ScreenColorConfig` 与 `ScreenColorReport`；实现 FullColor32、ReducedColor16（RGB565 5-6-5 量化）、Grayscale8（快速定点亮度加权 $Y=(77R+150G+29B)>>8$）与 Monochrome1（可配置二值化阈值 1..254）的 $O(N)$ 原地（In-Place）无多余分配切片算法；(2) **零格式破坏与透明向下兼容**：保留 BGRA8/RGBA8 像素尺寸与内存对齐，将色度熵降至零或极低水平，使 Zstd Lossless 压缩获得 >75% 极高压缩率，使 OpenH264 产生极平整 UV 平面以超低码率运行，彻底兼容已有渲染器与录制/快照/水印管线；(3) **线协议信令定长扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 19（`ControlMsg::SetScreenColorMode { mode: u8, threshold: u8 }`）与 Tag 20（`ControlMsg::ResetScreenColorMode`）；(4) **节点运行时采集流控与网络分发挂载**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_color_mode: Arc<RwLock<ScreenColorConfig>>` 与 `screen_color_updates: Arc<AtomicU64>`，在 `screen_send_frame` 帧抓取并完成 ROI 裁剪后立即执行色彩量化，大幅削减传输带宽与功耗；(5) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/color`（GET / POST）、`/api/screen/color/reset`（POST/DELETE）端点；集成 127.0.0.1 物理绑定与 Host 403 严格隔离；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen color [status|set|reset]` 子命令族与格式化报表输出，`screen status` 扩充展示色彩模式与量化遥测；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_32_screen_adaptive_color_depth_and_grayscale_pipeline` 全链路验证。 |
| 日期 | 2026-10-28 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-082「屏幕串流动态码率自适应控制器与 AIMD 流水线」、ADR-084「屏幕串流动态分辨率降采样与流控自适应流水线」、ADR-085「屏幕串流内容感知画质模式与 QP 自适应流水线」、ADR-089「屏幕串流脏矩形追踪与自适应静噪流水线」、ADR-107「屏幕串流动态感兴趣区域 (ROI) 裁剪与视口漫游流水线」 |

---

## 动机

1. **极弱网络、卫星链路与高丢包环境下的带宽瓶颈**：在野外移动弱网（2G/3G/恶劣4G）、高丢包卫星互联或边缘物联网通信中，信道吞吐可能骤降至 50~200 kbps。全量传输 32 位全彩色（BGRA8/RGBA8）视频流会迅速引发网络拥塞缓冲区膨胀（Bufferbloat）和连续丢帧，导致控制交互严重卡顿；
2. **文字与终端排障场景下色彩信息的冗余性**：在远程运维调试、日志滚动检查、代码结对编辑与终端 CLI 操作中，屏幕内容以黑底白字或白底黑字为主。色彩信息不仅对问题排查贡献极低，更引入了大量空间高频微细噪声与色度变化；
3. **极低色度熵与超高压缩率协同**：
   - **Grayscale8（8位灰度经济模式）**：将 RGB 三通道统一压缩为基于人眼生理感知的亮度标量 $Y = (77R + 150G + 29B) >> 8$。消除了 U/V 色度平面的所有熵变，使 Zstd 压缩率与 H.264 编码码率在无分辨率损失的情况下降低 60%~80%；
   - **Monochrome1（1位单色黑白模式）**：基于可控二值化阈值（默认 128，支持 1..254 微调），将像素非黑即白二值化（0 或 255）。屏幕转化为极简二值图像，在游程编码与熵编码下可节约 95% 以上带宽，并完美适配电子墨水屏（E-Ink）低刷新与高对比度需求；
   - **ReducedColor16（16位降阶彩色）**：执行 RGB565 阶梯截断，在保留彩色语义（如语法高亮、警告红黄色）的前提下滤除微阶色彩抖动；
4. **零破坏与透明向下兼容**：若改变像素内存布局（如直接输出紧凑 1bpp 格式），将迫使下游解码器、显示渲染器、录制容器（ADR-101）、单帧快照（ADR-103）、水印图层（ADR-104）和画板标注（ADR-106）全部重构。通过在原生 BGRA8/RGBA8 像素容器内做原地量化变换，下游所有管线完全零感知，同时享受极致压缩红利；
5. **零外部重型图形依赖（Zero External Crates, ADR-006）**：严格禁止引入 `image`、`opencv` 等依赖，纯 Rust 标准库定点整数运算，单核纳秒级吞吐；
6. **全链路协同与双端 CLI 100% 同构**：色彩模式无缝接入采集流控、控制面 REST API、双端 CLI 与 SSE 实时状态流。

---

## 决定

### 1. 色彩模式数据模型与算法核心（`crates/tetherly-core/src/screen.rs`）

- **数据模型**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "snake_case")]
  pub enum ScreenColorMode {
      #[default]
      FullColor32 = 0,
      ReducedColor16 = 1,
      Grayscale8 = 2,
      Monochrome1 = 3,
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
  pub struct ScreenColorConfig {
      pub mode: ScreenColorMode,
      pub threshold: u8,
  }

  #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenColorReport {
      pub ok: bool,
      pub mode: ScreenColorMode,
      pub threshold: u8,
      pub is_active: bool,
      pub update_count: u64,
  }
  ```
- **核心算法原语 (`apply_color_mode`)**：
  - **Grayscale8**：
    $$Y = ((77 \times R + 150 \times G + 29 \times B) \gg 8) \text{ as } u8$$
    将目标像素的 R、G、B 通道均写入 $Y$，Alpha 通道保持不变；
  - **ReducedColor16**：
    $$R' = (R \ \& \ 0xF8) \mid (R \gg 5)$$
    $$G' = (G \ \& \ 0xFC) \mid (G \gg 6)$$
    $$B' = (B \ \& \ 0xF8) \mid (B \gg 5)$$
    高位回填低位，完整保全 $[0, 255]$ 动态范围边界；
  - **Monochrome1**：
    $$Y = ((77 \times R + 150 \times G + 29 \times B) \gg 8) \text{ as } u8$$
    $$V = \begin{cases} 255, & Y \ge \text{threshold} \\ 0, & Y < \text{threshold} \end{cases}$$
    将目标像素的 R、G、B 通道均写入 $V$。

### 2. 14 字节定长信封线协议扩充（`TYPE_SCREEN_CONTROL = 0x0502`）

- **Tag 19 (`ControlMsg::SetScreenColorMode { mode: u8, threshold: u8 }`)**：
  - 64 位紧凑载荷：bits 0..8: `mode`, bits 8..16: `threshold`，bits 16..64 置 0；
- **Tag 20 (`ControlMsg::ResetScreenColorMode`)**：
  - 64 位载荷置 0，立刻恢复 FullColor32 原生全彩推流。

### 3. 节点运行时管线与生命周期挂接（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载 `screen_color_mode: Arc<RwLock<ScreenColorConfig>>` 与 `screen_color_updates: Arc<AtomicU64>`；
- `screen_send_frame` 捕获到原始帧并经 ROI 裁剪后，若 `color_config.is_active()`，调用 `frame.apply_color_mode(&color_config)?` 原地量化像素，再进入脏区静噪检测与压缩编码；
- `screen_server_loop` 与 `screen_client_loop` 接收信令并双向实时同步。

### 4. 回环 REST 控制面与双端 CLI 全同构

- **REST API**：
  - `GET /api/screen/color`：获取当前色彩深度模式与遥测指标；
  - `POST /api/screen/color`：设置色彩模式与二值化阈值；
  - `POST /api/screen/color/reset` 与 `DELETE /api/screen/color`：复位至 32 位真彩色；
  - 严格继承 `127.0.0.1` 物理绑定与 `Host` 403 强阻断。
- **CLI 子命令族**：
  - `screen color status [--json] [--ui-port <port>]`
  - `screen color set --mode <full|reduced|grayscale|monochrome> [--threshold <1-254>] [--json] [--ui-port <port>]`
  - `screen color reset [--json] [--ui-port <port>]`
  - `screen status` 扩充展示当前色彩模式与量化计数。
