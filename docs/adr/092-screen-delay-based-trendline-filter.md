# ADR-092 屏幕流基于时延梯度的拥塞控制器与缓冲膨胀趋势线滤波器（Screen Streaming Delay-Based Congestion Controller & Early Bufferbloat Trendline Filter）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v2.06）**——实现基于时延梯度的拥塞控制器与早期缓冲膨胀（Bufferbloat）趋势线滤波器（`ScreenTrendlineFilter`、`ScreenTrendlineStats`、`ScreenTrendlineReport`、`ScreenTrendlineMode`、`ScreenBandwidthUsageState`）；消除传统基于丢包拥塞控制对网络中间节点大队列积压的反应迟钝缺陷；采用最小二乘法（OLS）线性回归实时估计队列延迟变化斜率（$\mu s/\text{frame}$）；支持 Auto（自适应 RTT 缩放阈值 $\gamma$）、Aggressive（低时延敏感阈值 $6.0 \mu s/\text{frame}$）、Conservative（抗无线噪波阈值 $25.0 \mu s/\text{frame}$）与 Disabled 四种运行模式；在连续 3 帧判定队列积压（Overusing）或清空（Underusing）时触发状态机跃迁，向发射端自适应矩阵输出主动降速预警；深度集成至 `ScreenSender` 帧确认驱动流水线、`tetherly-node` 运行时与回环 REST API（`GET/POST /api/screen/trendline`）；实现 100% 全同构双端 CLI（`tetherly screen trendline`、`tetherly-cli screen trendline`、`--screen-trendline` 启动参数与 `screen status` 状态遥测）；新增核心单元测试与端到端自动化集成测试 `m5_16_screen_delay_based_trendline_filter` |
| 日期 | 2026-10-25 |
| 关联 | ADR-080「双向帧确认与自适应拥塞背压」、ADR-082「屏幕流动态码率自适应控制」、ADR-083「屏幕流动态帧率与自适应定步步频控制器」、ADR-084「屏幕流动态分辨率缩放与自适应下采样控制器」、ADR-086「屏幕流 4 维自主自适应协同调度器」、ADR-087「屏幕流接收端播放抖动缓冲区与自适应平滑控制器」、ADR-090「屏幕流前向纠错（FEC）与自适应冗余保护流水线」、ADR-091「屏幕流微秒级报文定步与令牌桶突发整形器」 |

## 动机

1. **突破传统基于丢包拥塞控制（Loss-Based Congestion Control）的滞后性瓶颈**：
   - 现存的屏幕流自适应调度器（如 4D 矩阵协调器 ADR-086、动态码率控制 ADR-082）主要依赖帧丢弃事件（Drop）或往返时延（RTT）绝对值的剧烈升高来触发下调；
   - 在现代家用 Wi-Fi 路由器、4G/5G 基站及长途宽带链路上，中间交换机通常配置有超大容量的数据包缓冲区（Bufferbloat 缓冲膨胀）；
   - 当屏幕发射码率轻微超出瓶颈带宽时，报文不会立即丢失，而是在中间队列中排队积压；随着队列逐渐累积，端到端单向时延会单调爬升 100ms ~ 500ms 以上；
   - 当网络最终发生丢包（Tail-Drop）时，网络管道内已经积压了数帧至数十帧过期的陈旧视频切片，此时再进行降速已经无法挽回用户端数十帧的严重卡顿与操控迟滞。

2. **引入前瞻性时延梯度估计与趋势线滤波（RFC 8888 / GCC Delay Gradient）**：
   - 借鉴 RFC 8888 及 Google Congestion Control (GCC) 延迟驱动思想，通过比较连续帧的发送时间间隔（Inter-departure time delta $\Delta s_i$）与接收时间间隔（Inter-arrival time delta $\Delta a_i$），计算单帧延迟抖动变动量：
     $$v_i = \Delta a_i - \Delta s_i$$
   - 对滑动窗口内的累积队列时延进行普通最小二乘法（Ordinary Least Squares, OLS）线性回归分析，实时求解斜率 $m$（$\mu s / \text{frame}$）；
   - 若斜率持续为正且超过动态阈值 $\gamma$，表明网络中间瓶颈队列正在持续膨胀积压，系统在**尚未发生任何单包丢弃之前**即可提前预警（Overusing），主动调减发射码率以抽干网络排队积压；反之若斜率持续为负且低于 $-\gamma$，表明排队已在消解（Underusing），链路可进入安全升速试探。

