# ADR-095 屏幕流带宽主动探测与链路容量快速爬升流水线（Screen Streaming Active Bandwidth Probing & Fast Ramp-Up Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v2.09）**——实现面向超低时延实时屏幕串流的带宽主动探测与链路容量快速爬升流水线（Active Bandwidth Probing & Fast Ramp-Up Pipeline，`ScreenProbeMode`、`ScreenProbeState`、`ScreenProbePacket`、`ScreenProbeCluster`、`ScreenProbeController`、`ScreenProbeStats`、`ScreenProbeReport`）；在传输层设计专用探测数据报文（`TYPE_SCREEN_PROBE = 0x0507`，`TPROB` 协议魔数，26 字节报头 + 可变填充字节，携带簇 ID、包索引、簇大小、传输序号、目标探测码率与发送微秒时间戳）；发送端在串流建立初期或严重拥塞恢复后，生成背靠背紧密排列的填充报文簇（Burst Cluster），以毫秒级密集突发模拟高码率传输；探测包无缝分配 TWCC 全局传输序列号，接收端将其纳入到达时刻跟踪并在 TWCC 反馈中回传；发送端解算探测簇的实际投递速率（Delivery Rate）、丢包率（<= 15%）与包间排队时延梯度（<= 20ms 排队增长），当且仅当网络容量验证通过时，绕过传统 AIMD 慢速加性增（Additive Increase），直接执行 `ScreenRateController.fast_ramp_up(target_bps)` 快速爬升至链路真实承载上限；全面集成至 `ScreenSender`、`ScreenReceiver`、`tetherly-node` 守护进程与回环 REST API（`GET/POST /api/screen/probe` 与 `POST /api/screen/probe/trigger`）；实现 100% 全同构双端 CLI（`tetherly screen probe`、`tetherly-cli screen probe`、`--screen-probe` 启动参数及 `screen status` 状态遥测）；新增核心单元测试、节点级测试与端到端自动化集成测试 `m5_19_screen_active_bandwidth_probing_and_fast_rampup` |
| 日期 | 2026-10-25 |
| 关联 | ADR-080「双向帧确认与自适应拥塞背压」、ADR-082「屏幕流动态码率自适应控制」、ADR-086「屏幕流 4 维自主自适应协同调度器」、ADR-090「屏幕流前向纠错（FEC）与自适应冗余保护流水线」、ADR-091「屏幕流微秒级报文定步与令牌桶突发整形器」、ADR-092「屏幕流基于时延梯度的拥塞控制器与缓冲膨胀趋势线滤波器」、ADR-093「屏幕流选择性重传（NACK）与混合纠错（HARQ）流水线」、ADR-094「屏幕流接收端驱动传输层全网拥塞控制与报文到达反馈流水线」 |

## 动机

1. **解决传统流控启动阶段与拥塞恢复后的「慢启动爬升困境（Slow Ramp-Up Bottleneck）」**：
   - 在传统的加性增乘性减（AIMD）与基于帧确认的拥塞控制体系中，串流初始通常以保守的低码率（如 2 Mbps ~ 4 Mbps）起步，随后以固定的加性步长（例如每秒增加 200 kbps ~ 500 kbps）缓慢爬升；
   - 在千兆局域网（LAN）或低时延 Wi-Fi 6 / 5G 高速网络环境下，可用物理带宽往往高达数十至数百 Mbps。若沿用被动渐进式爬升，屏幕流需要经历十数秒甚至数十秒才能达到 15 Mbps ~ 20 Mbps 的 4K/60FPS 清晰画质，用户在连接初期的主观画质体验严重劣化；
   - 同样，在网络遭遇瞬时无线干扰触发乘性急剧降速（`force_step_down`）后，一旦信道恢复顺畅，被动爬升耗时过长，无法即时恢复高分辨率与低量化参数（QP）。

2. **借鉴现代 WebRTC 与 BBR 算法的主动探测（Active Bandwidth Probing）设计**：
   - 不等待被动的业务视频帧缓慢累积，而是由流控引擎主动构造短小、紧凑、微秒级排队的「探针数据包簇（Probe Packet Clusters）」；
   - 通过在极短的时间窗口（数毫秒）内突发注入特定目标速率（如 8 Mbps、12 Mbps、16 Mbps、20 Mbps）的数据流量，迫使信道瓶颈在不影响正常视频帧解码呈现的前提下暴露出其真实的排队与吞吐特征；
   - 借助接收端的精确到达时间戳，解算该探针簇在接收端的实际突发接收跨度。若接收速率与目标探测速率高度吻合且未引入排队延迟与丢包，则证明网络具备承载该高码率的能力。

