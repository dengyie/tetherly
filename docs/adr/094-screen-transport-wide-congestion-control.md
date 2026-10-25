# ADR-094 屏幕流接收端驱动传输层全网拥塞控制与报文到达反馈流水线（Screen Streaming Receiver-Driven Transport-Wide Congestion Control & Packet Arrival Feedback Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v2.08）**——实现面向超低时延实时屏幕串流的接收端驱动传输层全网拥塞控制与报文到达反馈流水线（RFC 8888 风格 TWCC，`ScreenTwccMode`、`ScreenTwccFeedback`、`ScreenTwccReceiverTracker`、`ScreenTwccSenderTracker`、`ScreenTwccStats`、`ScreenTwccReport`）；在传输层为所有屏幕 FEC 分片报文分配全局单调递增的 16 位传输序列号（`transport_seq`，`1..=65535`）；接收端以微秒级时间戳记录每一个切片的实际到达时刻，并按自适应定时周期与包计数阈值聚合生成紧凑反馈帧（`TYPE_SCREEN_TWCC = 0x0506`，`TTWCC` 协议魔数，20 字节报头 + 每包 4 字节紧凑到达增量，丢失报文专用哨兵 `-32768`）；发送端滑动窗口跟踪在途报文并在收到反馈时执行双向配对，以 EWMA 平滑算法实时解算当前瓶颈链路的瞬时实际投递速率（Delivery Rate）、精确包丢失率（Loss Ratio）、平滑往返时延（Smoothed RTT）以及包间排队时延梯度（Inter-packet Delay Gradient）；与 AIMD 速率自适应控制器深度融合，当检测到严重拥塞丢包（> 5%）或投递速率被压制时触发乘性减（Multiplicative Decrease，`force_step_down`）；全面集成至 `ScreenSender`、`ScreenReceiver`、`tetherly-node` 守护进程与回环 REST API（`GET/POST /api/screen/twcc`）；实现 100% 全同构双端 CLI（`tetherly screen twcc`、`tetherly-cli screen twcc`、`--screen-twcc` 启动参数及 `screen status` 状态遥测）；新增核心单元测试、节点级测试与端到端自动化集成测试 `m5_18_screen_receiver_driven_twcc_and_congestion_feedback` |
| 日期 | 2026-10-25 |
| 关联 | ADR-080「双向帧确认与自适应拥塞背压」、ADR-082「屏幕流动态码率自适应控制」、ADR-086「屏幕流 4 维自主自适应协同调度器」、ADR-090「屏幕流前向纠错（FEC）与自适应冗余保护流水线」、ADR-091「屏幕流微秒级报文定步与令牌桶突发整形器」、ADR-092「屏幕流基于时延梯度的拥塞控制器与缓冲膨胀趋势线滤波器」、ADR-093「屏幕流选择性重传（NACK）与混合纠错（HARQ）流水线」 |

## 动机

1. **解决传统帧级别（Frame-Level）拥塞反馈的时效滞后瓶颈**：
   - 在 ADR-080 与 ADR-082 中，拥塞控制依赖接收端在完整帧装配完毕后发送的帧确认（Frame ACK）或基于关键帧间隔的统计反馈。对于一帧分割为数十甚至数百个 FEC 分片的 4K/60FPS 高码率屏幕流，单个分片在网络队列中遭遇排队拥塞时，发送端必须等待整帧所有分片到达或超时后才能感知；
   - 这种「帧级别」的粗粒度反馈存在高达数十毫秒的固有感知延迟，无法在微突发拥塞（Micro-burst Congestion）发生的最初几毫秒内迅速调节发送窗口，极易造成中间路由器缓冲膨胀（Bufferbloat）乃至突发丢包。

2. **借鉴现代 WebRTC Transport-Wide Congestion Control（TWCC / RFC 8888）前沿体系**：
   - 将拥塞控制粒度下沉至**传输报文层级（Per-Packet Level）**。发送端为每个发出的分片赋予全局单调自增的 16 位传输序列号（Transport Sequence Number）；
   - 接收端不仅关注「包是否丢失」，更精细记录「每一个包到达网卡的绝对微秒时刻」；
   - 接收端以高频间隔（5ms ~ 25ms）回传紧凑的报文到达时间差向量（Arrival Time Delta Matrix）。发送端据此不仅能算出丢包，更能算出每个包在传输路径上的单向时延变化（Inter-packet Delay Gradient），在丢包发生前精确探测到网络排队队列的微小积压。

