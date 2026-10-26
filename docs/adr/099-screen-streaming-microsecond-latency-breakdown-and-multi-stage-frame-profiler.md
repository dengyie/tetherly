# ADR-099 屏幕串流微秒级端到端延迟拆解、多阶段逐帧分析器与性能遥测流水线（Screen Streaming Microsecond End-to-End Latency Breakdown, Multi-Stage Frame Profiler & Performance Telemetry Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-27，v2.13 / v0.1.35）**——建立屏幕串流远程控制系统微秒级全链路 7 阶段端到端延迟拆解、逐帧剖析分析器（`ScreenFrameProfiler`）与性能指标统计流式管线；在线协议层引入专用剖析信封 `TYPE_SCREEN_PROFILE = 0x0509`（56 字节定长二进制协议头：`magic(5: TPROF)`、`proto(1: 0x01)`、`flags(2)`、`frame_seq(8)`、`capture_us(4)`、`encode_us(4)`、`pacer_us(4)`、`sender_ts_us(8)`、`network_us(4)`、`jitter_us(4)`、`decode_us(4)`、`render_us(4)`、`total_e2e_us(4)`）；在屏幕控制通道引入专用剖析重置信令 `ControlMsg::ResetLatencyProfile`（Envelope Tag = 6，14 字节定长线协议包头）；实现 7 阶段高精度耗时解耦（采集 `capture`、编码 `encode`、起搏 `pacer`、网络传输 `network`、抖动缓冲 `jitter`、解码 `decode`、渲染呈现 `render`）；建立主导瓶颈阶段自适应判定算法（Dominant Bottleneck Stage，单阶段耗时占比 $\ge 35\%$ 时自动识别判定）；建立 EWMA 指数移动加权平均（$\alpha = 0.15$）与 1024 样本环形缓冲区分位数统计（P50、P90、P95、P99、Min、Max）；全面集成至 `ScreenSender`、`ScreenReceiver`、`tetherly-node` 守护进程与回环 REST API（`GET /api/screen/latency`、`POST /api/screen/latency`、`POST /api/screen/latency/reset`）；实现 100% 严格全同构双端 CLI（`tetherly screen latency`、`tetherly-cli screen latency` 及其 `status` / `set` / `reset` 子命令与 `--mode`、`--reset`、`--json` 参数）；新增核心单元测试、节点级测试与端到端自动化集成测试 `m5_23_screen_microsecond_latency_breakdown_and_multi_stage_profiler_pipeline` |
| 日期 | 2026-10-27 |
| 关联 | ADR-079「屏幕流有损编码协商」、ADR-086「屏幕流4D全自主协同自适应流水线」、ADR-087「屏幕串流接收端动态抖动缓冲与平滑重放流水线」、ADR-091「屏幕串流发端数据包起搏与突发平滑整形流水线」、ADR-092「屏幕串流基于延迟梯度的双端 Trendline 拥塞探测与排队延迟检测」、ADR-094「屏幕串流传输级全带宽拥塞控制反馈流水线（TWCC）」、ADR-096「音视频时钟同步与唇音同步流水线」 |

---

## 动机

1. **实时远程控制场景下的延迟盲区与归因困境**：
   - 在远程桌面、跨屏协同与云端图形工作站场景中，“端到端交互延迟（End-to-End Latency）”是衡量用户体验的最核心生命线指标。
   - 过去系统仅能粗略观测双向往返时延（RTT）或发端帧间隔。当用户感知到鼠标粘滞、拖影或画面卡顿延缓时，工程师与自动控制算法无法区分延迟瓶颈究竟发生在宿主机操作系统抓屏（Capture）、OpenH264/Zstd 编码压缩（Encode）、网络队列丢包排队（Network）、接收端抖动平滑缓冲（Jitter Buffer）、帧解码（Decode）还是显卡窗口上屏呈现（Render）。
   - 缺少微秒级阶段级归因，导致 4D 自适应调度矩阵（ADR-086）、起搏整形（ADR-091）与 TWCC 拥塞控制（ADR-094）只能做黑盒调控，无法实施精准的靶向优化。

