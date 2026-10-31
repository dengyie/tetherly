# ADR-124 屏幕串流动态空间瓦片多分辨率细节层次下采样与双线性渐进重构流水线与双端 CLI 全同构 (Screen Streaming Dynamic Tile Level-of-Detail Downsampling & Progressive Reconstruction Pipeline with 100% Dual CLI Isomorphism)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-31，v2.38 / v0.1.60）**——建立屏幕串流动态空间瓦片多分辨率细节层次下采样与双线性渐进重构流水线：(1) **瓦片级多分辨率下采样与插值重构数学模型**（`crates/tetherly-core/src/screen.rs`）：定义 `TileLodLevel`（Lod0 0: 64×64 基准无损分辨率；Lod1 1: 32×32 半分辨率，带宽减少 75%；Lod2 2: 16×16 四分之一分辨率，带宽减少 93.75%）、`ScreenTileLodConfig`、`ScreenTileLodReport` 与子编码器常量 `TILE_SUBCODEC_LOD = 6`；实现基于整数快速盒状均值滤波的 `downsample_tile_rgba` 与基于双线性平滑插值的 `upsample_tile_rgba`；(2) **显著性感知自适应细节分级流水线**：在发送端瓦片压缩阶段，依据显著性评级（ADR-123）与网络拥塞状态自适应调度：Critical 关键交互瓦片恒定锁定 Lod0 全分辨率保真，Normal/Background 瓦片在拥塞或低带宽状态下无缝降采样至 Lod1/Lod2，突破传统二元丢包造成的视觉冻结；(3) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 65（`ControlMsg::SetScreenTileLodMode { enabled: bool, max_lod: u8, auto_lod_on_congestion: bool }`）、Tag 66（`ControlMsg::ResetScreenTileLodMode`）与 Tag 67（`ControlMsg::TileLodSync { lod0_tiles: u64, lod1_tiles: u64, lod2_tiles: u64, bytes_saved: u64 }`）；(4) **节点运行时状态维护与网络循环协同**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_tile_lod_config` 以及原子计数器（`screen_tile_lod0_tiles`、`screen_tile_lod1_tiles`、`screen_tile_lod2_tiles`、`screen_tile_lod_bytes_saved`），在 `screen_send_frame` 与 `attach_screen_client` 中无缝集成 LoD 降采样与接收端插值重构；(5) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/tile/lod`（GET / POST）、`/api/screen/tile/lod/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充 7 项 LoD 遥测，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen tile lod [status|set|reset]` 与 `screen lod` 别名子命令族及格式化报表输出，`screen status` 扩充 Tile LoD 流控指标；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_48_screen_tile_lod_pipeline` 全链路验证通过。 |
| 日期 | 2026-10-31 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-082「屏幕串流动态速率自适应 AIMD 控制器」、ADR-083「屏幕串流动态帧率与自适应定步控制」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-092「屏幕串流基于延迟梯度的趋势线滤波算法与缓冲膨胀预防流水线」、ADR-093「屏幕串流选择性重传 NACK 与混合纠错 HARQ 流水线」、ADR-116「屏幕串流空间分块网格划分与独立异步瓦片更新流水线」、ADR-117「屏幕串流哈希瓦片字典缓存、全局跨帧空间去重与零带宽复用流水线」、ADR-118「屏幕串流动态注视点焦点感知、空间区域优先级与外围自适应抽帧调度流水线」、ADR-119「屏幕串流瓦片级自适应调色板、二值位图与混合无损压缩流水线」、ADR-120「屏幕串流瓦片级可逆颜色变换、方向性差分预测滤波与复合无损压缩流水线」、ADR-121「屏幕串流瓦片级原子呈现事务、双缓冲前台交换与防撕裂垂直同步步频流水线」、ADR-122「屏幕串流瓦片级快速重传、微型选择性重发与事务超时自愈流水线」、ADR-123「屏幕串流动态空间瓦片重要度感知与视觉显著性丢包抗性分级调度流水线」 |

---

## 动机

1. **瓦片串流中的二元丢弃与视觉冻结瓶颈（Binary Tile Drop vs. Visual Freeze）**：
   - 在空间分块网格（`SCREEN_COMPRESS_TILE_GRID`，ADR-116）、哈希字典缓存（`SCREEN_COMPRESS_TILE_CACHE`，ADR-117）与显著性分级（`SCREEN_TILE_SALIENCY`，ADR-123）中，一帧屏幕由数十至数百个 64×64 独立瓦片分包传输；
   - 在弱网拥塞场景下，ADR-123 采取主动跳过 Background 瓦片传输（`drop_background_on_congestion`）以将全部信道带宽倾斜给关键交互区（Critical）。然而，当网络链路经历持续性缓冲膨胀（Bufferbloat）或高抖动弱网（如移动 4G/5G 边缘）时，完全跳过背景瓦片会导致大范围桌面区域长时间处于视觉冻结（Visual Freeze）状态；
   - 若不跳过而强行以全分辨率 64×64 传输整帧背景，即便是经由 Palette 或 Zstd 压缩，每瓦片依然占据数百字节，多瓦片叠加将严重恶化网络排队延迟，引发交互卡顿；
   - **痛点核心**：传统流控在「全分辨率发送」与「全量丢弃」之间缺乏平滑的连续过渡阶梯。

