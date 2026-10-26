# ADR-096 屏幕与音频播放音画同步与硬件时钟漂移补偿流水线（Screen & Audio Playout Lip-Sync & Clock Synchronization Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-26，v2.10 / v0.1.32）**——实现面向超低时延实时屏幕与音频串流的音画唇形同步（Lip-Sync）与晶振硬件时钟漂移（Clock Drift PPM）补偿流水线（`ScreenAudioSyncMode`、`ScreenAudioSyncFrame`、`ScreenAudioSyncCoordinator`、`ScreenAudioSyncStats`、`ScreenAudioSyncReport`）；在传输层设计专用同步信标报文（`TYPE_SCREEN_SYNC = 0x0508`，`TSYNC` 协议魔数，36 字节定长报头，携带同步信标序列号、发送端发送微秒墙上时间戳、最新视频帧微秒显示时间戳 Video PTS 与最新音频块微秒显示时间戳 Audio PTS）；采用 RFC 3550 RTCP 发送端报告（Sender Report）映射锚点模型，接收端计算音视频呈现时间偏差（$\Delta t_{\text{skew}} = \text{PTS}_{\text{video}} - \text{PTS}_{\text{audio}}$），并基于滑窗信标到达间隔比对解算发送端与接收端物理晶振偏差 PPM；当时间偏斜超出模式容限（Auto: $\pm 15\,\text{ms}$、Strict: $\pm 5\,\text{ms}$、Loose: $\pm 40\,\text{ms}$）时，自适应调节视频去抖动播放缓冲区（`ScreenPlayoutBuffer`）的目标播放延迟（步进限制在 $\pm 20\,\text{ms}$ 平滑滤波窗口内），实现画质无撕裂、音频不爆音的微秒级无缝唇音对齐；全面集成至 `ScreenSender`、`ScreenReceiver`、`tetherly-node` 守护进程与回环 REST API（`GET/POST /api/screen/sync` 与 `POST /api/screen/sync/trigger`）；实现 100% 全同构双端 CLI（`tetherly screen sync`、`tetherly-cli screen sync`、`--screen-sync` 启动参数及 `screen status` 状态遥测）；新增核心单元测试、节点级测试与端到端自动化集成测试 `m5_20_screen_audio_sync_and_clock_drift_compensation` |
| 日期 | 2026-10-26 |
| 关联 | ADR-066「跨设备媒体播放控制与状态同步」、ADR-072「跨设备系统音频路由」、ADR-079「屏幕流有损编码协商」、ADR-087「屏幕流去抖动播放缓冲区与平滑呈现流水线」、ADR-094「屏幕流接收端驱动传输层全网拥塞控制与报文到达反馈流水线」、ADR-095「屏幕流带宽主动探测与链路容量快速爬升流水线」 |

## 动机

1. **解决音视频独立捕获编码链路导致的「音画不同步（Lip-Sync Playout Divergence）」问题**：
   - 在低时延远程控制与跨设备投屏协同场景中，屏幕视频采集编码（OS GDI/Desktop Duplication、H.264/ZSTD）与系统音频采集编码（Loopback/WASAPI/PipeWire、Opus/PCM 音频分块）通常运行在彼此独立的系统线程与硬件管道中；
   - 视频帧体积较大（数 KB 到数十 KB），经过切片、FEC 保护、令牌桶定步与网络传输后，在接收端由去抖动缓冲区（`ScreenPlayoutBuffer`）进行平滑重组，引入了 15ms ~ 50ms 的动态目标呈现延迟；
   - 与此同时，音频数据块体积轻巧（数百字节）、发包频率高，往往通过独立通道以更短的排队延迟抵达。接收端若缺乏统一的呈现基准对齐机制，随着网络抖动和处理耗时波动，画面往往明显滞后于声音数十甚至上百毫秒，造成剧烈的人声对不上嘴唇（Lip-Sync 失步）、视听感官撕裂与操作脱节。

2. **补偿异构设备间物理晶体振荡器的微小频率偏差（Hardware Clock Drift PPM）**：
   - 即使发送端与接收端均采用高精度微秒级单调时钟，由于硬件晶振制造公差、环境温度与电源稳定性差异，不同计算机的石英晶振频率天然存在 $10 \sim 100\,\text{ppm}$（Parts Per Million）的固有偏差；
   - 举例而言，$50\,\text{ppm}$ 的硬件时钟偏差意味着每小时将产生约 $180\,\text{ms}$ 的时钟偏移累积。在数十小时的长时连线协同中，如果不同步晶振漂移，接收端播放缓冲区要么逐渐被撑爆溢出，要么面临周期性枯竭饥饿（Buffer Starvation/Underrun），导致严重的视音频卡顿与突发爆音。

3. **借鉴 RTP/RTCP RFC 3550 时间同步映射模型（Wall-clock Anchor Mapping）**：
   - 工业级音视频流媒体协议通过定期发送 RTCP Sender Report（SR），建立音视频呈现时间戳（PTS）与发送端绝对物理墙上时间戳（NTP/Wall-Clock）的映射关系；
   - Tetherly 需要在保证零外部依赖、极低 CPU/网络开销的前提下，引入轻量级微秒精度时钟同步信标（`ScreenAudioSyncFrame`），使接收端能够动态对齐音画时间轴，实时解算双向时钟漂移 PPM。

4. **播放缓冲区自适应微调（Adaptive Playout Buffer Adjustment）**：
   - 调整音视频同步不能通过粗暴的音频重采样硬截断（容易引发高频电击杂音与音调变异），而应利用接收端现存的弹性去抖动缓冲区（ADR-087 `ScreenPlayoutBuffer`）；
   - 当检测到视频相对音频滞后（$\Delta t_{\text{skew}} > 0$）或音频相对视频滞后（$\Delta t_{\text{skew}} < 0$）超出容限阈值时，自适应调节视频帧目标播放延迟 $D_{\text{target}}$，通过渐进微步进对齐呈现时间，平稳消除失步。

