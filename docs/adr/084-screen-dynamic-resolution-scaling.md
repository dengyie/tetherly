# ADR-084 屏幕流动态分辨率自适应降采样控制器（Screen Streaming Dynamic Resolution & Adaptive Downscaling Controller）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v1.98）**——基于定点数双线性插值的预编码降采样控制器（Downscaling Controller 25%–100%）、偶数几何尺寸约束（Even Dimension Clamping）、基于链路带宽简档的阶梯自动分级（<3 Mbps: 50%, 3–10 Mbps: 75%, >=10 Mbps: 100%）、编码器几何变化热重构联动与首帧 IDR 恢复、回环 API 与 CLI 动态调优接口 |
| 日期 | 2026-10-25 |
| 关联 | §9.7「压缩」、ADR-078「链路感知画质自动分级」、ADR-079「屏幕有损编码器与线上协商」、ADR-080「双向帧确认与自适应拥塞背压」、ADR-081「按需关键帧请求与序列断洞自愈恢复」、ADR-082「屏幕流动态速率自适应与 AIMD 码率控制器」、ADR-083「屏幕流动态帧率与自适应定步步频控制器」 |

## 动机

1. **宏块处理压力与编码算力瓶颈**：在高分屏（如 2K/4K/Retina 屏幕）或算力受限的嵌入式/移动设备上，原生分辨率（如 3840×2160）推流时，OpenH264 编码器每帧需处理数以万计的宏块（Macroblocks），导致 CPU 占用飙升、编码耗时超标（> 50 ms），直接破坏了交互式低延迟桌面的体验底线。
2. **极弱网与跨国高抖动链路下的降维纾困**：在可用网络带宽极度受限（< 3 Mbps）或丢包严重的跨国弱网环境中，单纯依赖降低码率（ADR-082）或调低帧率（ADR-083）会导致画面出现大面积块状马赛克。若能将推流分辨率等比下采样至 50%（像素总量锐减为原先的 1/4），不仅大幅缩减了关键帧分片总数，还能在低码率下保持极佳的文字边缘清晰度与交互可用性。
3. **视频编码器 YUV420 偶数边界刚性约束**：H.264 / AVC 标准要求输入 YUV420 图像的宽度和高度必须为偶数（以满足色度子采样 $2\times 2$ 宏块对齐）。若非对称缩放产生奇数几何尺寸（如 1366 缩放 75% 得到 1024.5），会直接导致底层 OpenH264 内存步长越界或编码崩溃。必须在降采样管道中引入确定性的偶数安全截断约束。
4. **零浮点开销的高性能定点数滤波**：桌面抓流往往达到每秒数千万像素量级。为了避免浮点计算带来的性能瓶颈与 SIMD 指令分歧，需采用 16.16 定点数双线性插值算法，以纯整数移位实现高效平滑的色彩插值。
5. **动态热调整与端到端协同**：推流会话在建立与运行过程中，应支持基于对端测速自动分级，同时也允许桌面控制台或回环 API 进行实时无缝热调优，且缩放引起的画面尺寸变化必须能被对端解码呈现端透明接纳。

## 决定

1. **核心缩放控制器（`ScreenScaleController`）与常数界限**：
   - 在 `tetherly-core::screen` 模块中实现确定性的分辨率缩放控制器。
   - **刚性上下界**：缩放比例严格限制在 $[25\%, 100\%]$ 之间（`SCREEN_SCALE_MIN_PCT = 25`, `SCREEN_SCALE_MAX_PCT = 100`），默认比例为 $100\%$ 原画质（`SCREEN_SCALE_DEFAULT_PCT = 100`）。
   - **偶数几何安全截断（Even Dimension Clamping）**：
     对任意原始宽高 $(W_{orig}, H_{orig})$ 与缩放百分比 $\text{scale\_pct}$，计算缩放后尺寸并确保为偶数且 $\ge 2$：
     $$W_{scaled} = \max\left(2, \left(\left\lfloor \frac{W_{orig} \times \text{scale\_pct}}{100} \right\rfloor\right) \ \& \ \sim 1\right)$$
     $$H_{scaled} = \max\left(2, \left(\left\lfloor \frac{H_{orig} \times \text{scale\_pct}}{100} \right\rfloor\right) \ \& \ \sim 1\right)$$
     例如：1366×768 在 50% 下为 682×384（均为偶数），1920×1080 在 75% 下为 1440×810（均为偶数）。

2. **16.16 定点数双线性插值降采样算法（`downscale_bgra8`）**：
   - 采用 16 位定点数（16.16 fixed-point arithmetic）进行水平与垂直加权双线性插值：
     $$x_{src} = \left\lfloor \frac{x_{dst} \times W_{orig}}{W_{dst}} \right\rfloor$$
     $$y_{src} = \left\lfloor \frac{y_{dst} \times H_{orig}}{H_{dst}} \right\rfloor$$
   - 提取相邻四个原始像素 $(P_{00}, P_{10}, P_{01}, P_{11})$，利用权重分数 $(fx, fy)$ 进行纯整数融合：
     $$P_{scaled} = (1-fx)(1-fy)P_{00} + fx(1-fy)P_{10} + (1-fx)fyP_{01} + fx fy P_{11}$$
   - 全程杜绝浮点运算，并在 $100\%$ 原画质模式下触发快速零拷贝透传。

