# ADR-116 屏幕串流空间分块网格划分与独立异步瓦片更新流水线与双端 CLI 全同构 (Screen Streaming Spatial Tile-Grid Decomposition and Independent Asynchronous Tile Streaming with 100% Dual CLI Isomorphism)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-29，v2.30 / v0.1.52）**——建立屏幕串流空间分块网格划分与独立异步瓦片更新流水线：(1) **空间瓦片网格 (Spatial Tile-Grid) 几何模型与脏瓦片提取**（`crates/tetherly-core/src/screen.rs`）：定义 `ScreenTileConfig`、`ScreenTileReport` 与 `DirtyTile`，新增专用 Codec 标记 `SCREEN_COMPRESS_TILE_GRID = 8`；将屏幕二维平面几何划分为均匀的 $M \times N$ 网格（瓦片尺寸 $T \times T$，$32 \le T \le 256$，默认 $T=64$ 像素）；针对多处离散微小更新（如左上角光标闪烁与右下角时钟跳动），独立判定各瓦片脏状态，彻底根除单矩形差分流水线包围盒面积急剧放大引发的全屏冗余重传（包围盒爆炸）；(2) **离散脏瓦片独立压缩传输与接收端空间重组 Blit**：仅提取并独立 Zstd 压缩真正变化的脏瓦片，发送端跳过无变化的静态瓦片（跳过率通常高达 90%~99%），带宽骤降 90% 以上；接收端根据瓦片索引定位物理坐标，直接执行内存块贴合（`apply_dirty_tile`），实现零重传、零撕裂与逐位像素级精确还原；(3) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 39（`ControlMsg::SetScreenTileMode { enabled: bool, tile_size: u16 }`）、Tag 40（`ControlMsg::ResetScreenTileMode`）与 Tag 41（`ControlMsg::TileGridSync { tiles_sent: u64 }`）；(4) **节点运行时状态维护与网络循环协同**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_tile_config` 及计数器（`grid_frames_sent`、`dirty_tiles_sent`、`skipped_tiles_count`、`saved_bandwidth_bytes`），在 `screen_send_frame()` 阶段动态触发瓦片提取并派发瓦片网格帧，接收端自动贴合各脏瓦片至渲染画布；(5) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/tile`（GET / POST）、`/api/screen/tile/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充瓦片网格遥测指标，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen tile [status|set|reset]` 子命令族与格式化报表输出，`screen status` 扩充瓦片流控遥测；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_40_screen_spatial_tile_grid_decomposition_and_streaming_pipeline` 全链路验证。 |
| 日期 | 2026-10-29 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-082「屏幕串流动态速率自适应 AIMD 控制器」、ADR-083「屏幕串流动态帧率与自适应定步控制」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-109「屏幕串流动态差分补丁与宏块缓存流水线」、ADR-111「屏幕串流客观 QoE 自主闭环调优流水线」、ADR-112「屏幕串流渐进式帧内平滑刷新与去脉冲流控流水线」、ADR-113「屏幕串流长期参考帧 (LTR) 锚定与增量断洞自愈流水线」、ADR-114「屏幕串流双层时域分级预测 (SVC-T) 层次化参考流水线」、ADR-115「屏幕串流窗口滚动 2D 运动矢量估计与位移补偿加速流水线」 |

---

## 动机

1. **单一大矩形差分流水线的「包围盒爆炸 (Bounding Box Explosion)」缺陷**：
   - 在高分辨率显示环境（1080p、2K、4K 与超宽屏）下，桌面的交互行为往往表现为多处空间离散的微小像素变化：
     - 左上角文本编辑器的单字符闪烁或输入光标更新 $(50, 50)$；
     - 右下角系统托盘的任务栏时间秒数跳动 $(3800, 2100)$；
     - 终端窗口内的构建进度条或状态灯跳动 $(1920, 1080)$。
   - 在现有的动态差分补丁（ADR-109）实现中，发送端通过扫描全局像素计算出包围所有差异像素的**单一最小外接矩形（Bounding Box）**；
   - 当两处或多处极小的更新分别位于屏幕的对角线两端时，计算出的外接矩形直接蔓延至屏幕的绝大部分区域（覆盖率往往达 90%~98%）；
   - 这导致中间多达 98% 毫无变化的静态背景像素被迫一同提取、压缩并传输，引发严重的带宽脉冲（Traffic Burst）与 CPU 压缩耗时飙升；在 Wi-Fi 弱网或蜂窝网络下极易导致排队拥塞与瞬时卡顿。

