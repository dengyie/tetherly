# ADR-119 屏幕串流瓦片级自适应调色板、二值位图与混合无损压缩流水线与双端 CLI 全同构 (Screen Streaming Tile-Level Adaptive Palette, Bi-Level Bitmap & Hybrid Lossless Compression Pipeline with 100% Dual CLI Isomorphism)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-29，v2.33 / v0.1.55）**——建立屏幕串流瓦片级自适应调色板、二值位图与混合无损压缩流水线：(1) **桌面图形低色彩熵分类与自适应混合压缩模型**（`crates/tetherly-core/src/screen.rs`）：定义 `ScreenTilePaletteConfig` 与 `ScreenTilePaletteReport`；针对桌面 UI、代码终端与文档排版的稀疏色彩分布特性，将候选空间脏瓦片划分为单色瓦片（Solid Color, $K=1$）、二值位图瓦片（Bi-Level Bitmap, $K=2$）、16色索引调色板瓦片（Indexed Palette, $3 \le K \le 16$）与全彩真彩色瓦片（True Color, $K > 16$）；(2) **极速线性颜色直方图扫描与短路探测**（`compress_tile_palette`）：利用栈上小型固定颜色数组在 $< 2\,\mu\text{s}$ 内完成色彩数统计，超过 16 色即刻短路回退；(3) **超高压缩率与纳秒级重构**：单色瓦片仅传输 5 字节（$16384:5 \approx 3276:1$，**99.97% 带宽节省**），接收端单指令 `fill` 重构；二值双色瓦片仅传输 521 字节（$16384:521 \approx 31.4:1$，**96.8% 带宽节省**），紧凑 1-bit 位图无浮点解包；16色调色板传输紧凑 4-bit 半字节索引流，带宽节省达 87% 以上；(4) **100% 数学级无损保证（Bit-for-Bit Lossless Invariance）**：与有损视频编码（H.264/HEVC）不同，所有调色板与二值像素解压后与源端屏幕图像 100% 逐位像素精确一致，彻底杜绝文字边缘模糊与彩边失真；(5) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 50（`ControlMsg::SetScreenTilePaletteMode { enabled: bool, max_colors: u8, enable_rle: bool }`）、Tag 51（`ControlMsg::ResetScreenTilePaletteMode`）与 Tag 52（`ControlMsg::TilePaletteSync { solid_tiles: u64, bilevel_tiles: u64, palette_tiles: u64, saved_bytes: u64 }`）；(6) **节点运行时状态维护与网络循环协同**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_tile_palette_config` 及原子计数器（`solid_count`、`bilevel_count`、`palette_count`、`truecolor_count`、`saved_bytes`），在 `screen_send_frame()` 阶段对脏瓦片实施调色板压缩并统计带宽收益；(7) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/tile/palette`（GET / POST）、`/api/screen/tile/palette/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充调色板流控遥测指标，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(8) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen tile palette [status|set|reset]` 子命令族与格式化报表输出，`screen status` 扩充 Tile Palette 优先级流控遥测；(9) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_43_screen_tile_palette_adaptive_compression_pipeline` 全链路验证。 |
| 日期 | 2026-10-29 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-082「屏幕串流动态速率自适应 AIMD 控制器」、ADR-083「屏幕串流动态帧率与自适应定步控制」、ADR-088「硬件光标解耦与独立流控通道」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-108「屏幕串流自适应色彩深度量化与动态单色/灰度经济模式流水线」、ADR-109「屏幕串流动态差分补丁与宏块缓存流水线」、ADR-116「屏幕串流空间分块网格划分与独立异步瓦片更新流水线」、ADR-117「屏幕串流哈希瓦片字典缓存、全局跨帧空间去重与零带宽复用流水线」、ADR-118「屏幕串流动态注视点焦点感知、空间区域优先级与外围自适应抽帧调度流水线」 |

---

## 动机

1. **桌面 UI 瓦片的典型低色彩熵特性**：
   - 在远程终端、代码编辑（VS Code/Sublime）、文本阅读及办公窗口中，屏幕瓦片（默认 64x64，16,384 字节）展现出极高的局部颜色聚集性与色彩稀疏性；
   - 单个瓦片内部往往仅由少量色彩构成：如编辑器边缘为 1 种纯背景色、终端代码块为 2 种黑白/深浅交替颜色、语法高亮代码块通常不超过 4~16 种颜色。

2. **交错 RGBA 原始全彩压缩瓶颈**：
   - 在 ADR-116（空间瓦片网格）与 ADR-117（瓦片字典缓存）中，未命中缓存的脏瓦片均直接以全彩 32-bit RGBA 交错像素格式（RGBA RGBA RGBA...）传入通用无损算法（如 Zstd）；
   - 四通道数据在空间上交错排列，打碎了像素间的天然游程连续性，导致 16KB 的纯文本/背景瓦片即便经 Zstd 压缩后仍需占用 600~2000 字节，并伴随不可忽视的 CPU 压缩时延。

