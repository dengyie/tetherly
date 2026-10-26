# ADR-100 屏幕串流瓶颈驱动的自主调优闭环流水线（Screen Streaming Bottleneck-Driven Autonomous Tuning Closed Loop）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-27，v2.14 / v0.1.36）**——在 ADR-099 微秒级 7 阶段延迟剖析的基础上建立瓶颈驱动的自主调优闭环：引入 `ScreenTuner` 瓶颈归因自主调优器（迟滞触发：Standard 连续 3 帧 / Aggressive 连续 2 帧同型瓶颈；冷却窗：Standard 2s / Aggressive 1s；反振荡恢复：Standard 连续 10 帧 / Aggressive 连续 6 帧均衡后才逐级回升）；定义 7 类靶向降级动作（`ReduceFps`、`ReduceScale`、`ReduceBitrate`、`RaiseQp`、`RaisePlayoutDelay`）与 5 类阶梯恢复动作（`RecoverBitrate`、`RecoverFps`、`RecoverScale`、`RecoverQp`、`LowerPlayoutDelay`），恢复栈严格按降级逆序逐级回升；引入调优模式 `ScreenTunerMode`（`Disabled` / `Standard` / `Aggressive`）；新增屏幕控制信令 `ControlMsg::SetTuningMode`（Envelope Tag = 7）与 `ControlMsg::ResetTuningProfile`（Tag = 8），实现双端调优模式与统计重置的跨端信令协同；全面集成至 `ScreenSender`（码率/FPS/尺度/QP 四维执行器）与 `ScreenReceiver`（播放缓冲延迟执行器）、`tetherly-node` 守护进程与回环 REST API（`GET /api/screen/tuning`、`POST /api/screen/tuning`、`POST /api/screen/tuning/reset`）；实现 100% 严格全同构双端 CLI（`tetherly screen tuning`、`tetherly-cli screen tuning` 及其 `status` / `set` / `reset` 子命令与 `--mode`、`--reset`、`--json` 参数）；新增核心单元测试、节点级 REST 测试与端到端自动化集成测试 `m5_24_screen_bottleneck_driven_autonomous_tuning_closed_loop` |
| 日期 | 2026-10-27 |
| 关联 | ADR-082「屏幕流动态码率自适应」、ADR-083「屏幕流动态帧率与自适应起搏」、ADR-084「屏幕流动态分辨率缩放」、ADR-086「屏幕流4D全自主协同自适应流水线」、ADR-087「屏幕串流接收端动态抖动缓冲与平滑重放流水线」、ADR-091「屏幕串流发端数据包起搏与突发平滑整形流水线」、ADR-094「屏幕串流传输级全带宽拥塞控制反馈流水线（TWCC）」、ADR-099「屏幕串流微秒级端到端延迟拆解与多阶段逐帧分析器」 |

---

## 动机

1. **剖析归因与自主执行的断层**：ADR-099 建立了微秒级 7 阶段延迟拆解与主导瓶颈自动识别（`Capture` / `Encode` / `Pacer` / `Network` / `Jitter` / `Decode` / `Render` / `Balanced`），但识别结果目前只用于观测——工程师或外部脚本读到 "Network 53.4% [BOTTLENECK]" 后仍需手动调参。系统缺少「归因 → 靶向执行 → 验证 → 回升」的全自主闭环。
2. **黑盒调控的误伤问题**：既有 4D 矩阵（ADR-086）与 TWCC（ADR-094）依据 RTT/丢包/带宽估计做黑盒调控，无法区分「编码器太慢」与「链路太堵」：编码瓶颈时降码率无效（画面依旧卡在 encode），网络瓶颈时降尺度收益低（ packets 变小但排队依旧）。只有按瓶颈阶段靶向执行才能真正缓解：编码瓶颈 → 抬 QP / 降尺度（减少编码工作量）；网络与起搏瓶颈 → 降码率；抖动瓶颈 → 加深播放缓冲；采集/解码/呈现瓶颈 → 降 FPS / 降尺度。
3. **迟滞与反振荡**：自主调优最大的工程风险是「降级-恢复」振荡（乒乓效应）：必须以连续 N 帧同型瓶颈为触发门槛（迟滞），加冷却窗限频，且恢复必须比降级更保守（更长均衡观测窗 + 逐级逆序回升），保证系统单调收敛到稳定工作点。
4. **双向信令协同与双端 CLI 100% 同构**：调优模式必须在双端（发端四维执行器 + 收端播放缓冲执行器）保持一致，并可通过控制通道跨端设置与重置；依据 Tetherly 核心工程规范，全部能力在本地回环 REST API、SSE 事件总线与 `tetherly` / `tetherly-cli` 双端命令行中保持 100% 字节级同构。

---

## 决定

### 1. 调优模式（`ScreenTunerMode`）

| 模式 | 触发门槛（连续同型瓶颈帧数） | 冷却窗 | 恢复门槛（连续均衡帧数） |
|---|---|---|---|
| `Disabled = 0` | 永不触发 | — | 永不恢复 |
| `Standard = 1`（默认） | 3 帧 | 2,000,000 µs | 10 帧 |
| `Aggressive = 2` | 2 帧 | 1,000,000 µs | 6 帧 |

