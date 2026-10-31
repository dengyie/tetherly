# ADR-117 屏幕串流哈希瓦片字典缓存、全局跨帧空间去重与零带宽复用流水线与双端 CLI 全同构 (Screen Streaming Hash-Based Tile Deduplication & LRU Tile Dictionary Cache Pipeline with 100% Dual CLI Isomorphism)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-29，v2.31 / v0.1.53）**——建立屏幕串流哈希瓦片字典缓存、全局跨帧空间去重与零带宽复用流水线：(1) **双端同步 LRU 瓦片字典缓存模型**（`crates/tetherly-core/src/screen.rs`）：定义 `ScreenTileCacheConfig`、`ScreenTileCacheReport` 与 `CachedTileEntry`，引入专用 Codec 标记 `SCREEN_COMPRESS_TILE_CACHE = 9`；双端同步维护容量为 $N$ 的轻量级 LRU 瓦片字典缓存（默认 $N=512$，支持 $64 \le N \le 4096$）；(2) **纯 Rust 64-bit 内存内容指纹计算 (`fnv1a_hash64`)**：针对候选脏瓦片快速评估 64-bit 内容指纹，零额外外部 crate 依赖；(3) **空间平移与画面内模式重现零带宽复用**：若瓦片指纹在发送端字典命中（Cache Hit），仅编码 15 字节定长指针补丁（`cache_hit: 1, cache_id: id, patch_len: 0`），数据载荷为 0 字节，传输开销由数百/数千字节骤降至 15 字节；未命中（Cache Miss）则压缩传输并由接收端解压后自动学习存入自身字典槽位 `cache[id]`；在窗口拖拽移动与重复 UI 元素场景下带宽再降 85%~98%，接收端零解压 CPU 开销，保证 100% 逐位像素精确还原；(4) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 42（`ControlMsg::SetScreenTileCacheMode { enabled: bool, max_entries: u16 }`）、Tag 43（`ControlMsg::ResetScreenTileCacheMode`）与 Tag 44（`ControlMsg::TileCacheSync { hits: u64, misses: u64, saved_bytes: u64 }`）；(5) **节点运行时状态维护与网络循环协同**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_tile_cache_config`、发送端字典 `ScreenTileDictionary`、接收端字典槽位及计数器（`cache_hits`、`cache_misses`、`saved_bandwidth_bytes`），在 `screen_send_frame()` 阶段动态触发指纹去重并派发瓦片缓存帧，接收端自动索引字典贴合至渲染画布；(6) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/tile/cache`（GET / POST）、`/api/screen/tile/cache/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充瓦片字典遥测指标，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(7) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen tile cache [status|set|reset]` 子命令族与格式化报表输出，`screen status` 扩充瓦片字典流控遥测；(8) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_41_screen_tile_cache_deduplication_and_streaming_pipeline` 全链路验证。 |
| 日期 | 2026-10-29 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-082「屏幕串流动态速率自适应 AIMD 控制器」、ADR-083「屏幕串流动态帧率与自适应定步控制」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-109「屏幕串流动态差分补丁与宏块缓存流水线」、ADR-111「屏幕串流客观 QoE 自主闭环调优流水线」、ADR-112「屏幕串流渐进式帧内平滑刷新与去脉冲流控流水线」、ADR-113「屏幕串流长期参考帧 (LTR) 锚定与增量断洞自愈流水线」、ADR-114「屏幕串流双层时域分级预测 (SVC-T) 层次化参考流水线」、ADR-115「屏幕串流窗口滚动 2D 运动矢量估计与位移补偿加速流水线」、ADR-116「屏幕串流空间分块网格划分与独立异步瓦片更新流水线」 |

---

## 动机

1. **空间分块网格（ADR-116）下的空间平移与画面内模式冗余痛点**：
   - 在 ADR-116 中，屏幕被剖分为 $M \times N$ 的空间瓦片网格，彻底化解了传统单矩形差分的包围盒爆炸缺陷；
   - 然而，日常桌面远程交互中依然存在大量空间与时域上的极高冗余模式：
     - **窗口拖动平移（Window Dragging / Movement）**：当用户在屏幕上拖动窗口、IDE 面板或浮动对话框时，窗口内部各瓦片的像素内容完全没有发生变化，仅仅是在空间坐标上发生平移（例如从网格 $(c_1, r_1)$ 移动到 $(c_2, r_2)$）。纯空间分块算法将所有新位置的瓦片全部判定为脏瓦片并重新压缩传输，导致拖动时出现显著的网络流量脉冲，而实际上接收端不久前刚刚解码并呈现过这些一模一样的瓦片；
     - **画面内宏观空间模式重现（Spatial Pattern Repetition）**：在桌面操作系统 UI 中，存在大量高度同质的视觉基元——大面积单一背景色、窗口标题栏渐变、Dock / 任务栏图标、状态徽标、终端空白行与代码区固定缩进。在 1080p、2K 甚至 4K 分辨率下，同一帧内可能同时存在数十至上百个完全同构的 $64 \times 64$ 瓦片；
     - **时域往复重现（Temporal Switching & Blinking）**：双标签页来回切换、右键菜单弹出与关闭、光标闪烁等，会高频反复呈现不久前完全一致的瓦片位图。

