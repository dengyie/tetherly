# ADR-091 屏幕流微秒级报文定步与令牌桶突发整形器（Screen Streaming Microsecond Packet Pacing & Token-Bucket Burst Shaper）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v2.05）**——实现微秒级报文定步与令牌桶突发平滑整形器（`ScreenPacketPacer`、`ScreenPacerStats`、`ScreenPacerReport`、`ScreenPacerMode`）；消除视频关键帧（IDR）及复杂大 P 帧切片后的瞬时网络微突发（Micro-Burst）与路由器队列尾丢弃（Queue Tail-Drop）；支持 Auto（允许 4 包初始突发，剩余分片在帧周期的 80% 内均匀发散）、Aggressive（允许 2 包初始突发，在帧周期的 100% 内均匀发散）与 Disabled（零时延直通）三种运行模式；引入 50ms 硬时延熔断保护（`SCREEN_PACER_MAX_DELAY_US = 50_000`）；深度集成至 `ScreenSender` 异步非阻塞发射流水线、`tetherly-node` 守护进程运行时与回环 REST API（`GET/POST /api/screen/pacer`）；实现 100% 全同构双端 CLI（`tetherly screen pacer`、`tetherly-cli screen pacer`、`--screen-pacer` 启动参数与 `screen status` 状态遥测）；新增端到端自动化集成测试 `m5_15_screen_packet_pacing_and_burst_shaping` |
| 日期 | 2026-10-25 |
| 关联 | ADR-080「双向帧确认与自适应拥塞背压」、ADR-083「屏幕流动态帧率与自适应定步步频控制器」、ADR-084「屏幕流动态分辨率缩放与自适应下采样控制器」、ADR-086「屏幕流 4 维自主自适应协同调度器」、ADR-087「屏幕流接收端播放抖动缓冲区与自适应平滑控制器」、ADR-089「屏幕流脏区差分检测与静止画面自适应静噪控制器」、ADR-090「屏幕流前向纠错（FEC）与自适应冗余保护流水线」 |

## 动机

1. **消除突发微暴流（Micro-Burst）引发的网络队列溢出与随机丢包（Tail-Drop Mitigation）**：
   - 在实时屏幕桌面串流中，视频编码器输出的画面在帧内切片后，单帧（尤其是包含大量运动场景或 IDR 关键帧）通常会切分为 10 ~ 100+ 个 UDP/QUIC 传输层数据报文；
   - 若不加任何平滑控制，发送端网卡会在极短的微秒窗口内将数十个数据包一并倾泻至传输链路中（线速线缆爆发），瞬间压满沿途交换机、家用 Wi-Fi 路由器和 NAT 中继节点的输出队列缓冲区，导致严重的队列溢出尾丢弃（Queue Overflow Tail-Drop）与端到端往返时延（RTT）剧烈抖动；
   - 传统基于 TCP/BBR 的定步往往在系统内核甚至传输连接层生效，缺乏对单视频大帧与呈现周期（Frame Interval）的感知，无法精确在帧生命周期预算内做微秒级自适应整形。

2. **平衡首包交互呈现时延与后续分片平滑发散（Burst Allowance vs Shard Smoothing）**：
   - 实时交互桌面（远程鼠标移动、按键输入回显）极度注重交互响应速度，如果从第 1 个包就开始均匀延时，将人为增加整帧的首包传输延迟，损伤视觉跟手感；
   - 引入带有初始突发配额（Burst Allowance）的令牌桶整形机制：允许初始 $M$ 个分片（Auto 模式 $M=4$，Aggressive 模式 $M=2$）瞬间无阻滞发射，立即唤醒接收端网络接收器并送入重组缓冲区；
   - 超出初始配额的剩余分片，则根据目标帧率计算的帧周期可用预算（Budget $T_f$），按照递增微秒偏移步长（$\Delta t = \text{budget} / (N - M + 1)$）平滑发散输出。

3. **防积压时延硬上限熔断（Pacing Upper-bound Clamping）**：
   - 若帧切片数量极大（例如高分辨率无损大画面或 4K IDR 关键帧切分为数百包），单包计算出的定步延时不得无限延长，否则会导致整帧传输跨越后续帧的产生周期，造成严重的发送端积压（Bufferbloat）与视频撕裂；
   - 必须设定微秒级硬上限（`SCREEN_PACER_MAX_DELAY_US = 50_000`，即 50 毫秒），确保超大突发帧能及时送达，避免破坏全局实时性。

4. **100% 双端命令行同构与全栈可观测性准则**：
   - Tetherly 规范要求所有运行时流控能力必须在全功能节点 CLI（`tetherly`）与轻量控制端（`tetherly-cli`）保持全同构语法、参数与 JSON 交互，并完整暴露定步报文计数、平滑突发次数、累计与均值定步微秒等遥测数据。