### 2. 瓶颈 → 靶向动作映射（降级路径）

| 主导瓶颈阶段 | 执行器侧 | 动作 | 参数 |
|---|---|---|---|
| `Capture` | 发端 | `ReduceFps` | 沿 FPS 阶梯下降一级（`screen_matrix_next_lower_fps`） |
| `Encode` | 发端 | `RaiseQp` | `qp_max += 4`（上限 `SCREEN_QP_ABSOLUTE_MAX`）；已到顶则 `ReduceScale` |
| `Pacer` | 发端 | `ReduceBitrate` | `bitrate × (100−15)%`（下限 `SCREEN_BITRATE_MIN_BPS`） |
| `Network` | 发端 | `ReduceBitrate` | 同上 |
| `Jitter` | 收端 | `RaisePlayoutDelay` | 播放缓冲目标延迟 `+5 ms`（`adjust_target_delay`，钳位既有上下界） |
| `Decode` | 发端 | `ReduceScale` | 沿尺度阶梯下降一级；已到下限则 `ReduceFps` |
| `Render` | 发端 | `ReduceFps` | 沿 FPS 阶梯下降一级；已到下限则 `ReduceScale` |

收端调优器仅执行播放缓冲动作（`Jitter`），其余阶段由发端调优器基于同一条剖析数据执行；双端各自独立决策，互不重复执行。

### 3. 阶梯恢复路径（反振荡）

- 仅当连续 `recovery_need` 帧判定为 `Balanced` 且恢复栈非空时，**每次弹出一项最近降级动作**执行逆操作：
  - `RecoverBitrate`：`bitrate × (100+12)%`（上限 `SCREEN_BITRATE_MAX_BPS`）；
  - `RecoverFps`：沿 FPS 阶梯上升一级（`screen_matrix_next_higher_fps`）；
  - `RecoverScale`：沿尺度阶梯上升一级（`screen_matrix_next_higher_scale`）；
  - `RecoverQp`：`qp_max −= 2`（下限 `qp_min`）；
  - `LowerPlayoutDelay`：播放缓冲目标延迟 `−5 ms`。
- 每次实际执行（降级或恢复）都刷新冷却窗；未执行动作的评估不刷新。

### 4. 控制信令（`TYPE_SCREEN_CONTROL = 0x0502`，14 字节定长）

- **Tag = 7 `ControlMsg::SetTuningMode { mode: u8 }`**：低 8 位承载 `ScreenTunerMode` tag；任一端经 REST/CLI 设置调优模式后向对端广播，对端同步应用到本地执行器；
- **Tag = 8 `ControlMsg::ResetTuningProfile`**：重置信令，双端原子清空调优计数、迟滞窗口与恢复栈（调优模式本身保留）。

### 5. 遥测与 DTO

- `ScreenStats` 扩展 `tuning_mode`、`tuning_actions_total`、`tuning_recoveries`、`tuning_last_stage`、`tuning_last_action`；
- `ScreenTuningDecision`：单次决策快照（帧序号、瓶颈阶段、动作、各维新值、是否恢复、时间戳）；
- `ScreenTuningReport`：REST/CLI 报告 DTO（模式、累计动作数、累计恢复数、最近阶段/动作、当前迟滞窗口计数、最近决策）。

### 6. REST API 设计

在 `tetherly-node` 本地回环接口（默认 `127.0.0.1:45716`）提供：
- **`GET /api/screen/tuning`**：获取当前瓶颈驱动自主调优报告（`ScreenTuningReport`）；
- **`POST /api/screen/tuning`**：动态配置调优模式（`disabled`、`standard`、`aggressive`）与可选触发重置（`{"mode": "aggressive", "reset": false}`）；设置模式时若存在已连接屏幕会话则向对端发送 `SetTuningMode`；
- **`POST /api/screen/tuning/reset`**：清空本地调优统计与恢复栈；若存在会话则向对端发送 `ResetTuningProfile`。

### 7. 双端 CLI 100% 同构（`tetherly` & `tetherly-cli`）

```bash
# 查询当前调优报告
tetherly screen tuning
tetherly screen tuning status

# 配置调优模式
tetherly screen tuning set aggressive
tetherly screen tuning --mode standard

# 重置调优统计与恢复栈
tetherly screen tuning reset
tetherly screen tuning --reset

# 机器可读 JSON
tetherly screen tuning --json
```

---

## 结论与工程影响

- **归因即执行**：ADR-099 的瓶颈识别首次直接驱动参数执行，消除「观测-决策-执行」断层，形成完整感知-决策-执行-验证闭环；
- **迟滞收敛保证**：触发门槛 + 冷却窗 + 更保守的恢复门槛三重反振荡设计，保证调优过程单调收敛、不乒乓；
- **零新增带宽负载**：调优完全复用 ADR-099 既有剖析数据流，仅新增两条 14 字节控制信令，对高帧率传输零侵入；
- **100% 同构与健全测试**：双端 CLI、回环 HTTP API、核心单元测试与端到端集成测试 `m5_24` 全面落地通过。