5. **100% 严格双端命令行同构与企业级遥测支持（Dual-CLI Parity）**：
   - 秉持 Dual-CLI 架构准则，`tetherly`（节点守护进程）与 `tetherly-cli`（远控客户端）提供参数命名、终端人机交互展示、JSON 遥测结构全同构的 Sync 子命令（`screen sync`）及启动配置（`--screen-sync`）。

---

## 决定

1. **紧凑专用线协议设计（`TYPE_SCREEN_SYNC = 0x0508`）**：
   - **固定头部（36 字节）**：
     - `magic: [u8; 5] = *b"TSYNC"`：5 字节专用协议魔数；
     - `proto: u8 = 0x01`：协议版本号；
     - `flags: u8`：标志位（0x01 标识关键帧对齐，0x02 标识手动触发，0x04 标识时钟基准重置）；
     - `reserved: u8 = 0`：保留对齐字节；
     - `sync_seq: u32`：大端序信标单调递增序列号；
     - `sender_wallclock_us: u64`：大端序发送端微秒绝对物理时间戳；
     - `video_pts_us: u64`：大端序发送端最新采集/编码的视频帧微秒显示时间戳（Video PTS）；
     - `audio_pts_us: u64`：大端序发送端最新采集/编码的音频块微秒显示时间戳（Audio PTS）。

2. **核心状态机与同步协调器（`crates/tetherly-core/src/screen.rs`）**：
   - **同步工作模式枚举（`ScreenAudioSyncMode`）**：
     - `Auto (0)`（默认）：自适应平滑同步（偏斜容限 $\pm 15\,\text{ms}$，信标周期 500ms）；
     - `Strict (1)`：超低时延严格同步（偏斜容限 $\pm 5\,\text{ms}$，信标周期 250ms）；
     - `Loose (2)`：高抖动宽松同步（偏斜容限 $\pm 40\,\text{ms}$，信标周期 1000ms）；
     - `Disabled (3)`：禁用同步信标生成与播放补偿。
   - **偏斜解算与判定公式**：
     $$\Delta t_{\text{skew}} = \text{PTS}_{\text{video}} - \text{PTS}_{\text{audio}}$$
     - 若 $\Delta t_{\text{skew}} > \tau_{\text{tolerance}}$：视频落后音频，需要调小视频播放延迟或加速呈现；
     - 若 $\Delta t_{\text{skew}} < -\tau_{\text{tolerance}}$：视频超前音频，需要增加视频播放延迟平滑等待音频；
     - 调整量阻尼滤波：
       $$\Delta D = \text{clamp}\left(\frac{\Delta t_{\text{skew}}}{2},\, -20\,\text{ms},\, +20\,\text{ms}\right)$$
   - **硬件时钟漂移（Clock Drift PPM）解算**：
     - 基于滑窗统计连续两个信标之间的发送端间隔 $\Delta T_{\text{sender}}$ 与接收端物理间隔 $\Delta T_{\text{receiver}}$：
       $$\text{PPM} = \frac{\Delta T_{\text{receiver}} - \Delta T_{\text{sender}}}{\Delta T_{\text{sender}}} \times 10^6$$
     - 采用一阶指数移动平均（EMA，$\alpha = 0.1$）滤波消除瞬时网络抖动影响。

3. **全链路运行时整合（`crates/tetherly-node/src/runtime.rs`）**：
   - 音频数据链路在 `receive_audio_routing_chunk` 中将音频块时间戳 `frame.timestamp_ms * 1000` 实时注入 `ScreenSender::record_audio_pts`；
   - 视频发包循环在 `send_screen_frames` 中记录最新视频 PTS，按周期生成 `TYPE_SCREEN_SYNC` 信标随视频流发送；
   - 接收端在 `attach_screen_client` 中截获 `TYPE_SCREEN_SYNC` 报文，交由 `ScreenReceiver::on_sync_beacon_with_time` 评估偏斜、计算 PPM 并平滑调用 `ScreenPlayoutBuffer::adjust_target_delay`；
   - 对外暴露 `screen_sync_mode()`、`screen_set_sync_mode()`、`screen_sync_report()` 与 `trigger_screen_sync()`。

4. **双端命令行与 REST 接口同构（Dual-CLI & UI HTTP API）**：
   - HTTP 接口：`GET /api/screen/sync`、`POST /api/screen/sync`、`POST /api/screen/sync/trigger`；
   - CLI 命令：`tetherly screen sync` 与 `tetherly-cli screen sync`；
   - 启动参数：`--screen-sync <auto|strict|loose|disabled>`；
   - 遥测展示：在 `screen status` 中输出 `Sync: auto (sent: 4, recv: 4, skew: +1.20 ms, drift: +12.5 ppm, adj: 1, in-sync: true)`。

---

## 验证与效果

1. **核心单元测试覆盖**：
   - `screen_audio_sync_pipeline_and_drift_compensation`：测试信标帧编码解码、容限边界判定、PPM 计算、延迟自适应补偿与 PeerGone 状态清理。
2. **节点与 HTTP API 测试覆盖**：
   - `test_uihttp_screen_sync_endpoints`：验证 GET/POST `/api/screen/sync` 查询、模式切换、手动信标触发及 Host 403 防护。
3. **端到端集成测试**：
   - `m5_20_screen_audio_sync_and_clock_drift_compensation`：双节点配对，验证全流程动态模式变更、音视频信标生成分发、呈现偏斜补偿与遥测闭环。
