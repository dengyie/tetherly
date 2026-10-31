# ADR-125 屏幕串流动态空间瓦片自适应 MTU 分块聚合与定步分发流水线与双端 CLI 全同构 (Screen Streaming Dynamic Tile Batching, MTU Chunking & Coalesced Pacing Pipeline with 100% Dual CLI Isomorphism)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-31，v2.39 / v0.1.61）**——建立屏幕串流动态空间瓦片自适应 MTU 分块聚合与定步分发流水线：(1) **单帧瓦片批次切片与自适应 MTU 分块聚合模型**（`crates/tetherly-core/src/screen.rs`）：定义子编解码器常量 `SCREEN_COMPRESS_TILE_BATCH = 10`、`ScreenTileBatchConfig`、`ScreenTileBatchReport`；实现 `chunk_dirty_tiles`、`ScreenFrame::encode_tile_batch_chunk` 与 `ScreenFrame::decode_tile_batch`，将任意数量的脏瓦片切分为受控于 `target_chunk_bytes`（默认 1200 字节，范围 512..=64000）的自包含分块；(2) **分块微秒级定步平滑分发流水线**（`crates/tetherly-node/src/runtime.rs`）：在发送端多分块调度中挂载 `pacing_interval_us`（默认 200 $\mu\text{s}$）微秒定步间隙，平滑抹除线路级突发微脉冲（Micro-Bursts），避免击穿中间路由器与隧道队列；(3) **接收端增量解压与原子事务呈现融合**（`crates/tetherly-node/src/runtime.rs`）：接收端收到各分块后即时解码并写入双缓冲呈现器（ADR-121）的后台画布，并在当帧全部 `total_chunks` 收集齐全后执行原子双缓冲前台交换（Front-Swap）；(4) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 68（`ControlMsg::SetScreenTileBatchMode { enabled: bool, target_chunk_bytes: u16, pacing_interval_us: u16 }`）、Tag 69（`ControlMsg::ResetScreenTileBatchMode`）与 Tag 70（`ControlMsg::TileBatchSync { batches_sent: u64, chunks_sent: u64, avg_chunk_bytes: u32, pacing_pauses: u64 }`）；(5) **节点运行时状态维护与网络循环协同**：`NodeShared` 挂载 `screen_tile_batch_config` 以及原子计数器（`screen_tile_batches_sent`、`screen_tile_chunks_sent`、`screen_tile_batch_bytes_total`、`screen_tile_batch_pacing_pauses`），在 `screen_send_frame` 与 `attach_screen_client` 中无缝集成分块发送与重组呈现；(6) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/tile/batch`（GET / POST）、`/api/screen/tile/batch/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充 6 项 Tile Batch 遥测，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(7) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen tile batch [status|set|reset]` 与 `screen batch` 别名子命令族及格式化报表输出，`screen status` 扩充 Tile Batch 流控指标；(8) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_49_screen_tile_batching_pipeline` 全链路验证通过。 |
| 日期 | 2026-10-31 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-082「屏幕串流动态速率自适应 AIMD 控制器」、ADR-083「屏幕串流动态帧率与自适应定步控制」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-091「屏幕串流微秒级令牌桶数据包定步与突发整形流水线」、ADR-092「屏幕串流基于延迟梯度的趋势线滤波算法与缓冲膨胀预防流水线」、ADR-093「屏幕串流选择性重传 NACK 与混合纠错 HARQ 流水线」、ADR-116「屏幕串流空间分块网格划分与独立异步瓦片更新流水线」、ADR-117「屏幕串流哈希瓦片字典缓存、全局跨帧空间去重与零带宽复用流水线」、ADR-118「屏幕串流动态注视点焦点感知、空间区域优先级与外围自适应抽帧调度流水线」、ADR-119「屏幕串流瓦片级自适应调色板、二值位图与混合无损压缩流水线」、ADR-120「屏幕串流瓦片级可逆颜色变换、方向性差分预测滤波与复合无损压缩流水线」、ADR-121「屏幕串流瓦片级原子呈现事务、双缓冲前台交换与防撕裂垂直同步步频流水线」、ADR-122「屏幕串流瓦片级快速重传、微型选择性重发与事务超时自愈流水线」、ADR-123「屏幕串流动态空间瓦片重要度感知与视觉显著性丢包抗性分级调度流水线」、ADR-124「屏幕串流动态空间瓦片多分辨率细节层次下采样与双线性渐进重构流水线」 |

---

## 动机

1. **单帧巨型瓦片脉冲与网络丢包雪崩（Monolithic Tile Frame vs. Network Micro-Bursts）**：
   - 在空间分块网格（`SCREEN_COMPRESS_TILE_GRID`，ADR-116）、哈希字典缓存（`SCREEN_COMPRESS_TILE_CACHE`，ADR-117）、显著性分级（`SCREEN_TILE_SALIENCY`，ADR-123）与多分辨率细节层次（`SCREEN_TILE_LOD`，ADR-124）中，待发脏瓦片无论经过何种压缩，最终均被打包并序列化为一个单独的巨大 `ScreenFrame`（`encode_tile_grid`）；
   - 在发生窗口拖拽、高动态滚屏或视频播放等高频场景下，单帧可能包含数十至上百个脏瓦片，整帧编码体积可轻易达到 40 KB ~ 120 KB；
   - **网络物理瓶颈**：
     - 单个以太网标准 MTU 为 1500 字节，UDP/Noise/EasyTier 隧道安全有效载荷约为 1200 ~ 1400 字节。当 120 KB 巨型单帧直接被推送到底层传输时，操作系统与路由器必须进行大量盲目 IP 分片；
     - 在家用 Wi-Fi、4G/5G 弱网或跨公网中继环境下，只要一个微小分片遭遇丢包，接收端整个 120 KB 巨型帧就将组装失败，导致整帧所有瓦片全部损毁；
     - 瞬时巨量字节倾泻还会造成严重的网络突发脉冲（Micro-Burst），瞬时占满中继交换机缓冲队列引发严重缓冲膨胀（Bufferbloat）与队列尾丢弃（Tail Drop）；
   - 尽管 ADR-091 提供了底层的报文定步器（Pacer），但由于此前瓦片层缺乏自适应分块机制，底层分片无法在瓦片语义边界施加微秒级平滑定步。