3. **应对复杂信道噪声与抖动（Adaptive Thresholding $\gamma$）**：
   - 在跨国公网或弱网 Wi-Fi 下，偶然的网络拥塞或基站重传可能导致个别帧瞬时时延跳变，若采用固定阈值极易误判；
   - 必须引入自适应阈值机制（Auto Mode）：动态结合当前链路的平滑 RTT（RTT p95），在长 RTT 高时延链路上自适应放宽容忍度，在局域网超低时延链路上收紧容忍度；
   - 同时支持对时延敏感的竞技级模式（Aggressive，阈值 $6.0 \mu s/\text{frame}$）以及对抖动抗干扰模式（Conservative，阈值 $25.0 \mu s/\text{frame}$），满足多样化的串流场景需求。

4. **100% 双端命令行全同构与全栈可观测性准则**：
   - 遵照 Tetherly 架构规范，全部流控能力必须在全功能节点 CLI（`tetherly`）与轻量远控端（`tetherly-cli`）保持全同构的子命令命名、参数体系、状态查询及 JSON 遥测结构。

---

## 决定

1. **核心数学模型与数据结构（`crates/tetherly-core/src/screen.rs`）**：
   - **趋势线模式枚举（`ScreenTrendlineMode`）**：
     - `Auto (0)`（默认）：自适应动态阈值 $\gamma = 12.5 \cdot \text{clamp}(\frac{\text{RTT}_{\text{p95}}}{100\text{ms}}, 0.5, 3.0)$；
     - `Aggressive (1)`：固定极低阈值 $\gamma = 6.0 \mu s/\text{frame}$，适合零缓冲膨胀交互场景；
     - `Conservative (2)`：固定抗噪阈值 $\gamma = 25.0 \mu s/\text{frame}$，适合高噪声无线网络；
     - `Disabled (3)`：透传旁路，始终输出 Normal。
   - **网络带宽占用状态枚举（`ScreenBandwidthUsageState`）**：
     - `Normal (0)`：链路排队平稳，延迟梯度处于 $[-\gamma, \gamma]$ 区间；
     - `Overusing (1)`：瓶颈队列持续积压，梯度连续 $\ge 3$ 帧超过 $\gamma$；
     - `Underusing (2)`：瓶颈队列正在排空，梯度连续 $\ge 3$ 帧低于 $-\gamma$。
   - **遥测状态与报表 DTO（`ScreenTrendlineStats` / `ScreenTrendlineReport`）**：
     ```rust
     #[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
     pub struct ScreenTrendlineStats {
         pub mode: ScreenTrendlineMode,
         pub state: ScreenBandwidthUsageState,
         pub current_slope_us_per_frame: f64,
         pub threshold_us_per_frame: f64,
         pub overuse_count: u64,
         pub underuse_count: u64,
         pub accumulated_delay_us: i64,
     }

     #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
     pub struct ScreenTrendlineReport {
         pub ok: bool,
         pub mode: ScreenTrendlineMode,
         pub state: ScreenBandwidthUsageState,
         pub samples_count: usize,
         pub current_slope_us_per_frame: f64,
         pub threshold_us_per_frame: f64,
         pub overuse_count: u64,
         pub underuse_count: u64,
         pub accumulated_delay_us: i64,
         pub timestamp_ms: u64,
     }
     ```

