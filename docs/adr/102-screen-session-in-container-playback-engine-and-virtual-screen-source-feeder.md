# ADR-102 屏幕串流会话容器回放引擎与虚拟屏幕源馈送流水线（Screen Session In-Container Playback Engine & Virtual Screen Source Feeder Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-27，v2.16 / v0.1.38）**——基于 ADR-101 TREC 容器格式建立屏幕会话动态回放引擎与虚拟屏幕源（`ScreenSource`）双模馈送流水线：(1) **回放引擎与状态机**（`ScreenPlaybackEngine`、`ScreenPlaybackState`、`ScreenPlaybackReport`）：支持对 `.trec` 容器做带时间戳定步（PTS Pacing）的逐帧回放与向 `ScreenSink` 呈现，支持 `speed` 变速倍率（0.5x、1.0x、2.0x、4.0x 或 0.0x CI 极速免等待）、`loop` 循环回放、`pause` 暂停、`resume` 恢复与 `stop` 终止；(2) **虚拟屏幕源馈送器**（`ScreenRecordingSource`）：实现标准 `ScreenSource` trait，将 `.trec` 录制容器无缝桥接为网络发端抓屏源，在无需物理显示器/GPU 抓屏的环境下以录制数据驱动远程串流管线（用于远程示教、CI 回放测试与基准压测）；(3) **节点运行时集成**：`NodeShared` 挂载 `screen_playback` 异步会话与状态机，提供 `screen_playback_start`、`screen_playback_pause`、`screen_playback_resume`、`screen_playback_stop` 与 `screen_playback_report`；(4) **回环 REST API**：`POST /api/screen/playback/start`、`POST /api/screen/playback/pause`、`POST /api/screen/playback/resume`、`POST /api/screen/playback/stop` 与 `GET /api/screen/playback`；`ScreenStatusReport` 扩充 `playback_active`、`playback_state`、`playback_frame` 遥测；(5) **双端 CLI 100% 全同构**：`screen playback start <path> [--speed] [--loop]`、`pause`、`resume`、`stop`、`status` 与 `--json`；(6) **测试覆盖**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_26_screen_session_playback_engine_and_virtual_source_feeder` 全链路验证。 |
| 日期 | 2026-10-27 |
| 关联 | ADR-077「屏幕流链路吞吐压测与带宽画像」、ADR-087「屏幕串流接收端动态抖动缓冲与平滑重放流水线」、ADR-097「屏幕串流多显示器枚举与虚拟显示器流复用」、ADR-101「屏幕串流会话录制容器与回放流水线」 |

---

## 动机

1. **录制成果闭环与回放呈现缺失**：ADR-101 实现了 TREC 自研轻量级容器与首帧自推导落盘，访客端能够稳定录制全流程会话；但录制产出的 `.trec` 文件此前仅支持静态只读校验与摘要抓取（`inspect / replay`），无法动态播放到接收端屏幕或系统画面（`ScreenSink`）上供人工回看、审计与交互复盘。
2. **虚拟屏幕源（Virtual Screen Source）需求**：在 CI 自动化回归测试、无头服务器（Headless Linux）、云手机/容器、算法基准评测以及远程协助示教演示中，通常没有可用的物理显卡或真实显示器供 DXGI/X11/Quartz 捕获。通过将录制好的 `.trec` 包装成标准 `ScreenSource`（虚拟屏幕捕获源），屏幕串流发端即可直接向远端访客广播高质量的会话录像，达成生产环境与测试环境的高保真同构。
3. **灵活的回放控制与时钟定步**：回放过程不仅需要按照原始采集呈现时间戳（PTS）保真回放（1.0x 真实速度），还必须支持倍速播放（如 2.0x 快速概览、0.5x 慢放查证）、0.0x CI 极速免休眠模式、循环播放（`loop`）、动态暂停（`pause`）与继续（`resume`）。
4. **双端 CLI 100% 同构与控制面闭环**：依据 Tetherly 一贯的设计契约，所有回放操作与状态透视必须通过 127.0.0.1 安全回环 REST API 提供，并在 `tetherly` 与 `tetherly-cli` 命令行工具中保持参数、行为与 `--json` 格式的 100% 严格同构。

---

## 决定

### 1. 回放状态机与数据模型（`crates/tetherly-core/src/screen.rs`）

- **`ScreenPlaybackState` 枚举**：
  - `Idle`（空闲就绪）
  - `Playing`（正在按定步播放）
  - `Paused`（暂停保持当前进度）
  - `Finished`（全量帧播放完毕，自然结束）
  - `Stopped`（用户主动终止）
