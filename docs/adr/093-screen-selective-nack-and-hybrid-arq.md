# ADR-093 屏幕流选择性重传（NACK）与混合纠错（HARQ）流水线（Screen Streaming Selective NACK Retransmission & Hybrid ARQ Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v2.07）**——实现面向超低时延实时屏幕串流的选择性重传（Selective NACK）与混合纠错（HARQ，Hybrid Automatic Repeat reQuest）流水线（`ScreenHarqMode`、`ScreenNack`、`ScreenRetransmitBuffer`、`ScreenHarqStats`、`ScreenHarqReport`）；统一前向纠错（Cauchy Reed-Solomon FEC，ADR-090）与微秒级定步（Pacer，ADR-091）；设计专用线协议报文（`TYPE_SCREEN_NACK = 0x0505`，`TNACK` 协议魔数，36 字节紧凑报头，64 位位图与动态丢失分片索引表）；建立播放缓冲截止期限约束（Playout Deadline Bounding），当预估到达时延超期时智能降级为关键帧快速恢复（IDR Fallback）；发送端维护 256 容量、300ms 最大存活期的轻量滑动重传环形缓冲区（`ScreenRetransmitBuffer`）；支持 Auto（自适应重传 2 次）、Aggressive（极速重试 3 次）、Conservative（保守重传 1 次）与 Disabled（透传纯 FEC）四种模式；接收端引入基于 RTT 防抖周期的 NACK 洪泛抑制状态机；全面集成至 `ScreenSender`、`ScreenReceiver`、`tetherly-node` 守护进程与回环 REST API（`GET/POST /api/screen/harq`）；实现 100% 全同构双端 CLI（`tetherly screen harq`、`tetherly-cli screen harq`、`--screen-harq` 启动参数及 `screen status` 状态遥测）；新增核心单元测试与端到端自动化集成测试 `m5_17_screen_selective_nack_and_hybrid_arq` |
| 日期 | 2026-10-25 |
| 关联 | ADR-080「双向帧确认与自适应拥塞背压」、ADR-086「屏幕流 4 维自主自适应协同调度器」、ADR-087「屏幕流接收端播放抖动缓冲区与自适应平滑控制器」、ADR-090「屏幕流前向纠错（FEC）与自适应冗余保护流水线」、ADR-091「屏幕流微秒级报文定步与令牌桶突发整形器」、ADR-092「屏幕流基于时延梯度的拥塞控制器与缓冲膨胀趋势线滤波器」 |

## 动机

1. **解决纯前向纠错（FEC）与纯重传（ARQ）的极端网络困境**：
   - 在 ADR-090 中引入的 Cauchy Reed-Solomon FEC 虽然能够在单向传输中实现零时延瞬时丢包恢复，但在网络突发丢包率超过前向冗余保护上限（例如 $k=4, m=2$ 仅能容忍 2 个分片丢失，突发丢弃 3 个分片）时，FEC 译码将彻底失效；
   - 传统方案在 FEC 译码失败后通常只能向发送端申请强制发送 IDR 完整关键帧，导致数百 KB 的关键帧注入网络引发严重突发拥塞（I-Frame Spikes），且端到端画面产生明显的卡顿与黑帧；
   - 另一方面，传统 TCP 式累积重传（Cumulative ARQ）会导致队头阻塞（Head-of-Line Blocking），违背交互式屏幕流的微秒级时效性要求。

2. **构建精细化微秒级 Hybrid ARQ（HARQ）融合架构**：
   - 借鉴现代 5G NR 与超低时延实时多媒体（WebRTC RTX / RFC 4588）的混合纠错机制：**优先由 FEC 消化随机均匀丢包，当丢包超出 FEC 恢复能力但仍在播放截止期限（Playout Deadline）内时，触发微秒级精准选择性重传（Selective NACK）**；
   - 仅对确实缺失的特定分片（Shard Index / Fragment Index）进行按需补传，杜绝整帧重传造成的宝贵带宽浪费；
   - 发送端将重传切片重新注入定步整形器（ADR-091 Pacer），避免重传切片引发二次网络微突发。

