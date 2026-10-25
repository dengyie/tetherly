# ADR-087 屏幕流接收端播放抖动缓冲区与自适应平滑控制器（Screen Playout Jitter Buffer & Dejittering Controller）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v2.01）**——在接收端解码渲染前端引入自适应抖动缓冲区 `ScreenPlayoutBuffer`，遵循 RFC 3550 EWMA 平滑抖动估计，提供三种工作模式（Adaptive / ZeroLatency / Smooth）、时间定步出队放映、反累积漂移超时丢帧与溢出防护、双向 ACK 解耦即时确认、回环 REST API 与全同构双端 CLI（`tetherly` & `tetherly-cli`）控制 |
| 日期 | 2026-10-25 |
| 关联 | ADR-080「双向帧确认与自适应拥塞背压」、ADR-082「屏幕流动态速率自适应与 AIMD 码率控制器」、ADR-083「屏幕流动态帧率与自适应定步步频控制器」、ADR-086「屏幕流 4 维自主自适应协同调度器」 |

## 动机

1. **网络到达延迟突变（Packet Delay Variation / Jitter）与微卡顿**：
   - 在公网或跨国无线/Overlay 组网（如 EasyTier、WireGuard、Termux/FRP 隧道）中，物理信道时延波动频繁。即便发送端以 60 FPS（$\Delta \approx 16.6\,\text{ms}$）匀速发包，网络抖动也会导致帧到达呈现突发聚集（Frame Bursting）或到达间隙拉大（Frame Starvation）。
   - 接收端若采取「到一帧、解一帧、画一帧」的直接无缓冲渲染模式，会产生肉眼可见的微卡顿（Micro-Stuttering）、渲染器绘制队列突发抖动，极大恶化人眼观感。
2. **零延迟与画面平滑度的权衡张力（Latency vs. Smoothness）**：
   - 远程桌面有两大典型业务场景：
     - **极速交互（鼠标光标、打字敲代码）**：对延迟极其敏感，期望端到端延迟控制在极限毫秒级（$\le 10\,\text{ms}$），宁可承受微弱抖动也不愿人为引入缓冲延迟；
     - **高动态视觉（视频播放、平滑滚动）**：对平滑连贯度极其敏感，微秒级的匀速呈现优先于极低延迟，需要一定的抗抖动时延垫片（Buffer Cushion）。
   - 系统必须具备灵活、动态可配置的工作模式以满足不同场景需求。
3. **消除累积延迟漂移（Cumulative Latency Drift）与队列溢出**：
   - 在高突发拥塞时，如果网络暂停数百毫秒后集中送达多个堆积帧，传统 FIFO 队列若按步就班依次播放，会导致接收端画面永久性落后于主机真实时间（延迟漂移）；
   - 必须设计严格的超时落后丢帧机制（Late Frame Dropping）与队列容量溢出熔断机制，一旦帧落后于现实时间窗，直接跳过并丢弃，使渲染瞬间追赶上发送端最新画面。
4. **网络确认（ACK）与渲染放映的严格解耦**：
   - 发送端的 RTT 估计与拥塞控制（ADR-080 / ADR-082 / ADR-086）依赖帧到达即时返回的 ACK。若将 ACK 延迟到画面最终呈现给显示器时再发送，会将接收端的人为缓冲延迟错误地算作网络 RTT，导致发送端发生严重的错误降速。必须在网络帧到达重组时立即回发 ACK，接收端抖动缓冲区仅在内部解耦控制呈现放映。

---

## 决定

1. **RFC 3550 规范平滑抖动估计（EWMA Jitter Estimation）**：
   - 对连续到达的两帧 $i-1$ 与 $i$，计算其到达时间差与理论发包间隙的绝对差值 $D(i-1, i) = |(t_{\text{recv}, i} - t_{\text{recv}, i-1}) - (t_{\text{send}, i} - t_{\text{send}, i-1})|$；
   - 采用 1/16 权重的指数移动加权平均（EWMA）持续迭代平滑抖动量：
     $$J_i = J_{i-1} + \frac{D(i-1, i) - J_{i-1}}{16}$$
   - 有效滤除单次突发异常尖刺，实时提供毫秒级信道抖动度量。