3. **精确解算真实信道投递速率（Delivery Rate Estimation）**：
   - 传统基于发送端发出速率的估计存在幻觉：发送端以为发了 20 Mbps，实际网络物理瓶颈只有 8 Mbps，多余的 12 Mbps 全部堆积在交换机队列中；
   - 借助 TWCC 接收时刻反馈，发送端可以准确计算接收端单位时间内的实际吞吐量：
     $$R_{\text{delivery}} = \frac{\Delta \text{Delivered Bytes} \times 8 \times 10^6}{\Delta t_{\text{arrival}} (\mu s)}$$
   - 将当前编码发送码率硬性钳位在 $R_{\text{delivery}} \times 1.05$，彻底消除发送端过载注入造成的持续缓冲膨胀。

4. **无缝整合 AIMD、FEC、Pacer、Trendline 与 HARQ 全栈流控生态**：
   - TWCC 产生的丢包率、投递速率、往返时延及包间梯度，为整个屏幕流控生态（ADR-082 动态码率、ADR-090 自适应 FEC 矩阵比率、ADR-091 定步间隔计算、ADR-092 趋势线过载检测、ADR-093 HARQ 重传配额）提供了第一手、高保真、低时延的基础度量支撑；
   - 当 TWCC 检测到短时严重拥塞（丢包率 > 5%）时，立即通知 `ScreenRateController` 执行无死区限制的强制乘性降速（`force_step_down`），实现断崖式拥塞的毫秒级自愈。

5. **100% 双端命令行全同构与企业级可观测性准则**：
   - 严格落实 Dual-CLI 设计哲学，`tetherly`（节点守护进程）与 `tetherly-cli`（远控客户端）提供参数命名、终端人机交互展示、JSON 遥测结构全同构的 TWCC 子命令及启动配置。

---

## 决定

1. **紧凑专用线协议设计（`TYPE_SCREEN_TWCC = 0x0506`）**：
   - **固定头部（20 字节）**：
     - `magic: [u8; 5] = *b"TTWCC"`：专用魔数校验；
     - `proto: u8 = 0x01`：协议版本；
     - `flags: u8`：标志位（保留对齐）；
     - `feedback_seq: u8`：反馈帧序号（`0..=255` 循环，用于检测反馈自身的丢失）；
     - `base_seq: u16`：本次反馈记录的第一个报文传输序列号；
     - `packet_count: u16`：本次反馈携带的连续报文条目数量；
     - `reference_time_us: u64`：本次反馈的绝对基准微秒时间戳（接收端本地时钟）。
   - **报文记录数组（每项 4 字节，紧跟在头部后）**：
     - `transport_seq: u16`：具体的传输序列号；
     - `recv_delta_us: i16`：相对于前一个报文到达时刻的增量（微秒），精度满足实时音视频传输；
     - 哨兵常量 `SCREEN_TWCC_MISSING_DELTA: i16 = -32768`（即 `i16::MIN`）：标识该序列号报文在接收端未收到（确认为丢失或跳号）。

2. **传输序列号嵌入与 FEC 分片兼容**：
   - 充分利用已有的 FEC 分片报文头部（`crates/tetherly-core/src/screen.rs`），将每个分片报头中 `hdr[6..8]` 定义为大端序 `transport_seq`；
   - 发送端维护原子递增的 16 位传输序列号分配器（`1..=65535` 循环，0 保留），在发包前注入切片头部；接收端在解析分片时提取序列号并打上 `now_us` 微秒到达时间戳送入接收跟踪器。