2. **普通最小二乘法（OLS）线性回归算法实现**：
   - 滑动窗口维护 $W=20$（最大容量 60）个样本点对 $(s_i, y_i)$，其中 $s_i$ 为发射微秒时间戳相对偏移量，$y_i = \sum_{k=1}^i (\Delta a_k - \Delta s_k)$ 为累计排队时延偏移量；
   - 线性回归斜率求解：
     $$\bar{x} = \frac{1}{N}\sum x_i, \quad \bar{y} = \frac{1}{N}\sum y_i$$
     $$\text{Cov}(x, y) = \sum (x_i - \bar{x})(y_i - \bar{y}), \quad \text{Var}(x) = \sum (x_i - \bar{x})^2$$
     $$\text{raw\_slope} = \frac{\text{Cov}(x, y)}{\text{Var}(x)}$$
     $$\text{slope\_us\_per\_frame} = \text{raw\_slope} \times \Delta t_{\text{avg\_frame}}$$
   - 防抖状态机：当计算出的每帧斜率超出当前阈值 $\pm \gamma$ 时，引入 3 帧防抖计数器（Debounce Window $\ge 3$），唯有在趋势稳定确认后方才进行状态跃迁，并在状态机中累计 `overuse_count` 和 `underuse_count`。

3. **双向确认驱动与发射端流控联动（`ScreenSender`）**：
   - 接收端回传 `ControlMsg::Ack { seq }` 携带远端到达时间戳时，在 `ScreenSender::on_control_with_time` 触发 `trendline_filter.on_ack()`；
   - 发射端实时更新 `ScreenStats` 中的 `trendline_mode`、`trendline_state`、`trendline_slope_us` 与 `trendline_overuse_count` 字段；
   - 当状态进入 `Overusing` 时，直接触发主动自适应降速逻辑，实现零丢包提前抽干网络队列。

4. **守护进程运行时与回环 REST API 集成（`tetherly-node`）**：
   - `NodeConfig` 与 `SharedNodeState` 新增 `screen_trendline_mode: ScreenTrendlineMode`，默认在全局初始化为 `Auto`；
   - 暴露标准异步接口 `screen_trendline_report()`、`screen_trendline_mode()`、`screen_set_trendline_mode(mode)`；
   - 回环 HTTP UI 注册端点：
     - `GET /api/screen/trendline` -> 返回完整 `ScreenTrendlineReport` JSON 遥测报表；
     - `POST /api/screen/trendline` -> 支持请求体 `{"mode": "..."}` 或 URL 查询参数 `?mode=...` 动态变更模式。

5. **100% 双端命令行全同构（`tetherly` & `tetherly-cli`）**：
   - 节点服务命令：`tetherly run --screen-trendline <auto|aggressive|conservative|disabled>`；
   - 节点交互命令：`tetherly screen trendline [MODE] [--json] [--ui-port <PORT>]`；
   - 远控交互命令：`tetherly-cli screen trendline [MODE] [--json] [--ui-port <PORT>]`；
   - 状态综合命令：`tetherly screen status` 与 `tetherly-cli screen status` 均在通道概览中打印当前 Trendline 模式、状态、斜率与超限告警统计。

---

## 验证与效果

1. **单元测试与数值回归**：
   - `screen_trendline_filter_bufferbloat_detection_and_modes` 验证了：
     - 恒定网络时延下，回归斜率稳定在 $0.0 \pm 1.0 \mu s/\text{frame}$，状态严格保持 `Normal`；
     - 人工模拟排队积压（$+200 \mu s/\text{frame}$）连续注入时，状态机准确于第 3 帧判定跃迁至 `Overusing`，`overuse_count` 正确递增；
     - 模拟队列消解（$-200 \mu s/\text{frame}$）时，状态机及时切入 `Underusing`；
     - `Auto` 模式下阈值随 RTT 在 $6.25 \sim 25.0 \mu s/\text{frame}$ 范围弹性缩放；
   - `test_uihttp_screen_trendline_endpoints` 与 `test_screen_trendline_mode_and_reporting` 验证了 REST API 与运行时交互的正确性。

2. **端到端集成测试**：
   - `m5_16_screen_delay_based_trendline_filter` 在双节点实机配对握手与虚拟显示驱动流水线中全面通过，验证了帧发射、状态查询、动态切换模式与综合遥测的无缝协同。