3. **预编码降采样流水线（Pre-Encode Downscaling Pipeline）**：
   - `EncodedSource<S>::grab()` 在获得未压缩 Raw Frame 后，在送入底层编码器 `compress_frame()` 之前，先执行分辨率下采样。
   - `H264Codec` 维护当前编码状态。当检测到帧尺寸发生改变（`state.w != w || state.h != h`）时，立即优雅重置并重建底层 OpenH264 编码实例，生成全新的 SPS/PPS 头信息并强制发送首帧 IDR 恢复关键帧，确保对端解码器平滑重置尺寸而不产生花屏或运动矢量漂移。

4. **实测链路带宽简档自动分级阶梯（Bandwidth Profile Auto-Tiering）**：
   - 实现 `screen_scale_from_profile(measured_mbps: Option<f64>) -> u32`：
     - **Minimal (< 3 Mbps)**: 50% 缩放（画面像素降至 1/4，彻底消除弱网延迟堆叠）
     - **Low / Medium (3–10 Mbps)**: 75% 缩放（平衡可读性与带宽占用）
     - **High ($\ge$ 10 Mbps) 或未测速**: 100% 原始分辨率（全画质输出）
   - 当节点配置了 `screen_scale_auto: true` 时，推流启动首帧将自动对准最优画质分级。

5. **回环 API 与 CLI 动态调优接口**：
   - HTTP API:
     - `GET /api/screen/scale`: 返回当前缩放比例、范围（25%–100%）与自动分级状态（`ScreenScaleReport`）。
     - `POST /api/screen/scale`: 支持 JSON 请求体 `{"scale": 50}` 或 Query 参数 `?scale=50` 动态热调节。
   - CLI 命令行：
     - `tetherly screen scale [scale_pct]`: 查询或热覆盖推流分辨率比例。
     - `tetherly run --screen-scale <auto|N>`: 启动参数支持手动指定初始比例。
     - `tetherly screen status`: 实时状态输出增加 `Scale: N%` 汇报。

## 落地记录（v1.98）

- `crates/tetherly-core/src/screen.rs`:
  - 定义常数：`SCREEN_SCALE_DEFAULT_PCT (100)`, `SCREEN_SCALE_MIN_PCT (25)`, `SCREEN_SCALE_MAX_PCT (100)`。
  - 实现核心控制器 `ScreenScaleController`，包含偶数尺寸截断算法 `scale_dims`。
  - 实现纯整数定点双线性算法 `downscale_bgra8` 与帧缩放封装 `downscale_frame`。
  - `ScreenStats` 扩充 `scale_pct: u32` 指标。
  - `ScreenSender` 集成 `scale_controller`，提供 `set_scale_pct` 与 `scale_pct`。
  - 单元测试：`resolution_scale_controller_and_downscaling`。
- `crates/tetherly-core/src/ports.rs` & `lib.rs`:
  - `ScreenSource` 与 `ScreenCodec` 抽象端口扩充 `set_scale_pct` 与 `scale_pct`。
  - 导出常数、控制器及插值函数。
- `crates/tetherly-node/src/screen.rs`:
  - `H264Codec`、`NegotiatedCodec` 与 `EncodedSource` 实现缩放比例转发与预编码降采样。
  - 底层 OpenH264 尺寸变化检测与 IDR 关键帧热重构。
  - 实现链路测速自动分级阶梯 `screen_scale_from_profile`。
- `crates/tetherly-node/src/runtime.rs`:
  - `NodeConfig` 新增 `screen_scale_pct: u32` 与 `screen_scale_auto: bool`。
  - 新增 `ScreenScaleReport` 结构体，`ScreenStatusReport` 透出 `scale_pct`。
  - `Node` 提供 `screen_set_scale_pct`、`screen_scale_pct`、`screen_scale_report` 等方法。
  - 发送泵首帧根据对端测速自动分级推流分辨率。
- `crates/tetherly-node/src/uihttp.rs` & `bin/tetherly.rs`:
  - 路由 `GET /api/screen/scale` 与 `POST /api/screen/scale`。
  - CLI `ScreenCmd::Scale` 与 `Cmd::Run --screen-scale`。
  - `tetherly screen status` 格式化透出实时分辨率比例。
- `tests/phase5.rs`:
  - 增加 Phase 5 集成测试 `m5_8_screen_dynamic_resolution_scaling`，覆盖 API 查询/热调、范围钳位、50% 降采样帧推流与远端呈现。
