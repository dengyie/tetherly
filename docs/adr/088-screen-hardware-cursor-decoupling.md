# ADR-088 屏幕流硬件光标解耦与高频指针同步协议（Screen Hardware Cursor Decoupling & High-Frequency Pointer Synchronization Protocol）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v2.02）**——解耦鼠标指针位置、可见性与形状更新与桌面视频捕获/编码流水线；采用独立线格式通道 `TYPE_SCREEN_CURSOR (0x0503)` 与 32 字节紧凑帧头，支持标准光标形状语义与最高 64x64 RGBA8 自定义光标位图；设计专用控制器 `ScreenCursorController` 支持位置去重与自适应 125Hz 速率限制（8ms），状态语义与位图更新即时穿透；提供三种工作模式（Decoupled / Embedded / Hidden）；集成于 `ScreenSender`、`ScreenReceiver`、`tetherly-node` 守护进程运行时与回环 REST API（`GET/POST /api/screen/cursor`）；实现 100% 全同构双端 CLI（`tetherly screen cursor` 与 `tetherly-cli screen cursor`） |
| 日期 | 2026-10-25 |
| 关联 | ADR-080「双向帧确认与自适应拥塞背压」、ADR-083「屏幕流动态帧率与自适应定步步频控制器」、ADR-084「屏幕流动态分辨率缩放与自适应下采样控制器」、ADR-086「屏幕流 4 维自主自适应协同调度器」、ADR-087「屏幕流接收端播放抖动缓冲区与自适应平滑控制器」 |

## 动机

1. **消除鼠标橡皮筋与操作拖拽延迟（Rubber-banding & Motion Latency）**：
   - 在传统的远程桌面模式中，鼠标光标直接被 DXGI / X11 / CoreGraphics 捕获并烙印（Burn-in）进视频像素流中；
   - 视频帧从抓屏、降采样缩放、H.264/Zstd 编码、网络分片分包，到接收端重组、抖动缓冲（ADR-087 15~35ms）、硬件解码再到渲染，全链路固有延迟通常在 25~60ms 之间；
   - 鼠标操作属于极高频人机交互（人眼对指针延迟阈值小于 10ms），若指针随视频帧更新，用户会感知强烈的粘滞、拖拽感（Rubber-banding），操作效率大幅下降。
2. **避免有损视频压缩与分辨率缩放导致的指针模糊与边缘光晕**：
   - 视频编码器（如 H.264 / H.265 / AV1）采用 YUV420 颜色采样与 DCT 变换，在高动态或弱网自适应降码率时，指针周围会产生严重的色度失真与边缘振铃效应（Ringing Artifacts）；
   - 在 ADR-084 降分辨率缩放（如 50% 下采样）或 ADR-085 高 QP 压缩下，嵌在视频帧中的细致箭头、文本光标（IBeam）、手型光标会变得模糊不清甚至断裂失真；
   - 指针必须以原生屏幕分辨率独立在接收端客户端表面（Surface/Overlay）上绘制。
3. **摆脱视频帧率节流下的指针抽搐（Decoupling from Video FPS Throttling）**：
   - 在静态代码阅读或低带宽场景下，ADR-083 步频控制器与 ADR-086 4D 调度器可能将视频帧率自动下调至 15 FPS 甚至 5 FPS；
   - 此时视频帧更新间隔长达 66~200ms。若光标绑定在视频帧中，鼠标移动将呈现严重的顿挫跳跃（Stuttering）；
   - 解耦后的光标可独立以 125 Hz 高频匀速同步，无论背景视频帧率降至多低，光标指针移动始终平滑如飞。
4. **光标语义与形状状态同步完整性**：
   - 光标不仅仅是 $(x, y)$ 坐标，还承载丰富的系统上下文（普通箭头、文本输入工字型、超链接手型、四向缩放、等待旋转漏斗、禁止符号以及特定应用自定义的位图指针）；
   - 协议需要高效编码这些语义状态，在极低带宽占用（每秒数 KB）的前提下提供毫秒级就绪体验。

---

## 决定

1. **协议设计与独立线格式通道（Wire Protocol & Frame Typing）**：
   - 新增专用屏幕流光标帧类型常量：
     ```rust
     pub const TYPE_SCREEN_CURSOR: u16 = 0x0503;
     pub const CURSOR_MAGIC: [u8; 4] = *b"TMCU";
     pub const CURSOR_PROTO: u8 = 1;
     pub const CURSOR_BITMAP_MAX_BYTES: usize = 64 * 64 * 4; // 16,384 字节 (64x64 RGBA8)
     pub const SCREEN_CURSOR_MIN_INTERVAL_US: u64 = 8_000;    // 8ms 限频 (~125 Hz)
     ```
   - 紧凑 32 字节固定帧头：
     ```text
     +-----------------------------------------------------------------------+
     | Magic "TMCU" (4B) | Proto=1 (1B) | Seq (8B) | X (4B) | Y (4B)         |
     +-----------------------------------------------------------------------+
     | Visible (1B) | Shape (1B) | HotspotX (2B) | HotspotY (2B)             |
     +-----------------------------------------------------------------------+
     | Width (2B) | Height (2B) | HasCustom (1B) | Reserved (2B)            |
     +-----------------------------------------------------------------------+
     | [Optional] Raw RGBA8 Custom Bitmap Payload (Width * Height * 4 Bytes) |
     +-----------------------------------------------------------------------+
     ```
   - 免解压极速编解码：单包体积仅 32 字节，无堆分配零拷贝反序列化，极大降低发送端与接收端 CPU 消耗。