---

## 决定

1. **核心数据结构设计（`crates/tetherly-core/src/screen.rs`）**：
   - **定步模式枚举（`ScreenPacerMode`）**：
     - `Auto (0)`（默认）：初始突发配额 4 包，剩余分片在帧周期的 80% 预算内平滑定步；
     - `Aggressive (1)`：初始突发配额 2 包，剩余分片在帧周期的 100% 预算内完全平滑；
     - `Disabled (2)`：透传模式，所有分片无延时（0 $\mu s$）直出。
   - **定步状态与遥测模型（`ScreenPacerStats` / `ScreenPacerReport`）**：
     ```rust
     #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
     pub struct ScreenPacerStats {
         pub mode: ScreenPacerMode,
         pub packets_paced: u64,
         pub bursts_smoothed: u64,
         pub total_delay_us: u64,
         pub avg_delay_us: u64,
         pub last_delay_us: u64,
     }
     ```
   - **报文定步与突发整形器（`ScreenPacketPacer`）**：
     - 常量约束：
       - `SCREEN_PACER_BURST_AUTO: usize = 4;`
       - `SCREEN_PACER_BURST_AGGRESSIVE: usize = 2;`
       - `SCREEN_PACER_MAX_DELAY_US: u64 = 50_000;`（50ms 熔断保护）
     - 微秒延迟计算公式：
       $$\Delta t_i = \begin{cases} 0 & \text{if } i < M \\ \min\left( (i - M + 1) \cdot \frac{T_{\text{budget}}}{N - M + 1}, \text{MAX\_DELAY} \right) & \text{if } i \ge M \end{cases}$$
       其中：$N$ 为分片总数，$M$ 为当前模式的突发包配额，$T_{\text{budget}} = \frac{1,000,000}{\text{fps}} \times \text{ratio}$。

2. **异步发射流水线集成（`crates/tetherly-node/src/runtime.rs`）**：
   - 在 `Node::send_screen_frames` 异步发送链路中，当帧编码为多个 `InnerFrame` 分片后，调用 `engine.compute_packet_delays(frags.len())` 计算各分片的相对发射时间戳；
   - 采用基于 Tokio 协程的高精度非阻塞调度 `tokio::time::sleep(Duration::from_micros(step))` 依次发散报文，杜绝阻塞主事件循环与并发信令流；
   - 完美兼容 ADR-080 拥塞背压（`tx.capacity() < frags.len()` 丢帧保护）与 ADR-090 前向纠错分片。

3. **回环 REST API 规范（`crates/tetherly-node/src/uihttp.rs`）**：
   - `GET /api/screen/pacer`：查询当前定步模式、已定步分片数、平滑突发次数、平均及末次延迟；
   - `POST /api/screen/pacer`：动态切换定步模式（支持请求体 JSON `{"mode": "aggressive"}` 或 URL 查询参数 `?mode=auto`）；
   - `GET /api/screen` 综合状态接口同步扩充 `pacer_mode`、`pacer_packets_paced`、`pacer_bursts_smoothed`、`pacer_avg_delay_us` 字段。

4. **100% 双端命令行全同构规范（`crates/tetherly-node/src/bin/tetherly.rs` & `bins/tetherly-cli/src/main.rs`）**：
   - 守护进程启动参数：`tetherly run --screen-pacer <auto|aggressive|disabled>`；
   - 独立配置子命令：
     ```bash
     tetherly screen pacer [auto|aggressive|disabled] [--json] [--ui-port <port>]
     tetherly-cli screen pacer [auto|aggressive|disabled] [--json] [--ui-port <port>]
     ```
   - 状态输出无缝展示定步指标与平滑效率。

---

## 验证与落地成效

1. **单元测试与集成测试（100% 通过）**：
   - `crates/tetherly-core/src/screen.rs`：`screen_packet_pacer_burst_smoothing_and_modes` 覆盖全部模式切换、突发限额截断、平滑延迟递增性与 50ms 上限熔断；
   - `crates/tetherly-node/src/uihttp.rs`：`test_uihttp_screen_pacer_endpoints` 验证 REST GET/POST 查询、模式修改、非法参数 400 校验及 Host Header 403 安全防护；
   - `tests/phase5.rs`：`m5_15_screen_packet_pacing_and_burst_shaping` 验证双节点配对、动态切换模式、真实大帧微秒定步发射与接收端完整装配。

2. **代码质量标准**：
   - `cargo clippy --all-targets --all-features -- -D warnings` 零告警通过；
   - `cargo test --workspace` 全工作区所有阶段单元测试与集成测试全部绿灯通过。