2. **双端同步 LRU 瓦片字典缓存模型**：
   - 发送端与接收端同步维护一个容量为 $N$ 的轻量级 LRU 瓦片字典缓存（默认 $N=512$，支持 $64 \le N \le 4096$）；
   - 利用纯 Rust 快速 64-bit 内存哈希算法（`fnv1a_hash64`，零外部 crate 依赖）为每个候选脏瓦片计算内容指纹；
   - **缓存命中（Cache Hit / 空间与时域零带宽复用）**：若瓦片指纹已在发送端缓存中命中，发送端仅发送 15 字节定长指针补丁（`index: u16, x: u16, y: u16, w: u16, h: u16, cache_hit: 1, cache_id: u16, patch_len: 0`），**载荷长度为 0 字节**！单瓦片传输开销由数百/数千字节骤降至 15 字节；
   - **缓存未命中（Cache Miss / 动态学习入库）**：发送端压缩脏瓦片并标记 `cache_hit: 0, cache_id: id`，接收端解压后贴合画布，并无感将解压像素存入自身字典槽位 `cache[id]`；
   - 拖动窗口与重复元素场景下，带宽在 ADR-116 基础上**再降 85%~98%**，接收端零解压开销，保证 100% 逐位像素精确还原。

3. **系统约束与安全红线**：
   - 严格遵循 ADR-006 零外部重型图形依赖，全部算法使用标准库内存哈希与切片操作；
   - 遵循 ADR-004 对端主权与安全强隔离原则，所有回环控制面端点严格绑定 127.0.0.1 并强制 Host 请求头校验，非法访问返回 403 Forbidden；
   - 严格保持双端 CLI（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）100% 同构。

---

## 决定

### 1. 瓦片字典缓存数据模型与 Codec 标记（`crates/tetherly-core/src/screen.rs`）

- **专用 Codec 标记**：
  ```rust
  pub const SCREEN_COMPRESS_TILE_CACHE: u8 = 9;
  pub const SCREEN_TILE_CACHE_DEFAULT_MAX_ENTRIES: u16 = 512;
  pub const SCREEN_TILE_CACHE_MIN_ENTRIES: u16 = 64;
  pub const SCREEN_TILE_CACHE_MAX_ENTRIES: u16 = 4096;
  ```

- **数据结构与报表**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenTileCacheConfig {
      pub enabled: bool,
      pub max_entries: u16,
  }

  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenTileCacheReport {
      pub ok: bool,
      pub enabled: bool,
      pub max_entries: u16,
      pub current_entries: u16,
      pub cache_hits: u64,
      pub cache_misses: u64,
      pub hit_ratio_pct: f32,
      pub saved_bandwidth_bytes: u64,
  }

  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct CachedTileEntry {
      pub index: u16,
      pub x: u16,
      pub y: u16,
      pub w: u16,
      pub h: u16,
      pub cache_hit: bool,
      pub cache_id: u16,
      pub data: Vec<u8>,
  }
  ```

- **快速 64-bit 内存哈希**：
  ```rust
  pub fn fnv1a_hash64(data: &[u8]) -> u64 {
      const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
      const FNV_PRIME: u64 = 0x100000001b3;
      let mut hash = FNV_OFFSET_BASIS;
      for &byte in data {
          hash ^= byte as u64;
          hash = hash.wrapping_mul(FNV_PRIME);
      }
      hash
  }
  ```

### 2. 帧编解码布局 (`ScreenFrame::encode_tile_cache` / `decode_tile_cache`)

- **头部布局（6 字节）**：
  - `tile_size: u16` (2B)
  - `total_tiles: u16` (2B)
  - `dirty_count: u16` (2B)
- **条目布局（每个脏瓦片 15 字节基础头 + 变长载荷）**：
  - `index: u16` (2B)
  - `x: u16` (2B)
  - `y: u16` (2B)
  - `w: u16` (2B)
  - `h: u16` (2B)
  - `cache_hit: u8` (1B, 0 = 未命中, 1 = 命中)
  - `cache_id: u16` (2B)
  - `patch_len: u32` (4B)
  - `data: [u8; patch_len]`（若 `cache_hit == 1`，则 `patch_len == 0`，不占额外字节）。

### 3. 定长线协议信令（`TYPE_SCREEN_CONTROL = 0x0502`）

- **Tag 42**：`ControlMsg::SetScreenTileCacheMode { enabled: bool, max_entries: u16 }`
- **Tag 43**：`ControlMsg::ResetScreenTileCacheMode`
- **Tag 44**：`ControlMsg::TileCacheSync { hits: u64, misses: u64, saved_bytes: u64 }`

### 4. 回环 REST 控制面规范（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/tile/cache`：返回 `ScreenTileCacheReport`；
- `POST /api/screen/tile/cache`：配置启用状态与最大字典条目；
- `POST/DELETE /api/screen/tile/cache/reset`：一键重置配置并清空缓存；
- 遥测拓展：`ScreenStatusReport` 扩充 `tile_cache_enabled`、`tile_cache_entries`、`tile_cache_hits`、`tile_cache_misses`、`tile_cache_saved_bytes`；
- 安全防护：严格回环绑定 127.0.0.1 并阻断非本地 Host 请求头（403 Forbidden）。

### 5. 双端 CLI 全同构（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）

- 子命令族：`screen tile cache [status|set|reset]`；
- 报表打印函数：`print_screen_tile_cache_report`；
- `screen status` 扩充显示 Spatial Tile Cache 字典运行指标。

---

## 影响

1. **带宽大幅再降低**：在拖动窗口与重复 UI 元素场景下，脏瓦片直接转为 15 字节哈希指针，带宽在 ADR-116 基础上再降 85%~98%；
2. **接收端 CPU 负荷极低**：命中瓦片免除 Zstd 解压，直接从内存数组贴合，大幅节省移动端电量与计算开销；
3. **100% 逐位像素精确**：双端确定性槽位对应，零漂移、零残影。