2. **微秒级（Microsecond）全链路时间戳同步与跨端剖析需求**：
   - 7 个子阶段横跨发送端操作系统内核、用户态守护进程、传输网络以及接收端解码渲染管线；
   - 必须设计紧凑、低开销的专用二进制线协议（Wire Protocol Profile Frame），使接收端完成解包、渲染后能够将该帧的完整生命周期阶段数据精准聚合，并能够通过控制通道实时反馈回发端。

3. **双端 CLI 同构与现代化自动化运维要求**：
   - 依据 Tetherly 的核心工程规范，所有核心控制能力必须在本地回环 REST API、SSE 遥测事件总线以及 `tetherly` / `tetherly-cli` 双端命令行中保持 100% 字节级同构，供自动化脚本、CI 流水线和桌面 GUI 统一无缝调用。

---

## 决定

### 1. 7 阶段微秒级端到端延迟拆解模型

定义每一帧从产生到最终呈现在用户屏幕上的全生命周期 7 阶段微秒耗时：
1. **`capture_us`（OS 抓屏捕获）**：从操作系统桌面子系统（DXGI Desktop Duplication、X11 SHM、CoreGraphics Display Stream 等）触发抓屏到获取未压缩显存/内存原始帧的耗时；
2. **`encode_us`（编码压缩）**：输入原始帧送入压缩引擎（Zstd 快速压缩或 OpenH264 视频帧硬件/软件编码）至输出压缩完成数据块的耗时；
3. **`pacer_us`（发端出站起搏排队）**：分片帧在发送端漏桶/令牌桶起搏整形器（ScreenPacketPacer，ADR-091）队列中等待平滑发送的时长；
4. **`network_us`（单向网络线上传输）**：数据包从发端物理网卡输出到接收端物理网卡接收的时长（通过时钟漂移补偿时钟或对半 RTT 平滑估算）；
5. **`jitter_us`（接收端抖动缓冲重放等待）**：数据包进入接收端自适应抖动缓冲区（ScreenPlayoutBuffer，ADR-087）到由于重放定时器触发弹出的等待排队时长；
6. **`decode_us`（接收端解压解码）**：从抖动缓冲区取出压缩帧并送入解码器还原为 RGBA/BGRA 像素位图的运算耗时；
7. **`render_us`（显示呈现）**：解码后的像素位图送入最终窗口图形接收槽（BitBlt / XPutImage / Metal / SDL2）并完成垂直同步刷新的时长。

总时延公式严格保证可加性：
$$\text{total\_e2e\_us} = \text{capture\_us} + \text{encode\_us} + \text{pacer\_us} + \text{network\_us} + \text{jitter\_us} + \text{decode\_us} + \text{render\_us}$$

---

### 2. 线协议设计：56 字节定长剖析信封（`TYPE_SCREEN_PROFILE = 0x0509`）

在既有屏幕串流信令家族中新增专用线协议信封 `TYPE_SCREEN_PROFILE = 0x0509`：
- **`magic: [u8; 5] = *b"TPROF"`**：5 字节协议魔数；
- **`proto: u8 = 0x01`**：1 字节协议版本；
- **`flags: u16`**：2 字节大端序标志位（保留拓展）；
- **`frame_seq: u64`**：8 字节大端序视频帧单调序列号；
- **`capture_us: u32`**：4 字节抓屏耗时；
- **`encode_us: u32`**：4 字节编码耗时；
- **`pacer_us: u32`**：4 字节起搏耗时；
- **`sender_ts_us: u64`**：8 字节发端发送绝对时间戳；
- **`network_us: u32`**：4 字节网络传输耗时；
- **`jitter_us: u32`**：4 字节抖动缓冲耗时；
- **`decode_us: u32`**：4 字节解码耗时；
- **`render_us: u32`**：4 字节渲染呈现耗时；
- **`total_e2e_us: u32`**：4 字节全链路总耗时。

合计定长 56 字节（`SCREEN_PROFILE_HEADER_LEN = 56`），在网络带宽占用上不足 1 个小数据包，对网络无侵入性负载。

---

### 3. 控制通道重置信令（`ControlMsg::ResetLatencyProfile`）

在 14 字节定长屏幕控制信令信封（`TYPE_SCREEN_CONTROL = 0x0502`）中引入 Tag 6：
- **Tag = 6**：`ControlMsg::ResetLatencyProfile`
- 当发端或接收端触发性能遥测重置时，通过控制通道向对端广播，双方原子清空 1024 样本环形缓冲区、EWMA 平滑状态与分位数统计，实现状态同步。

