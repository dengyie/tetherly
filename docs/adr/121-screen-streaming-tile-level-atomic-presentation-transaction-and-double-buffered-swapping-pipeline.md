# ADR-121 屏幕串流瓦片级原子呈现事务、双缓冲前台交换与防撕裂垂直同步步频流水线与双端 CLI 全同构 (Screen Streaming Tile-Level Atomic Presentation Transaction, Double-Buffered Front-Swap & Anti-Tearing V-Sync Pacing Pipeline with 100% Dual CLI Isomorphism)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-30，v2.35 / v0.1.57）**——建立屏幕串流瓦片级原子呈现事务、双缓冲前台交换与防撕裂垂直同步步频流水线：(1) **瓦片级原子呈现事务模型**（`crates/tetherly-core/src/screen.rs`）：定义 `ScreenTilePresentVsyncMode`、`ScreenTilePresentConfig`、`ScreenTilePresentReport` 与双缓冲事务控制器 `ScreenTilePresenter`；彻底解决瓦片网格与缓存流式接收中因增量补丁（In-place patching）导致的跨瓦片局部渲染撕裂（Inter-Tile Visual Tearing）；(2) **后置暂存画布与前后台原子交换（Double-Buffered Staging & Atomic Front-Swap）**：接收端维持专用后置暂存画布（Back Buffer），帧传输期间所有解码后的脏瓦片首先写入后置画布，当前正在展示的前台画布（Front Buffer）保持不可变；整帧瓦片齐备后触发原子指针/内存交换（`std::mem::swap`），实现 100% 杜绝多瓦片渲染撕裂；(3) **防撕裂垂直同步步频控制（Anti-Tearing V-Sync Pacing）**：提供 `Off`、`Vsync60`（16.6ms 周期对齐）、`Vsync120`（8.3ms 周期对齐）与 `Adaptive` 步频模式，使呈现时序与显示器刷新节拍协同，消除微卡顿与屏幕撕裂线；(4) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 56（`ControlMsg::SetScreenTilePresentMode { enabled: bool, vsync_mode: u8, double_buffer: bool, tear_prevention: bool }`）、Tag 57（`ControlMsg::ResetScreenTilePresentMode`）与 Tag 58（`ControlMsg::TilePresentSync { committed_txs: u64, swapped_frames: u64 }`）；(5) **节点运行时状态维护与网络循环协同**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_tile_present_config`、原子计数器（`committed_transactions`、`swapped_frames`、`torn_frames_prevented`、`avg_swap_latency_us`）以及专用后置暂存画布 `screen_receiver_back_buffer`，在 `attach_screen_client` 中无缝集成双缓冲原子交换流水线；(6) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/tile/present`（GET / POST）、`/api/screen/tile/present/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充 8 项呈现事务流控遥测，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(7) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen tile present [status|set|reset]` 与 `screen present` 别名子命令族及格式化报表输出，`screen status` 扩充 Tile Present 流控指标；(8) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_45_screen_tile_presentation_transaction_pipeline` 全链路验证通过。 |
| 日期 | 2026-10-30 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-082「屏幕串流动态速率自适应 AIMD 控制器」、ADR-083「屏幕串流动态帧率与自适应定步控制」、ADR-088「硬件光标解耦与独立流控通道」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-108「屏幕串流自适应色彩深度量化与动态单色/灰度经济模式流水线」、ADR-109「屏幕串流动态差分补丁与宏块缓存流水线」、ADR-116「屏幕串流空间分块网格划分与独立异步瓦片更新流水线」、ADR-117「屏幕串流哈希瓦片字典缓存、全局跨帧空间去重与零带宽复用流水线」、ADR-118「屏幕串流动态注视点焦点感知、空间区域优先级与外围自适应抽帧调度流水线」、ADR-119「屏幕串流瓦片级自适应调色板、二值位图与混合无损压缩流水线」、ADR-120「屏幕串流瓦片级可逆颜色变换、方向性差分预测滤波与复合无损压缩流水线」 |

---

## 动机

1. **瓦片级串流渲染撕裂痛点（Inter-Tile Visual Tearing）**：
   - 在空间分块网格（`SCREEN_COMPRESS_TILE_GRID`）与哈希字典缓存（`SCREEN_COMPRESS_TILE_CACHE`）中，一帧屏幕画面往往包含数十至数百个分散的 64×64 脏瓦片，并通过多个网络 UDP/TCP 数据包分散到达；
   - **现有渲染缺陷**：访客端在解包每个脏瓦片时，直接在当前唯一的展示画布（`screen_receiver_canvas`）上原地覆盖写入（In-place Blitting）。当 UI 刷新线程、Display Sink 呈现回调或外部截屏在瓦片包到达中途读取画布时，屏幕画面将处于「部分瓦片属于新帧、其余瓦片属于旧帧」的未完成中间态；
   - 在窗口快速拖动、网页长文本平滑滚动、播放视频或复杂动态 UI 交互下，用户肉眼能清晰感知到横向或纵向断层裂纹与锯齿错位，严重影响远程交互质感。

2. **垂直同步节拍错位与微卡顿（V-Sync Phase Misalignment）**：
   - 解码器在网络数据包到达并拼装完毕后即刻向操作系统显示接口提交，无序呈现打乱了显示器的固定刷新周期（如 60Hz 的 16.6ms 或 120Hz 的 8.3ms）；
   - 这不仅导致显示扫描线与缓冲区更新相位冲突产生额外撕裂（Display Tearline），更会导致帧间隔忽快忽慢的抖动与微卡顿。