3. **播放截止期限约束与关键帧自适应降级（Deadline-Bounded Fallback）**：
   - 屏幕视频帧存在严格的展示生命周期。当接收端检测到分片缺失时，计算重传到达预估时刻：
     $$t_{\text{arrival}} = t_{\text{now}} + \text{RTT}$$
   - 若 $t_{\text{arrival}}$ 已晚于该帧在播放去抖缓冲区（ADR-087 Dejitter Buffer）的预期渲染截止时间（Deadline），则重传毫无意义。此时 NACK 状态机自适应置位 `requires_keyframe_fallback = true`，触发发送端编码器立即切入自适应 IDR 关键帧刷新，避免无谓重传占用带宽。

4. **NACK 报文抑制与防抖控制（Storm Suppression & Debounce）**：
   - 在高丢包恶劣链路下，如果接收端针对同一丢失切片盲目连续发送 NACK，将导致回传通道发生「NACK 风暴（NACK Storm）」并挤占上行信道；
   - 接收端必须维护针对 `(frame_seq, block_id)` 的 NACK 历史时间戳，严格执行 $\max(\text{RTT}, 20\text{ms})$ 的防抖去重周期，并结合滑动窗口对超时分片设置最大重传次数上限。

5. **100% 双端命令行全同构与全栈可观测性准则**：
   - 遵照 Tetherly 架构规范，全部 HARQ 流控能力必须在全功能节点 CLI（`tetherly`）与轻量远控端（`tetherly-cli`）保持全同构的子命令命名、参数体系、状态查询及 JSON 遥测结构。

---

## 决定

1. **专用线协议帧设计（`TYPE_SCREEN_NACK = 0x0505`）**：
   - 报文以 4 字节魔数 `TNACK`（`0x54, 0x4E, 0x41, 0x43`）起始，1 字节协议版本号 `0x01`；
   - 1 字节标志位 `flags`：
     - `0x01` (`SCREEN_NACK_FLAG_FEC`)：指明目标缺失数据属于 FEC 分块还是普通分片；
     - `0x02` (`SCREEN_NACK_FLAG_KEYFRAME_FALLBACK`)：指明由于超期需要发送端强制刷新 IDR 关键帧；
   - 1 字节保留对齐，8 字节帧序列号 `frame_seq`，2 字节分块标识 `block_id`；
   - 8 字节位图掩码 `missing_mask`（低 64 个分片快速掩码索引）；
   - 2 字节缺失分片计数 `count`，后随 $N \times 2$ 字节具体缺失索引数组；
   - 报尾追加 8 字节接收端平滑 `rtt_estimate_us`。

2. **核心状态机与重传缓冲区（`crates/tetherly-core/src/screen.rs`）**：
   - **HARQ 模式枚举（`ScreenHarqMode`）**：
     - `Auto (0)`（默认）：自适应重传，重传次数上限为 2 次，兼顾时延与恢复率；
     - `Aggressive (1)`：进取模式，重传次数上限为 3 次，优先保证无损画质；
     - `Conservative (2)`：保守模式，重传次数上限为 1 次，专为跨国长时延弱网设计；
     - `Disabled (3)`：彻底旁路重传，仅保留纯 FEC 与丢包直接落入 IDR 刷新。
   - **发送端重传滑动窗口（`ScreenRetransmitBuffer`）**：
     - 维护固定容量 256 项的循环环形队列，对发送的每一帧 FEC 分片与未分片视频报文保留缓存；
     - 报文最大存活期限制为 $300\text{ms}$（`SCREEN_HARQ_MAX_AGE_US = 300_000`），超时自动驱逐释放内存；
     - 收到合法的 `ScreenNack` 时，精准索引对应 `(frame_seq, shard_index)`，检查该分片的重传次数是否已达到当前模式设定的重传上限；若未达到则递增计数并提取报文切片返回重传队列，同时更新 `shards_retransmitted` 统计。
   - **接收端 NACK 生成与防抖抑制（`ScreenReceiver`）**：
     - 当收到部分 FEC 分片但无法在有效时间窗口内恢复整帧时，调用 `generate_nack_on_fec(now_us, rtt_estimate_us)` 探测确实缺失的分片；
     - 计算重传到达预期时刻与播放缓冲 deadline，若已超时则标记 `requires_keyframe_fallback = true`；
     - 记录最近一次发送 NACK 的时间戳，若在当前 RTT 保护窗口内则静默去重防抖，避免重复索要。