---

### 4. 主导瓶颈自适应判定算法（Dominant Bottleneck Identification）

引入 `ScreenBottleneckStage` 枚举：
`Capture`、`Encode`、`Pacer`、`Network`、`Jitter`、`Decode`、`Render` 与 `Balanced`。
- **阈值判定准则**：在任一统计窗口内，计算各阶段在 EWMA 平滑耗时及当前帧瞬时耗时中所占比例；
- 若某一阶段耗时占总时延比例 $\ge 35\%$，则判定该阶段为当前系统的**主导性能瓶颈**；
- 若所有阶段耗时占比均 $< 35\%$，则判定系统处于**均衡状态（`Balanced`）**。

---

### 5. 统计聚合与分位数分布（Statistics & Percentiles）

- **EWMA 平滑更新**：采用衰减因子 $\alpha = 0.15$ 维护各阶段的指数加权移动平均；
- **1024 样本环形缓冲区**：维护最近 1024 帧的 `total_e2e_us` 样本列表，计算：
  - **P50（中位数）**
  - **P90**
  - **P95**
  - **P99**
  - **Min / Max**

---

### 6. REST API 设计

在 `tetherly-node` 本地回环接口（默认 `127.0.0.1:45716`）提供：
- **`GET /api/screen/latency`**：获取当前屏幕串流延迟剖析报告（`ScreenLatencyReport`）；
- **`POST /api/screen/latency`**：动态配置剖析模式（`auto`、`detailed`、`disabled`）与可选触发重置（`{"mode": "detailed", "reset": false}`）；
- **`POST /api/screen/latency/reset`**：原子清空延迟样本与统计历史。

---

### 7. 双端 CLI 100% 同构（`tetherly` & `tetherly-cli`）

双端 CLI 完全同构支持：
```bash
# 查询当前微秒级端到端延迟拆解报告、阶段分布及分位数
tetherly screen latency
tetherly screen latency status

# 配置剖析模式
tetherly screen latency set detailed
tetherly screen latency --mode auto

# 重置延迟统计指标
tetherly screen latency reset
tetherly screen latency --reset

# 输出机器可读 JSON
tetherly screen latency --json
```

人机友好的终端输出样例：
```
Screen End-to-End Microsecond Latency Breakdown & Multi-Stage Profiler (ADR-099):
  Profiler Mode:        Auto
  Dominant Bottleneck:  Network
  Frames Profiled:      1024
  Latest Frame Seq:     #4096
  Stage Breakdown (Latest Frame):
    Capture (OS):         2.15 ms (   2150 us)
    Encode (Codec):       4.80 ms (   4800 us)
    Pacer (Outbound):     0.45 ms (    450 us)
    Network (Transit):   15.20 ms (  15200 us)
    Jitter (Playout):     3.10 ms (   3100 us)
    Decode (Receiver):    1.85 ms (   1850 us)
    Render (Display):     0.90 ms (    900 us)
    Total End-to-End:    28.45 ms (  28450 us)
  Stage Distribution (%):
    Capture:               7.6%
    Encode:               16.9%
    Pacer:                 1.6%
    Network:              53.4% [BOTTLENECK]
    Jitter:               10.9%
    Decode:                6.5%
    Render:                3.2%
  Latency Distribution (Total E2E):
    EWMA (Smoothed):     28.10 ms (  28100 us)
    P50 (Median):        26.50 ms (  26500 us)
    P90:                 31.20 ms (  31200 us)
    P95:                 34.80 ms (  34800 us)
    P99:                 42.00 ms (  42000 us)
```

---

## 结论与工程影响

- **全链路微秒归因**：彻底消除桌面远程控制系统的延迟“黑盒”，为算法自主调控（4D 矩阵自适应、FEC 冗余调节、起搏速率整形）提供直接的数据支撑；
- **极简高性能开销**：56 字节定长信封与无锁/原子环形缓冲区，对高帧率（60fps/120fps）传输零瓶颈；
- **100% 同构与健全测试**：双端 CLI、回环 HTTP API、单元测试及全链路集成测试 `m5_23` 全面落地通过。