2. **三种自适应缓冲模式（`ScreenPlayoutMode`）与目标延迟计算**：
   - **`Adaptive (0)`（自适应模式，默认）**：
     - 动态计算目标缓冲延迟：$D_{\text{target}} = \text{clamp}(15\,\text{ms} + 2 \times J, 5\,\text{ms}, 300\,\text{ms})$；
     - 在弱网抖动加大时自动抬高缓冲垫片以平滑帧率，在网络恢复平稳时自动平滑回落，兼顾交互响应与流畅度。
   - **`ZeroLatency (1)`（极致零延迟模式）**：
     - 最小化人工缓冲时延：$D_{\text{target}} = \text{clamp}(5\,\text{ms} + \frac{J}{4}, 5\,\text{ms}, 30\,\text{ms})$；
     - 采取激进的落后帧丢弃策略，专为高频鼠标移动和低延迟敲代码优化。
   - **`Smooth (2)`（平滑优先模式）**：
     - 提供充足的平滑垫片：$D_{\text{target}} = \text{clamp}(35\,\text{ms} + 3 \times J, 30\,\text{ms}, 300\,\text{ms})$；
     - 专为全屏视频回放、动画渲染、平滑页面滚动设计。

3. **微秒级时间戳定步放映与追赶算法（Playout Pacing & Anti-Drift）**：
   - 每一帧推入队列时计算目标播放时间：
     $$P_i = \max(t_{\text{recv}, i} + D_{\text{target}}, P_{i-1} + \Delta_{\text{pacing}})$$
   - **出队放映决策（`ScreenPlayoutAction`）**：
     - 若当前时间 $t_{\text{now}} < P_i$：返回 `Wait(P_i - t_{\text{now}})`，由定时器保持睡眠等待；
     - 若当前时间落后 $t_{\text{now}} - P_i > \text{LATE\_THRESHOLD}$（$80\,\text{ms}$）：判定为严重落后帧，触发 `DropLate`，递增 `dropped_late_frames`，立即检查下一帧以追平现实时间；
     - 正常窗口期内：返回 `Play`，将帧交给解码呈现层。
   - **缓冲区容量熔断**：设置硬容量上限 `SCREEN_PLAYOUT_MAX_CAPACITY = 12` 帧。当队列满载时，主动丢弃最旧未呈现帧（`dropped_overflow_frames`），确保内存有限且绝不引起内存积压。

4. **双端 CLI 与回环 REST API 全同构控制**：
   - **REST API**：
     - `GET /api/screen/playout`：查询当前播放模式、目标延迟、实时抖动、当前缓冲帧数、已播放帧数、超时丢帧数与溢出丢帧数；
     - `POST /api/screen/playout`：支持 JSON 请求体 `{"mode": "adaptive" | "zerolatency" | "smooth"}` 或查询参数 `?mode=...` 动态热调优；
     - `GET /api/screen`：扩展增加 `playout_mode`、`playout_delay_ms`、`playout_jitter_ms`、`playout_buffered` 等实时健康指标。
   - **全同构 CLI 命令行**：
     - `tetherly screen playout [mode]` 与 `tetherly-cli screen playout [mode]` 行为完全一致；
     - `tetherly run --screen-playout <adaptive|zerolatency|smooth>` 启动配置。

---

## 落地验证

1. **核心单元测试**（`crates/tetherly-core/src/screen.rs`）：
   - `screen_playout_modes_and_target_delays`：验证 Adaptive、ZeroLatency、Smooth 三种模式在不同输入抖动下的目标延迟与钳制范围；
   - `screen_playout_jitter_calculation_and_pacing`：验证 RFC 3550 EWMA 抖动递推、帧排序定步及正常出队逻辑；
   - `screen_playout_late_frame_dropping_and_overflow`：验证超时（$>80\,\text{ms}$）自动丢弃追赶机制与容量溢出（$>12$ 帧）熔断机制；
   - `screen_receiver_playout_buffer_integration`：验证 `ScreenReceiver` 管道中 `push_playout` 与 `pop_playout` 的无缝串接。
2. **节点运行时测试**（`crates/tetherly-node/src/runtime.rs` & `uihttp.rs`）：
   - `test_screen_playout_mode_and_reporting`：验证 API 报告序列化、热切换与 `screen_status_report` 状态同步；
   - `test_uihttp_screen_playout_endpoints`：验证 HTTP GET 与 POST `/api/screen/playout` 回环接口交互。
3. **Phase 5 端到端集成测试**（`tests/phase5.rs`）：
   - `m5_11_screen_playout_dejitter_buffer`：双节点 45720 QUIC 屏幕流握手、回环 API 热切换模式、帧穿透抖动缓冲区定步呈现至 `MemoryScreenSink`，验证遥测一致性。
4. **远程沙盒真实环境验证**：
   - 部署至 LeaseWeb 测试 VPS / 沙盒虚拟机（`mesh-nl` / `10.144.144.12`），实时查看现场系统日志与平滑推流指标。