3. **核心状态机与跟踪解算器（`crates/tetherly-core/src/screen.rs`）**：
   - **TWCC 运行模式枚举（`ScreenTwccMode`）**：
     - `Auto (0)`（默认）：自适应高频反馈（每 10ms 或累计 40 个报文生成一次反馈）；
     - `Aggressive (1)`：超高频反馈（每 5ms 或累计 20 个报文生成一次反馈），专为竞技级极低时延链路定制；
     - `Conservative (2)`：保守节流模式（每 25ms 或累计 80 个报文生成一次反馈），降低反馈报文占用的反向信道带宽；
     - `Disabled (3)`：彻底旁路 TWCC 反馈与报文序列跟踪。
   - **接收端跟踪器（`ScreenTwccReceiverTracker`）**：
     - 滑动保留最近接收的报文到达记录，自动过滤乱序报文；
     - 维持定时触发器（Interval Timeout）与报文阈值计数器（Threshold Trigger），满足条件时自适应封装 `ScreenTwccFeedback` 并复位累积窗口；
     - 当连续报文序号跳跃时，自动将中间缺失的序号记录标记为 `SCREEN_TWCC_MISSING_DELTA`。
   - **发送端跟踪器（`ScreenTwccSenderTracker`）**：
     - 维护滑动在途报文表（In-Flight Buffer），容量 2048 项，记录每个分片的 `(transport_seq, sent_time_us, bytes)`，超时自动驱逐；
     - 收到 `ScreenTwccFeedback` 时，执行双向序号匹配，计算未确认报文的丢包状态与确认报文的单向往返与投递速率；
     - 采用指数加权移动平均（EWMA，$\alpha = 0.85$）平滑更新 `loss_ratio`、`delivery_rate_bps`、`delay_gradient_us` 与 `smoothed_rtt_us`；
     - 当估算丢包率大于 5% 时，触发底层码率控制器的 `force_step_down()`，并限制发送码率上限不超过当前平滑投递速率的 105%。

4. **运行时系统与本地控制面整合（`tetherly-node`）**：
   - `NodeConfig` 与 `SharedNodeState` 新增 `screen_twcc_mode: ScreenTwccMode` 配置项；
   - 在 Visitor 接收循环中，接收端根据模式配置定期或阈值触发 `receiver.generate_twcc_feedback(now_us)`，编码为 `TYPE_SCREEN_TWCC` 报文回传给 Host；
   - 在 Host 发送循环中，捕获 `TYPE_SCREEN_TWCC` 反馈报文，调用 `engine.handle_twcc_feedback(&feedback, now_us)` 解算网络遥测，更新全局状态并动态联动编码器码率；
   - 暴露异步管理接口：
     - `screen_twcc_report()` -> 返回当前模式与全网统计数据；
     - `screen_twcc_mode()` 与 `screen_set_twcc_mode(mode)`；
   - 回环 HTTP UI 注册端点：
     - `GET /api/screen/twcc` -> 返回完整 `ScreenTwccReport` JSON 报表；
     - `POST /api/screen/twcc` -> 接收 `{"mode": "..."}` 动态变更 TWCC 策略；
   - `ScreenStatusReport` 扩充 6 个 TWCC 关键字段：`twcc_mode`、`twcc_feedbacks_sent`、`twcc_feedbacks_received`、`twcc_delivery_rate_bps`、`twcc_delay_gradient_us`、`twcc_loss_ratio`。

5. **100% 双端命令行同构体验（`tetherly` & `tetherly-cli`）**：
   - 守护进程运行参数：`tetherly run --screen-twcc <auto|aggressive|conservative|disabled>`；
   - 节点端控制子命令：`tetherly screen twcc [MODE] [--json] [--ui-port <PORT>]`；
   - 远控端交互子命令：`tetherly-cli screen twcc [MODE] [--json] [--ui-port <PORT>]`；
   - 状态综合监控：`screen status` 命令实时展示当前 TWCC 模式、反馈收发量、实际链路投递速率、时延梯度与微观丢包率。

---

## 验证与效果

1. **核心单元测试与数据面验证（`crates/tetherly-core`）**：
   - `screen_twcc_transport_wide_congestion_control_pipeline` 全面验证：
     - `ScreenTwccFeedback` 编码与解码的字节精确一致性；
     - 报文接收时间记录与缺失报文哨兵标记机制；
     - 在途发包跟踪、反馈配对与丢包率解算；
     - 投递速率计算与 EWMA 时延梯度跟踪；
     - 模式阈值与反馈触发时机的精准匹配。

2. **运行时与 REST API 测试（`crates/tetherly-node`）**：
   - `test_uihttp_screen_twcc_endpoints` 验证 `GET /api/screen/twcc` 与 `POST /api/screen/twcc` 的模式切换与异常输入校验；
   - `test_screen_twcc_mode_and_reporting` 验证守护进程内状态流转与直接管理接口的幂等一致性。

3. **端到端集成测试（`tests/phase5.rs`）**：
   - `m5_18_screen_receiver_driven_twcc_and_congestion_feedback` 验证了双节点配对下屏幕通道建立、视频帧流动、TWCC 反馈收发及 REST API / 状态遥测字段的完整闭环。