2. **标准光标语义形状（`CursorShape`）与自定义位图**：
   - 支持 12 种标准光标形状语义枚举：
     - `Arrow (0)`、`IBeam (1)`、`Hand (2)`、`Crosshair (3)`、`SizeAll (4)`、`SizeNS (5)`、`SizeWE (6)`、`SizeNESW (7)`、`SizeNWSE (8)`、`Wait (9)`、`NotAllowed (10)`、`Custom (11)`。
   - 当 `Shape == Custom` 时，携带 hotspot 锚点与尺寸不超过 64x64 的原始 RGBA8 像素位图；标准形状不带位图负荷，带宽开销降至绝对最低。

3. **三种工作模式（`ScreenCursorMode`）**：
   - **`Decoupled (0)`（解耦模式，默认）**：
     - 视频流捕获时剔除鼠标光标，光标状态以高频通道（125 Hz）独立传输；接收端在视频帧顶层原生全分辨率叠加绘制光标；
   - **`Embedded (1)`（嵌入模式）**：
     - 传统兼容模式，光标直接烙印在抓取的桌面图像中，高频光标更新通道关闭；
   - **`Hidden (2)`（隐藏模式）**：
     - 抑制远端光标显示，适用于用户只希望使用本地光标操作（如在单一显示器统一指针映射时）。

4. **控制器去重与自适应限频机制（`ScreenCursorController`）**：
   - **位置去重（Deduplication）**：若新坐标 $(x, y)$ 与上一包一致，且可见性、形状未变，自动静默丢弃（`updates_dropped_dedup`）；
   - **速率限制（Rate-Limiting）**：位置更新间隔强制不低于 `SCREEN_CURSOR_MIN_INTERVAL_US = 8_000\,\mu\text{s}`（约 125 Hz），平稳控制上行发包速率，杜绝鼠标狂甩饱和 QUIC 单流；
   - **语义穿透（Semantic Bypass）**：当光标形状发生变化（如从 Arrow 切换为 Hand）、可见性改变（如打字时光标隐藏）、或收到自定义位图时，立即绕过 8ms 限频窗口，即刻发射同步帧，确保指针形状切换零延迟。

5. **双端 CLI 与回环 REST API 全同构控制**：
   - **REST API**：
     - `GET /api/screen/cursor`：查询光标模式、当前坐标、可见性、形状、已发送数、已接收数、去重丢弃数、限频丢弃数；
     - `POST /api/screen/cursor`：动态切换光标模式（`{"mode": "decoupled" | "embedded" | "hidden"}` 或 `?mode=...`）；
     - `GET /api/screen`：扩展增加 `cursor_mode`、`cursor_x`、`cursor_y`、`cursor_visible`、`cursor_shape`、`cursor_updates_sent`、`cursor_updates_received`。
   - **全同构 CLI 命令行**：
     - `tetherly screen cursor [mode]` 与 `tetherly-cli screen cursor [mode]` 行为 100% 同构；
     - `tetherly run --screen-cursor <decoupled|embedded|hidden>` 启动配置。

---

## 落地验证

1. **核心协议与控制器测试**（`crates/tetherly-core/src/screen.rs`）：
   - `screen_cursor_update_codec_and_roundtrip`：32 字节帧头与带 RGBA8 自定义位图负载的极速序列化与防篡改反序列化；
   - `screen_cursor_controller_dedup_and_rate_limiting`：静态防重复更新丢弃、8ms 速率限制削峰及形状变更即刻穿透验证；
   - `screen_cursor_sender_and_receiver_integration`：发送端光标状态机注入、线格式收发与接收端遥测计数一致性测试。
2. **节点运行时与 REST API 测试**（`crates/tetherly-node/src/runtime.rs` & `uihttp.rs`）：
   - `test_screen_cursor_mode_and_reporting`：节点光标模式默认 Decoupled、动态热修改及状态报告同步；
   - `test_uihttp_screen_cursor_endpoints`：回环 HTTP GET 与 POST `/api/screen/cursor` 接口及 403 恶意 Host 防护测试。
3. **Phase 5 端到端集成测试**（`tests/phase5.rs`）：
   - `m5_12_screen_hardware_cursor_decoupling`：双节点在 45720 屏幕通道建立后，独立于视频帧推送高频光标更新，验证防抖动、去重及接收端高精度呈现。
4. **远程沙盒真实环境验证**：
   - 遵循严格隔离规范，在 LeaseWeb 测试 VPS / 沙盒虚拟机（`mesh-nl` / `10.144.144.12`）上进行真实构建、现场日志捕获与功能验收。