2. **空间分块网格 (Spatial Tile-Grid) 架构与几何划分原理**：
   - 借鉴现代图像压缩与瓦片地图（Tiled Mapping）的分块思想，构建纯 Rust 标准库自包含的极速二维空间瓦片网格剖分系统：
     - **空间几何剖分**：将宽为 $W$、高为 $H$ 的屏幕画面几何剖分为 $cols \times rows$ 个独立瓦片，默认单瓦片尺寸为 $T \times T$（$32 \le T \le 256$，默认 $T=64$ 像素）；屏幕边缘不足一个完整瓦片宽高的部分按实际边界安全截断；
     - **瓦片级独立脏检测 (Tile Dirty Evaluation)**：发送端针对每个瓦片计算与参考帧的局部哈希或内容比较，仅标记真正发生像素变动的瓦片为脏瓦片（Dirty Tile）；
     - **跳过静态背景 (Tile Skipping)**：对于无变动的瓦片（Skipped Tiles），发送端完全不编码任何像素数据。在离散微小更新场景下，仅编码极少数的脏瓦片（如 2 个 $64 \times 64$ 瓦片），跳过率高达 95%~99%，带宽相比包围盒大矩形骤降 90% 以上。

3. **离散脏瓦片提取、压缩与接收端无缝重组 Blit**：
   - **发送端数据压缩与封包**：
     - 发送端为每个脏瓦片独立提取矩形像素并执行 Zstd 压缩；
     - 封装专用 Codec 格式 `SCREEN_COMPRESS_TILE_GRID = 8`，头部包含瓦片网格元数据（`tile_size: u16`, `total_tiles: u16`, `dirty_count: u16`），后续紧跟每个脏瓦片的流式记录（`tile_index: u16`, `compressed_len: u32`, `payload`）；
   - **接收端画布贴合 (`ScreenFrame::apply_dirty_tile`)**：
     - 接收端在收到网格帧后，遍历脏瓦片列表，根据 `tile_index` 换算出屏幕绝对坐标 $(x, y) = (col \times T, row \times T)$；
     - 对每个脏瓦片解压后直接利用 `copy_from_slice` 贴入本地活跃画布（`screen_receiver_canvas`）；
     - 保持 100% 逐位像素精确还原，不改变其他未变动区域的像素，杜绝任何撕裂或闪烁。

4. **系统约束与安全红线**：
   - 严格遵循 ADR-006 零外部重型图形依赖，全部算法使用标准库内存切片与高效遍历；
   - 遵循 ADR-004 对端主权与安全强隔离原则，所有回环控制面端点严格绑定 127.0.0.1 并强制 Host 请求头校验，非法访问返回 403 Forbidden；
   - 严格保持双端 CLI（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）100% 同构。

---

## 决定

### 1. 瓦片网格数据模型与 Codec 标记（`crates/tetherly-core/src/screen.rs`）

- **专用 Codec 标记**：
  ```rust
  pub const SCREEN_COMPRESS_TILE_GRID: u8 = 8;
  ```