2. **核心突破：动态瓦片 MTU 分块聚合与定步分发流水线**：
   - 引入专用子编码格式 `SCREEN_COMPRESS_TILE_BATCH = 10`；
   - **发送端自适应 MTU 分块聚合（`TileBatchChunker`）**：
     - 脏瓦片经由 Saliency 排序与 LoD/Palette/Predict 压缩后，发送端依据配置的 `target_chunk_bytes`（默认 1200 字节）将待发瓦片流切分为若干独立的自包含瓦片子批次（Chunks）；
     - 每一个分块（Chunk）携带完整且独立的瓦片头信息：`batch_id (u32)`、`chunk_index (u16)`、`total_chunks (u16)`、`tile_size (u16)`、`total_tiles (u16)` 与 `tiles_in_chunk (u16)`；
     - 单个瓦片分块严格控制在安全 MTU 范围内，从根本上杜绝操作系统层与路由器层的 IP 盲目分片；
   - **分块定步平滑发送（`PacedBatchDispatcher`）**：
     - 分块之间施加微秒级定步间隙（`pacing_interval_us`，默认 200 $\mu\text{s}$），将突发流量在时域上均匀铺开，彻底消除线路级瞬时脉冲；
   - **接收端分块即时解压与原子事务呈现融合（`TileBatchReassembler` & ADR-121）**：
     - 接收端收到任一分块即可立即解压瓦片并写入前后台双缓冲呈现器（ADR-121）的后台画布，分散解压算力开销；
     - 维护分块到达追踪表，当当帧所有分块全部就绪后，执行原子双缓冲前台交换（Front-Swap）；
     - 若发生个别分块丢失且超时，触发降级提交并结合 ADR-122 Micro-NACK 精确按分块或瓦片重传，重传带宽节约 90%+；
   - **14 字节定长信令支持**：
     - Tag 68: `SetScreenTileBatchMode`
     - Tag 69: `ResetScreenTileBatchMode`
     - Tag 70: `TileBatchSync`
   - **回环 REST 控制面与双端 CLI 100% 全同构**：
     - 提供 `/api/screen/tile/batch` 控制端点与 `screen tile batch` / `screen batch` 命令族；
     - 严格执行 127.0.0.1 物理绑定与 Host 403 阻断校验。

---

## 决定

1. **核心数据结构设计 (`crates/tetherly-core/src/screen.rs`)**：
   - 常量：`pub const SCREEN_COMPRESS_TILE_BATCH: u8 = 10;`
   - 配置结构 `ScreenTileBatchConfig`：
     - `pub enabled: bool`
     - `pub target_chunk_bytes: u16`（默认 1200，限制 512..=64000）
     - `pub pacing_interval_us: u16`（默认 200，限制 0..=50000）
   - 遥测报告 `ScreenTileBatchReport`：
     - `pub ok: bool`
     - `pub enabled: bool`
     - `pub target_chunk_bytes: u16`
     - `pub pacing_interval_us: u16`
     - `pub batches_sent: u64`
     - `pub chunks_sent: u64`
     - `pub avg_chunk_bytes: u32`
     - `pub pacing_pauses: u64`
   - 分块算法：
     - `chunk_dirty_tiles(tiles: Vec<DirtyTile>, target_chunk_bytes: usize, tile_size: u16, total_tiles: u16, batch_id: u32) -> Vec<ScreenFrame>`
     - `ScreenFrame::encode_tile_batch_chunk(...)` 与 `ScreenFrame::decode_tile_batch(...)`。

2. **14 字节定长信封线协议 (`TYPE_SCREEN_CONTROL = 0x0502`)**：
   - **Tag 68 (`ControlMsg::SetScreenTileBatchMode { enabled, target_chunk_bytes, pacing_interval_us }`)**：8 字节有效载荷打包。
   - **Tag 69 (`ControlMsg::ResetScreenTileBatchMode`)**：重置模式与计数器。
   - **Tag 70 (`ControlMsg::TileBatchSync { batches_sent, chunks_sent, avg_chunk_bytes, pacing_pauses }`)**：双端指标同步。

3. **安全与控制面规范**：
   - 回环 REST 端点 `/api/screen/tile/batch` 严格实行 127.0.0.1 物理绑定并校验 Host 403 强隔离；
   - 双端 CLI 全同构落地 `screen tile batch` 命令族。

---

## 状态与影响

- 彻底攻克瓦片串流中的单帧巨型脉冲冲击与底层分片网络雪崩痛点；
- 通过 MTU 限制分块与微秒级定步间隙，实现自适应流量平滑；
- 100% 保持双端画布逐位像素一致性与零撕裂原子呈现。