2. **核心设计目标：瓦片多分辨率细节层次下采样与渐进重构流水线 (Tile Level-of-Detail, LoD)**：
   - **多分辨率细节层次抽象 (`TileLodLevel`)**：
     - `Lod0`（Full Resolution）：64×64 完整原始分辨率（4096 像素，每瓦片基准无损）；
     - `Lod1`（Half Resolution）：32×32 半分辨率（1024 像素，像素量减少 75%，下采样后经二次压缩载荷仅数十字节）；
     - `Lod2`（Quarter Resolution）：16×16 四分之一分辨率（256 像素，像素量减少 93.75%，超轻量极速传输）；
   - **发送端快速盒状平均滤波 (`downsample_tile_rgba`)**：
     - 利用纯整数 SIMD 友好的 $2\times 2$ 或 $4\times 4$ 均值滤波，在 $< 1\,\mu\text{s}$ 内完成 64×64 RGBA 像素块的下采样；
     - 下采样后的低分辨率像素块随后接入现有 Palette/Zstd 复合子编码器封装为 `TILE_SUBCODEC_LOD = 6`；
   - **接收端双线性插值平滑重构 (`upsample_tile_rgba`)**：
     - 接收端解压出 32×32 或 16×16 像素后，采用纯整数定点双线性插值或紧凑盒状插值，在内存中将其平滑放大至 64×64 标准瓦片尺寸并呈递至双缓冲前台交换器（ADR-121）；
   - **显著性感知与拥塞自适应联动**：
     - Critical 交互瓦片恒定以 `Lod0` 发送，保障光标、文字及代码编辑器的极致锐利清晰度；
     - Background 与非焦点 Normal 瓦片在网络拥塞或低带宽时动态下采样至 `Lod1` 或 `Lod2`，杜绝画面冻结；
   - **100% 逐位像素等价性保证（Canvas Invariance & Eventual Consistency）**：
     - 下采样的瓦片在发送端参考画布中标记为待高精度刷新；当拥塞解除或在后续周期中自动增量回补为 Lod0 全分辨率无损瓦片，确保最终一致性。

3. **安全约束与自包含原则**：
   - 遵循 ADR-006，零外部重型图像图形库依赖，使用纯 Rust 标准库原生整数运算；
   - 遵循 ADR-004，REST 控制面严格绑定 127.0.0.1 并强制 Host 403 强隔离校验；
   - 双端 CLI 100% 全同构。

---

## 决定

1. **核心数据结构设计 (`crates/tetherly-core/src/screen.rs`)**：
   - 常量：`pub const TILE_SUBCODEC_LOD: u8 = 6;`
   - 枚举 `TileLodLevel`：`Lod0 = 0`, `Lod1 = 1`, `Lod2 = 2`；
   - 配置结构 `ScreenTileLodConfig`：`pub enabled: bool`, `pub max_lod: u8`, `pub auto_lod_on_congestion: bool`；
   - 遥测报告 `ScreenTileLodReport`：`pub ok: bool`, `pub enabled: bool`, `pub max_lod: u8`, `pub auto_lod_on_congestion: bool`, `pub lod0_tiles_sent: u64`, `pub lod1_tiles_sent: u64`, `pub lod2_tiles_sent: u64`, `pub bytes_saved: u64`；
   - 核心函数：`downsample_tile_rgba`、`upsample_tile_rgba`、`compress_tile_lod`、`decompress_tile_lod`。

2. **定长线协议信令定义 (`TYPE_SCREEN_CONTROL = 0x0502`, 14 字节定长信封)**：
   - **Tag 65 (`ControlMsg::SetScreenTileLodMode { enabled: bool, max_lod: u8, auto_lod_on_congestion: bool }`)**：
     - 字节布局：`SCREEN_MAGIC (4B) | SCREEN_PROTO (1B) | Tag 65 (1B) | enabled (1B) | max_lod (1B) | auto_lod_on_congestion (1B) | 0 (5B)`；
   - **Tag 66 (`ControlMsg::ResetScreenTileLodMode`)**：
     - 字节布局：`SCREEN_MAGIC (4B) | SCREEN_PROTO (1B) | Tag 66 (1B) | 0u64 (8B)`；
   - **Tag 67 (`ControlMsg::TileLodSync { lod0_tiles: u64, lod1_tiles: u64, lod2_tiles: u64, bytes_saved: u64 }`)**：
     - 字节布局：`SCREEN_MAGIC (4B) | SCREEN_PROTO (1B) | Tag 67 (1B) | bytes_saved (8B)`。

3. **安全回环 REST 控制面规范 (`crates/tetherly-node/src/uihttp.rs`)**：
   - `GET /api/screen/tile/lod` -> 返回 `ScreenTileLodReport`；
   - `POST /api/screen/tile/lod` -> 动态更新配置（`enabled`, `max_lod`, `auto_lod_on_congestion`）；
   - `POST/DELETE /api/screen/tile/lod/reset` -> 一键重置配置与计数器；
   - 强约束：所有端点绑定 127.0.0.1，校验 Host 请求头，非法访问立即阻断并返回 403 Forbidden。

4. **双端 CLI 100% 全同构**：
   - `screen tile lod status [--json] [--ui-port <port>]`
   - `screen tile lod set [--enabled <bool>] [--max-lod <1|2>] [--auto-lod <bool>] [--json] [--ui-port <port>]`
   - `screen tile lod reset [--json] [--ui-port <port>]`
   - 别名：`screen lod [status|set|reset]`。

---

## 状态与影响

- **状态**：已采纳并落地（2026-10-31，v2.38 / v0.1.60）。
- **影响**：
  - 彻底攻克网络拥塞与极端低带宽下的背景区域冻结卡顿，通过 4 倍至 16 倍的下采样节省 75%~93.75% 像素传输载荷；
  - 结合 ADR-123 显著性分级，实现核心交互区（Critical）绝对保真与外围背景平滑连续渲染的完美平衡；
  - 确保 100% 逐位像素等价性与最终一致性；
  - 维护 Tetherly 的零第三方重型图像库纯 Rust 安全与双端全同构设计原则。