- **数据结构与报表**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenTileConfig {
      pub enabled: bool,
      pub tile_size: u16,
  }

  #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenTileReport {
      pub ok: bool,
      pub enabled: bool,
      pub tile_size: u16,
      pub grid_frames_sent: u64,
      pub dirty_tiles_sent: u64,
      pub skipped_tiles_count: u64,
      pub saved_bandwidth_bytes: u64,
  }

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct DirtyTile {
      pub index: u16,
      pub x: u16,
      pub y: u16,
      pub w: u16,
      pub h: u16,
      pub data: Vec<u8>,
  }
  ```

- **瓦片几何划分与脏瓦片提取算法**：
  - `compute_tile_grid_dims(width: u16, height: u16, tile_size: u16) -> (u16, u16, u16)`：返回 $(cols, rows, total\_tiles)$；
  - `extract_dirty_tiles(prev: &ScreenFrame, curr: &ScreenFrame, tile_size: u16) -> Vec<DirtyTile>`：扫描网格内各瓦片区域，仅在检测到内容差异时提取并输出脏瓦片；
  - `ScreenFrame::encode_tile_grid(tile_size: u16, total_tiles: u16, dirty_tiles: &[DirtyTile], codec: &ScreenCodec) -> Result<ScreenFrame, CoreError>`；
  - `ScreenFrame::decode_tile_grid(&self, codec: &ScreenCodec) -> Result<(u16, u16, Vec<DirtyTile>), CoreError>`；
  - `ScreenFrame::apply_dirty_tile(&mut self, tile: &DirtyTile) -> Result<(), CoreError>`。

### 2. 定长线协议信令定义（14 字节定长信封，`TYPE_SCREEN_CONTROL = 0x0502`）

- **Tag 39 (`ControlMsg::SetScreenTileMode { enabled: bool, tile_size: u16 }`)**：
  - 编码至 `seq: (enabled as u64) | ((tile_size as u64) << 8)`；
  - $tile\_size$ 钳位在 $32 \le T \le 256$；
- **Tag 40 (`ControlMsg::ResetScreenTileMode`)**：
  - 编码至 `seq: 0`；重置为默认关闭状态；
- **Tag 41 (`ControlMsg::TileGridSync { tiles_sent: u64 }`)**：
  - 编码至 `seq: tiles_sent`；同步已发送脏瓦片数。

### 3. 节点运行时状态与网络循环协同（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载：
  - `screen_tile_config: Arc<RwLock<ScreenTileConfig>>`
  - `screen_tile_frames_sent: AtomicU64`
  - `screen_tile_dirty_tiles_sent: AtomicU64`
  - `screen_tile_skipped_count: AtomicU64`
  - `screen_tile_saved_bytes: AtomicU64`
  - `screen_sender_tile_canvas: Arc<RwLock<Option<ScreenFrame>>>`
- 屏幕推流循环与收流循环对称处理 Tag 39 / 40 / 41；
- 发送端在检测到 `tile_config.enabled` 时，对比参考画布并提取脏瓦片：
  - 若脏瓦片数大于 0 且脏瓦片数量小于全量上限（例如 $\le 75\%$ 总瓦片数），构造 `SCREEN_COMPRESS_TILE_GRID` 帧发送，更新已发送瓦片计数与跳过瓦片计数，累加节省带宽；
  - 否则平滑回退至普通差分或关键帧；
- 接收端收到 Tag 8 瓦片网格帧后，将各解压后的脏瓦片贴入 `screen_receiver_canvas`，保持画面像素一致性。

### 4. 回环 REST 控制面接口闭环（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/tile` -> 返回 `ScreenTileReport`
- `POST /api/screen/tile` -> 请求体 `{"enabled": true, "tile_size": 64}`
- `POST/DELETE /api/screen/tile/reset` -> 一键重置配置与计数器
- `ScreenStatusReport` 扩充瓦片网格遥测字段，严格实行 127.0.0.1 物理绑定与 Host 请求头校验。

### 5. 双端 CLI 全同构（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）

- 对称实现 `screen tile [status|set|reset]` 子命令族；
- 对称实现格式化报表输出函数 `print_screen_tile_report`；
- `screen status` 扩充显示 Spatial Tile-Grid 流控指标。

---

## 影响

1. **带宽与弱网鲁棒性提升**：
   - 彻底解决离散微小更新场景下单一矩形包围盒爆炸的严重缺陷，带宽开销从数十兆直接降至数十千字节，跳过率超过 90%；
2. **零图像库外部依赖与零安全风险**：
   - 纯 Rust 标准库内存切片切分与贴合，零 C 代码，零泛化图形引擎依赖；
   - 严格的坐标范围越界防护，任何异常瓦片坐标均在解包阶段安全校验并阻断；
3. **协议平滑前向兼容**：
   - 独立 Codec Tag 8 与 Tag 39~41 独立信令，不影响既有 SVC-T、LTR、Motion Scroll 及常规差分流水线。