3. **守护进程运行时与回环 REST API 集成（`tetherly-node`）**：
   - `NodeConfig` 与 `SharedNodeState` 新增 `screen_harq_mode: ScreenHarqMode` 配置项；
   - 在 `attach_screen_host` 的接收循环中，当判定包类型为 `TYPE_SCREEN_NACK` 时，通过 `ScreenNack::decode(&frame.payload)` 解析；
   - 发送端通过 `engine.handle_nack(&nack, now_us)` 提取待重传切片，若标记了关键帧回退则调用 `screen_source.force_idr()`；重传切片经由发送通道有界压入并纳入流控遥测；
   - 暴露异步管理接口：
     - `screen_harq_report()` -> 返回当前模式与详细丢包重传统计；
     - `screen_harq_mode()` 与 `screen_set_harq_mode(mode)`；
   - 回环 HTTP UI 注册端点：
     - `GET /api/screen/harq` -> 返回完整 `ScreenHarqReport` JSON 报表；
     - `POST /api/screen/harq` -> 接收 `{"mode": "..."}` 动态变更 HARQ 运行策略；
   - `ScreenStatusReport` 扩充 `harq_mode`、`harq_shards_retransmitted` 与 `harq_recovered_frames` 字段。

4. **100% 双端命令行全同构（`tetherly` & `tetherly-cli`）**：
   - 守护进程运行参数：`tetherly run --screen-harq <auto|aggressive|conservative|disabled>`；
   - 节点端控制子命令：`tetherly screen harq [MODE] [--json] [--ui-port <PORT>]`；
   - 远控端交互子命令：`tetherly-cli screen harq [MODE] [--json] [--ui-port <PORT>]`；
   - 状态综合监控：`tetherly screen status` 与 `tetherly-cli screen status` 在输出面板中实时渲染 HARQ 模式、重传切片计数与恢复帧数。

---

## 验证与效果

1. **核心单元测试与数据面验证（`crates/tetherly-core`）**：
   - `screen_harq_selective_nack_retransmission_pipeline` 验证了：
     - `ScreenNack` 编码解码全流程与位图掩码提取的 100% 字节精确性；
     - `ScreenRetransmitBuffer` 在连续载入分片后，针对缺失分片 1 的精准重传；
     - 重试计数受限机制：在 Auto 模式下第 1 次与第 2 次重传成功放行，第 3 次被拦截并记录超时；
     - 老化机制：超过 300ms 的过期分片自动剔除并拒绝重传；
     - 接收端 RTT 防抖：在同一 20ms 窗口内的重复请求被严格抑制，窗口过期后恢复放行。

2. **运行时与 REST API 测试（`crates/tetherly-node`）**：
   - `test_uihttp_screen_harq_endpoints` 验证了 `GET /api/screen/harq` 与 `POST /api/screen/harq` 动态调控各模式的正确性；
   - `test_screen_harq_mode_and_reporting` 验证了节点配置、状态报告及模式转换的一致性。

3. **端到端集成测试（`tests/phase5.rs`）**：
   - `m5_17_screen_selective_nack_and_hybrid_arq` 实现了双节点配对连接、HARQ 动态配置测试、实时屏幕通道握手及视频帧流通，验证了整个混合纠错流水线与通道遥测的健壮协同。