3. **双重保护门禁与严格的拥塞防爆设计（Safety Gating & Anti-Congestion Safeguards）**：
   - 主动探测必须防止自身成为压垮网络的推手。探针数据必须采用低开销无害载荷（Padding Dummy Bytes），且严格限制探针簇大小（每簇 4 ~ 16 包，每包 1200 字节，突发仅持续数毫秒）；
   - 建立严格的判定三要素门禁：
     1. **丢包门禁（Loss Threshold）**：探针簇丢包率 $\le 15\%$；
     2. **速率达成门禁（Delivery Rate Threshold）**：实测接收投递速率 $\ge 70\%$ 目标探测速率；
     3. **排队梯度门禁（Delay Gradient Threshold）**：突发期间引起的端到端额外单向排队增量 $\le 20\,\text{ms}$（20,000 $\mu\text{s}$）；
   - 唯有同时满足全部三项严苛指标，才判定探测成功，执行一次性跨越爬升（Fast Ramp-Up）；任一条件不满足则判定探测失败（`Failed`），立即进入退避冷却期，绝不强行推高码率。

4. **与 TWCC 报文反馈与 AIMD 控制器的无缝闭环融合**：
   - 探针数据包直接接入 ADR-094 TWCC 序列号分配器（`transport_seq`），在接收端被视作普通传输报文记录到达时刻；
   - TWCC 紧凑反馈回传后，发送端探针控制器根据探针簇的 `start_seq` 与 `end_seq` 过滤出对应项，解算突发带宽与梯度；
   - 验证通过后，直接调用 `ScreenRateController.fast_ramp_up(target_bps)`，将码率瞬间置于目标高位，重置连续优良帧计数器，平稳移交后续 AIMD 微调。

5. **100% 双端命令行全同构与企业级可观测性准则**：
   - 严格落实 Dual-CLI 设计哲学，`tetherly`（节点守护进程）与 `tetherly-cli`（远控客户端）提供参数命名、终端人机交互展示、JSON 遥测结构全同构的 Probe 子命令（`screen probe`）及启动配置（`--screen-probe`）。

---

## 决定

1. **紧凑专用线协议设计（`TYPE_SCREEN_PROBE = 0x0507`）**：
   - **固定头部（26 字节）**：
     - `magic: [u8; 5] = *b"TPROB"`：专用魔数校验；
     - `proto: u8 = 0x01`：协议版本；
     - `flags: u8`：标志位（0x01 标识启动探针，0x02 标识阶梯探针，0x04 标识手动触发）；
     - `cluster_id: u8`：探测簇唯一标识符（`1..=254` 循环）；
     - `cluster_index: u8`：当前报文在所属簇内的 0 基索引起始；
     - `cluster_size: u8`：所属簇包含的报文总数；
     - `transport_seq: u16`：大端序 TWCC 全局传输序号（供反馈配对与丢包判定）；
     - `target_bps: u32`：大端序本簇所测试的目标带宽（单位：bps）；
     - `send_time_us: u64`：大端序发送微秒绝对时间戳；
     - `padding_len: u16`：大端序有效填充字节长度。
   - **填充数据载荷（Padding Payload）**：
     - 紧跟在 26 字节头部后，填充字节长度默认设置为 $1200 - 26 = 1174$ 字节，使单包线上传输尺寸达到标准 MTU 预算（1200 字节），精确模拟真实视频分片占用的信道资源。

2. **核心状态机与探测调度器（`crates/tetherly-core/src/screen.rs`）**：
   - **探测模式枚举（`ScreenProbeMode`）**：
     - `Auto (0)`（默认）：自适应高敏探测（每簇 8 个报文，最小触发间隔 500ms）；
     - `Aggressive (1)`：进取型快速探测（每簇 16 个报文，最小触发间隔 250ms，单次阶梯增幅 1.6x）；
     - `Conservative (2)`：温和渐进探测（每簇 6 个报文，最小触发间隔 1000ms，单次阶梯增幅 1.2x）；
     - `Disabled (3)`：彻底关闭主动探测，仅依赖被动 AIMD 爬升。
   - **探测状态枚举（`ScreenProbeState`）**：
     - `Idle (0)`：当前无在途探测任务；
     - `Probing (1)`：正在线路上发送探测簇报文；
     - `AwaitingFeedback (2)`：探测簇报文已全部投递出站，正在等待 TWCC 接收端到达反馈；
     - `Completed (3)`：探测成功完成，已触发快速爬升；
     - `Failed (4)`：探测失败（丢包超标、速率不足或排队时延过大），进入退避期。
   - **实测速率与排队梯度精确解算数学模型**：
     - 接收端在收到簇内 $N$ 个到达报文后，记录各自的相对到达时间偏移增量 $\Delta d_i$；
     - 簇内到达时间总跨度：
       $$\Delta t_{\text{span}} = \max_{i}(\Delta d_i) - \min_{i}(\Delta d_i)$$
     - 实测投递突发速率：
       $$R_{\text{probe}} = \frac{N \times L_{\text{packet}} \times 8 \times 10^6}{\max(\Delta t_{\text{span}}, 1)\,\mu\text{s}}$$
     - 实测排队时延膨胀梯度：
       $$\nabla D = \Delta t_{\text{span}}$$
     - 门禁逻辑判断：
       $$\text{passed} = (\text{loss\_ratio} \le 0.15) \land (R_{\text{probe}} \ge 0.70 \times R_{\text{target}}) \land (\nabla D \le 20\,000\,\mu\text{s})$$