3. **核心解法：瓦片级自适应调色板、二值位图与混合无损压缩流水线**：
   - **单色模式（Solid Color, $K=1$）**：若瓦片内所有像素全同，直接编码为 1 字节类型 + 4 字节 RGBA（共 5 字节），压缩率高达 $16384 : 5 \approx 3276:1$（**99.97% 带宽节省**），接收端单指令 `fill` 重构；
   - **二值双色模式（Bi-Level Bitmap, $K=2$）**：若瓦片内仅有 2 种颜色，编码为 1 字节类型 + 8 字节双色调色板 + 1-bit/像素紧凑位图（64x64 为 512 字节），总大小 521 字节，压缩率达 $31.4:1$（**96.8% 带宽节省**）；
   - **16 色索引调色板模式（Indexed Palette, $3 \le K \le 16$）**：编码为 1 字节类型 + 1 字节调色板大小 $K$ + $K \times 4$ 字节调色板 + 4-bit/像素半字节流（64x64 为 2048 字节），压缩率在 87% 以上；
   - **真彩色回退（True Color, $K > 16$）**：扫描中若检测到超过 16 种不同颜色，在 $< 2\,\mu\text{s}$ 内快速短路并平滑回退至现有 Zstd / 原始全彩压缩；
   - **100% 数学级无损保证（Bit-for-Bit Lossless Invariance）**：与有损视频编码（H.264/HEVC）不同，所有调色板与二值像素解压后与源端屏幕图像 100% 逐位像素精确一致，彻底杜绝文字边缘模糊、振铃与色彩溢出。

4. **系统约束与安全红线**：
   - 严格遵循 ADR-006 零外部重型图形依赖，所有位图打包解包与调色板操作纯标准库手写；
   - 遵循 ADR-004 对端主权与安全强隔离原则，所有回环控制面端点严格绑定 127.0.0.1 并强制 Host 请求头校验，非法访问返回 403 Forbidden；
   - 严格保持双端 CLI（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）100% 同构。

---

## 决定

### 1. 瓦片子编解码器类型与定长线协议（`crates/tetherly-core/src/screen.rs`）

- **子编解码器标识常量**：
  ```rust
  pub const TILE_SUBCODEC_RAW: u8 = 0;
  pub const TILE_SUBCODEC_ZSTD: u8 = 1;
  pub const TILE_SUBCODEC_SOLID: u8 = 2;
  pub const TILE_SUBCODEC_BILEVEL: u8 = 3;
  pub const TILE_SUBCODEC_PALETTE16: u8 = 4;

  pub const SCREEN_TILE_PALETTE_DEFAULT_MAX_COLORS: u8 = 16;
  ```

- **配置与报表模型**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenTilePaletteConfig {
      pub enabled: bool,
      pub max_colors: u8,
      pub enable_rle: bool,
  }

  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenTilePaletteReport {
      pub ok: bool,
      pub enabled: bool,
      pub max_colors: u8,
      pub enable_rle: bool,
      pub solid_tiles_count: u64,
      pub bilevel_tiles_count: u64,
      pub palette_tiles_count: u64,
      pub truecolor_tiles_count: u64,
      pub saved_bandwidth_bytes: u64,
  }
  ```

- **核心自适应压缩与解压算法**：
  ```rust
  pub fn compress_tile_palette(raw_pixels: &[u8], w: u16, h: u16, config: &ScreenTilePaletteConfig) -> (u8, Vec<u8>);
  pub fn decompress_tile_palette(subcodec: u8, payload: &[u8], w: u16, h: u16, expected_len: usize) -> Result<Vec<u8>, CoreError>;
  ```

- **定长线协议信令（14 字节定长信封，`TYPE_SCREEN_CONTROL = 0x0502`）**：
  - **Tag 50 (`ControlMsg::SetScreenTilePaletteMode { enabled: bool, max_colors: u8, enable_rle: bool }`)**；
  - **Tag 51 (`ControlMsg::ResetScreenTilePaletteMode`)**；
  - **Tag 52 (`ControlMsg::TilePaletteSync { solid_tiles: u64, bilevel_tiles: u64, palette_tiles: u64, saved_bytes: u64 }`)**。

### 2. 回环 REST 控制面规范（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/tile/palette`：返回当前调色板压缩状态与多模态瓦片遥测报表；
- `POST /api/screen/tile/palette`：动态启闭调色板压缩并配置参数；
- `POST/DELETE /api/screen/tile/palette/reset`：一键重置配置并清空统计；
- 安全保障：严格绑定 127.0.0.1 回环地址，并对 Host 头实施 403 Forbidden 校验。

### 3. 双端 CLI 100% 全同构落地

- `bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly` 对称实现：
  - `screen tile palette status [--json] [--ui-port <port>]`
  - `screen tile palette set [--enabled <bool>] [--max-colors <u8>] [--enable-rle <bool>] [--json] [--ui-port <port>]`
  - `screen tile palette reset [--json] [--ui-port <port>]`
  - `screen status` 扩充输出 Tile Palette 压缩运行遥测。

---

## 影响与收益

1. **带宽大幅削减**：单色瓦片带宽节省达 99.97%，二值位图瓦片节省 96.8%，16色调色板瓦片节省 87% 以上；
2. **CPU 算力与功耗节约**：避免对纯色与简单文字瓦片执行全量 Zstd 压缩/解压，编解码时延降至微秒级；
3. **100% 逐位像素无损保真**：完全保留源端字体渲染与高频边缘，彻底避免有损视频编解码造成的字体发虚与彩边；
4. **与上层空间调度完美互补**：在 ADR-116（空间网格）、ADR-117（瓦片字典缓存）与 ADR-118（焦点优先级）的基础上构建瓦片内的高密无损压缩，形成立体多层流控网络。