- **`ScreenPlaybackReport` 结构体**：
  - `ok: bool`
  - `state: String`（"idle" | "playing" | "paused" | "finished" | "stopped"）
  - `path: String`（回放源容器路径）
  - `speed: f32`（回放倍率，> 0.0 时按 pts 差值 / speed 定步，== 0.0 时全速推流免休眠）
  - `loop_enabled: bool`（循环回放标志）
  - `current_frame: u64`（当前已呈现帧序号计数）
  - `total_frames: u64`（容器总帧数）
  - `elapsed_ms: u64`（已回放相对时间毫秒）
  - `duration_ms: u64`（容器总录制时长毫秒）
  - `width: u16` / `height: u16` / `format: String` / `codec: String`（容器头规格）

### 2. 回放引擎（`ScreenPlaybackEngine`）

- 负责从给定 `ScreenRecordingReader` 迭代读取并解码 `ScreenFrame`；
- 维护 PTS 基线与时钟定步（`std::thread::sleep` 或微秒定时）：
  - 针对两帧间 PTS 增量 $\Delta t$，若 `speed > 0.0`，则等待 $\Delta t / speed$；
  - 若 `speed == 0.0`，跳过等待直接投递，保证单元测试与 CI 不受微秒定时器抖动与性能拖累；
- 支持 `step(&mut self, sink: &dyn ScreenSink)` 单步投递与 `tick()` 循环投递；
- 当读取到容器 EOF 且 `loop_enabled == true` 时，自动 rewind 文件并重置 PTS 基线，实现无缝连续回放。

### 3. 虚拟屏幕源（`ScreenRecordingSource`）

- 实现 `crates/tetherly-core/src/ports.rs` 中定义的 `ScreenSource` trait：
  - `fn grab(&self) -> Result<Option<ScreenFrame>, CoreError>`：按配置的 FPS 与 speed 从 `.trec` 中取出下一帧；
  - `fn displays(&self) -> Vec<ScreenDisplayInfo>`：根据容器头分辨率返回单主虚拟显示器；
  - `fn set_fps(&self, fps: u32)`：动态调整拉取步频；
  - 内部具备并发线程安全锁（`Mutex<ScreenRecordingReader>`），当开启 `loop` 模式时，流末尾自动重置至首帧，充当持续可用的虚拟视频发生器。

### 4. 节点运行时与会话管理（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 新增 `screen_playback: Mutex<Option<ScreenPlaybackSession>>`；
- 提供方法：
  - `screen_playback_start(path, speed, loop_enabled)`：检查路径安全性（拒绝 `..` 父目录穿越），打开 `.trec` 文件验证 magic/proto，启动后台异步播放任务并向 `node.shared.screen_sink` 与 `node.shared.screen_observer` 呈现帧；
  - `screen_playback_pause()`：原子置为暂停态；
  - `screen_playback_resume()`：恢复播放态；
  - `screen_playback_stop()`：终止后台任务并复位状态；
  - `screen_playback_report()`：读取当前进度快照；
- 扩展 `ScreenStatusReport`，注入 `playback_active: bool`、`playback_state: String`、`playback_frame: u64`。

### 5. 回环 REST API（`crates/tetherly-node/src/uihttp.rs`）

在 `127.0.0.1` 环回端口提供（受 `Host` 请求头守卫）：
- `POST /api/screen/playback/start`（body / query: `path`, 可选 `speed`, 可选 `loop`；参数缺失或文件非法返回 400）；
- `POST /api/screen/playback/pause`；
- `POST /api/screen/playback/resume`；
- `POST /api/screen/playback/stop`；
- `GET /api/screen/playback`（返回 `ScreenPlaybackReport`）。

### 6. 双端 CLI 100% 全同构（`bins/tetherly-cli/src/main.rs`, `crates/tetherly-node/src/bin/tetherly.rs`）

全同构支持：
```bash
# 启动会话回放（支持变速倍率与循环回放）
tetherly screen playback start /path/to/session.trec --speed 1.5 --loop
# 暂停回放
tetherly screen playback pause
# 恢复回放
tetherly screen playback resume
# 停止回放
tetherly screen playback stop
# 查看回放状态
tetherly screen playback
tetherly screen playback status
```
在 `screen status` 命令输出中新增 `Playback:` 状态行。

---

## 结论与工程影响

- **全流程闭环**：Tetherly 屏幕串流体系实现了从「编码传输 → 实时呈现 → 瓶颈分析 → 自主调优 → 完整录制 → 动态回放/虚拟源馈送」的完备生命周期闭环；
- **虚拟源赋能**：无需物理显卡与驱动支持，即可在纯服务器/CI 容器中借由录制文件驱动完整的端到端串流与性能测试；
- **零新依赖与严格安全**：所有逻辑基于纯 Rust 自研容器及零拷贝解帧实现，严格防范文件路径穿越。