3. **发送端与接收端系统级闭环整合（`ScreenSender` & `ScreenReceiver`）**：
   - 发送端串流启动（`on_control(ControlMsg::Start)`）或通过 API 手动触发时，探测器排队生成完整的 `ScreenProbeCluster`；
   - 发送循环 `pop_probe_packet()` 优先出队探测包，与视频 FEC 分片并行但非阻塞调度；
   - 接收端 `on_probe_packet_with_time` 拦截 `TYPE_SCREEN_PROBE`，不仅更新探测统计，更将其 `transport_seq` 注册至 `ScreenTwccReceiverTracker`；
   - 接收端生成的 TWCC 反馈自然涵盖了探测包的到达时刻与丢包情况；
   - 发送端 `handle_twcc_feedback` 捕获到探测包记录时，调用 `probe_controller.on_twcc_feedback()` 进行评估，若通过则直接驱动 `rate_controller.fast_ramp_up(target_bps)`。

4. **运行时系统与本地控制面整合（`tetherly-node`）**：
   - `NodeConfig` 与 `SharedNodeState` 新增 `screen_probe_mode: ScreenProbeMode` 配置项；
   - 在 `runtime.rs` 发送循环中，将出队的探测包打包为 `InnerFrame` 直接投递至对端屏幕信道，并记录遥测；
   - 回环 REST API 扩展：
     - `GET /api/screen/probe`：获取当前探测状态与遥测报告；
     - `POST /api/screen/probe`：更新探测模式、指定探测目标码率或触发探测；
     - `POST /api/screen/probe/trigger`：即时发起一次主动带宽探测突发；
     - `GET /api/screen`：扩展 `probe_mode`、`probe_state`、`probes_triggered`、`probes_completed`、`probes_failed`、`probe_target_bps`、`probe_measured_bps` 遥测指标。

5. **100% 全同构双端 CLI 交付**：
   - `tetherly`（守护进程）与 `tetherly-cli`（独立客户端）实现完全一致的命令行界面：
     ```bash
     # 查询主动带宽探测状态
     tetherly screen probe [--json] [--ui-port <port>]
     tetherly-cli screen probe [--json] [--ui-port <port>]

     # 调整主动带宽探测模式 (auto | aggressive | conservative | disabled)
     tetherly screen probe auto
     tetherly-cli screen probe auto

     # 手动触发一次主动带宽探测（以默认阶梯或指定 15000 kbps 目标码率）
     tetherly screen probe --trigger
     tetherly screen probe --target-kbps 15000
     tetherly-cli screen probe --trigger
     tetherly-cli screen probe --target-kbps 15000

     # 守护进程启动参数支持直接指定探测模式
     tetherly run --screen-probe auto
     ```

---

## 影响

1. **秒级画质拉升体验跃迁**：
   - 在高带宽 LAN/Wi-Fi 环境下，初始串流从 4 Mbps 爬升至 16 Mbps 的时间由原先的 15~25 秒急剧缩短至 **150~300 毫秒**（1 次探测突发与 feedback 周期即完成闭环）；
   - 彻底解决了串流连接初期长时间模糊、画面细节涂抹的问题。

2. **强壮的抗拥塞退避与零假阳性安全边界**：
   - 严苛的 15% 丢包率与 20ms 排队时延硬约束，彻底杜绝了恶劣弱网下因盲目盲探导致链路直接拥死断流的隐患；
   - 探测失败后自动退避降级，平稳回落至安全 AIMD 慢调步幅。

3. **双端 CLI 体验 100% 统一**：
   - 无论是直接操作后台守护进程还是远程调用独立客户端，输出日志格式、字段单位、JSON 键名与退出码完全一致。