3. **核心设计目标：原子呈现事务与双缓冲流水线**：
   - **双缓冲流水线（Double Buffering）**：
     - **Front Buffer（前台活跃画布）**：仅供 UI/显示槽安全读取与渲染，在事务提交前保持恒定；
     - **Back Buffer（后台暂存画布）**：承接所有解包到达的脏瓦片，作为事务暂存区；
   - **原子事务边界（Presentation Transaction）**：
     - 一帧内的所有脏瓦片构成不可分割的原子事务；
     - 当且仅当整帧的所有脏瓦片全部成功暂存至后台画布后，执行原子指针/缓冲区交换（Atomic Front-Swap），瞬间将完整无撕裂画面翻转至前台；
   - **防撕裂垂直同步步频（Anti-Tearing V-Sync Pacing）**：
     - 支持固定 60Hz、120Hz 或自适应步频平滑对齐，彻底消除显示锯齿与时间轴抖动；
   - **100% 逐位等价性保证（Canvas Invariance）**：
     - 原子交换不改变像素数据本身，解压后像素与源端完全 bit-exact 一致。

4. **安全约束与自包含原则**：
   - 遵循 ADR-006，零外部重型图形库依赖（不引入 OpenGL、Vulkan、DirectX、Skia 或 Cairo），纯 Rust 标准库高效内存互换与双缓冲管理；
   - 遵循 ADR-004，REST 控制面强制绑定 127.0.0.1 并强制 Host 头 403 强隔离校验；
   - 双端 CLI 100% 同构。

---

## 决定

### 1. 呈现事务数据结构与配置模型（`crates/tetherly-core/src/screen.rs`）

- **垂直同步模式枚举**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub enum ScreenTilePresentVsyncMode {
      Off = 0,
      Vsync60 = 1,
      Vsync120 = 2,
      Adaptive = 3,
  }
  ```

- **呈现事务配置与遥测模型**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenTilePresentConfig {
      pub enabled: bool,
      pub vsync_mode: ScreenTilePresentVsyncMode,
      pub double_buffer: bool,
      pub tear_prevention: bool,
  }

  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenTilePresentReport {
      pub ok: bool,
      pub enabled: bool,
      pub vsync_mode: ScreenTilePresentVsyncMode,
      pub double_buffer: bool,
      pub tear_prevention: bool,
      pub committed_transactions: u64,
      pub swapped_frames: u64,
      pub torn_frames_prevented: u64,
      pub avg_swap_latency_us: u64,
  }
  ```

- **双缓冲事务控制器（`ScreenTilePresenter`）**：
  - 管理前台画布、后台暂存画布、事务状态与时延追踪；
  - 提供 `begin_transaction`、`stage_dirty_tile`、`commit_and_swap` 等核心无锁/轻量锁操作。

### 2. 定长线协议信令（14 字节定长信封，`TYPE_SCREEN_CONTROL = 0x0502`）

- **Tag 56: `ControlMsg::SetScreenTilePresentMode { enabled: bool, vsync_mode: u8, double_buffer: bool, tear_prevention: bool }`**
  - 8 字节 Payload 编码格式：
    - bit 0: `enabled`
    - bits 8..15: `vsync_mode` (0=Off, 1=Vsync60, 2=Vsync120, 3=Adaptive)
    - bit 16: `double_buffer`
    - bit 17: `tear_prevention`
- **Tag 57: `ControlMsg::ResetScreenTilePresentMode`**
  - 一键重置为默认关闭状态并清空流控计数。
- **Tag 58: `ControlMsg::TilePresentSync { committed_txs: u64, swapped_frames: u64 }`**
  - 同步双端事务提交数与前台翻转帧数。

### 3. 节点运行时集成（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 状态扩展：
  - `screen_tile_present_config: Arc<RwLock<ScreenTilePresentConfig>>`
  - `screen_tile_present_committed_txs: AtomicU64`
  - `screen_tile_present_swapped_frames: AtomicU64`
  - `screen_tile_present_torn_prevented: AtomicU64`
  - `screen_tile_present_total_latency_us: AtomicU64`
  - `screen_receiver_back_buffer: Arc<RwLock<Option<ScreenFrame>>>`
- 串流处理逻辑集成（`attach_screen_client`）：
  - 当启用双缓冲事务时，解压的脏瓦片首先写入 `screen_receiver_back_buffer`；
  - 瓦片批次就绪后，执行 `std::mem::swap` 原子交换前后台画布；
  - 计算防止撕裂计数（当一帧包含多个脏瓦片时记为一次防撕裂），并提交至呈现槽。

### 4. 安全回环 REST 控制面（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/tile/present`：查询当前呈现状态报告；
- `POST /api/screen/tile/present`：配置呈现事务与防撕裂模式；
- `POST/DELETE /api/screen/tile/present/reset`：重置呈现配置；
- `ScreenStatusReport` 扩充呈现遥测字段；
- 强制校验 Host 头与 127.0.0.1 物理绑定。

### 5. 双端 CLI 100% 全同构

- `screen tile present [status|set|reset]` 与 `screen present` 别名命令族；
- `screen status` 显示 Tile Present 运行遥测。

---

## 影响与安全性

1. **视觉保真度与抗撕裂性**：
   - 彻底根除高速动态交互下的局部多瓦片割裂感；
   - 保证画布像素与宿主机屏幕 100% 位一致性。
2. **性能与内存开销**：
   - 双缓冲仅需额外维护一个屏幕分辨率的 RGBA 帧缓冲（1080p 约 8MB），内存占用轻微可控；
   - 前后台交换通过 Rust 内存所有权指针置换完成，交换延迟在微秒级（< 5µs）。
3. **向后兼容**：
   - 默认保持向后兼容单缓冲直推；启用后平滑切换至双缓冲事务。
