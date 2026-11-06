# ADR-138: 远程桌面控制（Remote Desktop Control）开发设计规范

状态: 规划（开发文档，未实施）
版本: v0.1.90 规划 / v2.60
日期: 2026-11-07

---

## 0. 摘要与开发范围（Scope）

在既有屏幕流管线（ADR-079~131）与输入隧道（ADR-136）之上，补齐最后一公里：
**查看端主动发起会话 → 在查看窗口内获得对端桌面的完整键鼠控制权**（向日葵 / RustDesk 语义），
区别于 ADR-133/136 的"物理光标越边漫游（Deskflow 语义）"。

**核心差异（本 ADR 的全部设计动因）**：

| 维度 | Deskflow 漫游（既有） | 远程桌面控制（本 ADR 新增） |
|---|---|---|
| 触发 | 物理光标推过共享边 | 查看端显式发起会话（UI/API/托盘） |
| 视觉 | 无（直接操作对端物理屏） | 查看端**窗口内呈现**对端屏幕流 |
| 坐标系 | 归一化跨屏映射（SCREEN_UNITS=1000） | **窗口视口 → 对端物理像素**的点对点映射 |
| 事件源 | 本机物理键鼠全局钩子 | 查看窗口的 AppKit/Win32 事件回调 |
| 通道 | TCP 45719 input wire | **复用 45719 wire 协议**，但以会话信令进入 Session 模式 |
| 并存性 | 全局单 controller 不变式 | 受 ADR-136 接管治理约束：进入 Session 模式先 polite takeover |

**MVP 边界（Phase 1）**：Mac 控 Windows（生产主场景）。
Windows 查看窗与 Windows 控 Mac 列入 Phase 2。

---

## 1. 背景与现状盘点（What Already Exists）

实现本功能**不需要新建任何协议层**，以下设施全部现成（v0.1.89 已上线）：

### 1.1 屏幕流（查看端"看"）
- 抓屏：Windows `DxgiSource`（DXGI Desktop Duplication，`crates/tetherly-node/src/screen.rs:927`）、macOS CoreGraphics（`screen/macos.rs`）、Linux X11 shm；
- 编码：`ScreenCodec` 可插拔 — OpenH264 H.264（ADR-079）/ zstd delta / BGRA raw；
- 传输：TCP 45720，`TYPE_SCREEN_FRAME 0x0501` + 拥塞控制/静默/FEC/NACK/Tile 全家桶（ADR-080~126）；
- 会话控制面：`/api/screen/{allow,revoke,start,stop,...}`（`uihttp/api_screen_session.rs`），`ControlMsg::{Start,Stop,...}` 经 `TYPE_SCREEN_CONTROL 0x0502` 双向同步；
- 端到端测试：phase5 55 个管线测试（`ControllableDeltaScreenSource` 注入 + `screen_sink_snapshot()` 断言）。

### 1.2 输入（查看端"控"）
- wire 协议：`InputEvent { seq, kind }`，`InputKind::{Enter,Leave,Move,Button,Wheel,Key,ClipHint}`（`tetherly-core/src/input.rs:43`），帧类型 `0x0401~0x0407`；
- 受控端注入：`InputSink` trait（`move_abs/button/wheel/key`，core/input.rs:339）→ Windows `SendInput` 绝对坐标（platform.rs:357/891）→ `DualInputSink`（runtime/input.rs:153）路由 real sink；
- 会话状态机：`InputClient<S>`（core/input.rs:877）— `focused` 门控、`release_all_inputs()` 卡键保护、seq 单调去重；
- 键码映射：`KeymapProfile::mac_to_windows()`（core/keymap.rs:136）+ `MAC_TO_WIN_KEYS` 表（platform.rs:927）。

### 1.3 会话信令与治理
- `InputTunnelRequest/Resp/Release/Heartbeat` 帧（`0x0211~0x0214`，core/input_tunnel.rs:31）；
- 治理策略 `InputTunnelPolicy`（store.rs:1514）：`allow_inbound_control / allow_keyboard / allow_mouse / idle_timeout_secs / keymap`；
- ADR-136 polite takeover：controller 侧 `receive_input_tunnel_request` 先 `input_stop()` 释放；方向守卫防双 controller；
- REST：`/api/input/tunnel/{attach,release}`、`/api/input/policy`、`/api/input/ensure`（TCC 检查）。

### 1.4 缺失的三块（本 ADR 要写的）
1. **查看窗口的输入捕获**：`tetherly-viewer` 只读 stdin 呈现（macOS-only，`bin/tetherly_viewer.rs:194`），无鼠标键盘事件捕获与回传；
2. **Session 模式的注入路由**：受控端 `InputClient` 目前只被"越边漫游"的 Enter/Leave 驱动聚焦，需要会话信令显式置 `focused=true`（Session 模式无边界概念）；
3. **编排层**：把"开流 + 建会话 + 起查看器 + 策略检查"串成一个原子操作。

---

## 2. 架构设计（Proposed Architecture）

### 2.1 总览

```
┌─ Mac（Viewer/Controller）───────────────┐      ┌─ Windows（Host/Target）─────────┐
│                                          │      │                                 │
│  tetherly-viewer 窗口                    │      │  DxgiSource 抓屏                │
│   ├─ NSView 呈现 BGRA/H264 解码帧        │      │   └─ ScreenEngine 编码          │
│   ├─ NSEvent 捕获 → InputEvent 映射      │      │                                 │
│   │    (视口→物理像素, keymap 重映射)    │      │  InputServer (45719)            │
│   └─ stdout/stdin 或新控制通道 ──────────┼──┐   │   └─ InputClient(focused=true)  │
│                                          │  │   │        └─ DualInputSink         │
│  daemon: RemoteSessionController         │  │   │             └─ SendInput        │
│   ├─ session_start(): takeover + 开流    │  └───┼─▶ TCP 45720 (screen wire)      │
│   ├─ 转发 viewer 输入事件 → input_out    │      │                                 │
│   └─ 心跳/空闲超时/窗口关闭清理          │      │  治理: InputTunnelPolicy 检查    │
└──────────────────────────────────────────┘      └─────────────────────────────────┘
```

**关键决策：viewer 与 daemon 的进程边界保持现状**（AppKit 主线程限制决定了 viewer 必须独立进程，见 tetherly_viewer.rs 模块注释），输入事件经 viewer 进程 → daemon 的本地 IPC 回传，再复用既有 `send_input_events()`（runtime/input.rs:2309）出站。

### 2.2 帧协议扩展（tetherly-core）

新增 3 个控制帧（复用 ControlMsg 通道 45717 会话，非新端口）：

```rust
// crates/tetherly-core/src/frame.rs
pub const TYPE_REMOTE_SESSION_REQUEST:  u16 = 0x0229; // viewer→host: 请求会话
pub const TYPE_REMOTE_SESSION_RESP:     u16 = 0x022A; // host→viewer: 接受/拒绝
pub const TYPE_REMOTE_SESSION_CLOSE:    u16 = 0x022B; // 双向: 会话结束

#[derive(Serialize, Deserialize)]
pub struct RemoteSessionRequestFrame {
    pub session_id: String,          // 复用 InputTunnelValidator::validate_session_id
    pub client_device_id: String,
    pub reason: RemoteSessionReason, // UserInitiated | ApiTriggered
    pub timestamp_ms: u64,
}

#[derive(Serialize, Deserialize)]
pub struct RemoteSessionRespFrame {
    pub session_id: String,
    pub accepted: bool,
    pub reject_reason: Option<String>, // PolicyDisabled | ScreenBusy | AllowRequired | Busy
    pub screen_width: u32,             // host 物理分辨率（viewer 视口映射基准）
    pub screen_height: u32,
    pub keymap_profile: Option<String>, // host 建议的 keymap（"mac_to_windows"）
    pub timestamp_ms: u64,
}
```

**为什么不复用 `InputTunnelRequest`**：tunnel 请求语义是"我要开始漫游驱动你的物理桌面"（进入 CursorSeat::Remote、触发 takeover），而远程会话请求语义是"我要开一个窗口看并控制你"，需要附带屏幕分辨率协商且允许被 `allow_inbound_control=false` 但屏幕流开放的机器拒绝输入部分。两者共用底层治理但不共用信令，避免语义耦合。

### 2.3 会话生命周期状态机（tetherly-core 新增 `remote_session.rs`）

```
Idle ──request──▶ Requesting ──accept──▶ Streaming ──input_enabled──▶ Controlling
                     │                      │                            │
                     └─reject──▶ Idle       └──close/error──▶ Idle ◀──close/error──┘
```

- `Streaming`：屏幕流已通（复用 `open_screen` + `/api/screen/start` 流程），输入未开；
- `Controlling`：输入隧道进入 Session 模式。唯一出口是 `Close`（用户关窗 / idle 超时 / 对端 release）；
- 状态持久化仅内存（`Shared.remote_session: Mutex<Option<RemoteSessionState>>`），崩溃即清零，无恢复语义（会话本就是瞬态的）。

### 2.4 受控端：Session 模式注入（tetherly-node/runtime/input.rs 扩展）

**现状**：`InputClient.focused` 只在收到 `Enter`（越边）时置 true。Session 模式没有边界，
需要一个显式的聚焦信令。

**方案**：扩展 `InputEngine` 增加 `enter_session(entry: (i32,i32))` / `exit_session()`：

```rust
// Session 模式 = 单次永久 Enter：seat → Remote + focused=true + 光标置于 host 中心
// exit_session() = release_all_inputs() + seat → Local（复用既有清理）
```

`receive_input_event` 分派逻辑不变（`InputClient.on_frame` 已有的 focused 门控、
seq 去重、release_all_inputs 全部复用），只是 `Enter` 帧的来源从"边界越境"变为
"会话建立"。**`InputEvent::to_frame()`/`from_frame` wire 格式零改动。**

受控端权限闸（在 `receive_remote_session_request` 处检查，全部现成字段）：
1. `InputTunnelPolicy.allow_inbound_control`（键盘鼠标总闸）；
2. `allow_keyboard` / `allow_mouse` 细分：被拒的类别事件在 `DualInputSink` 前丢弃；
3. 屏幕流闸：host 处于 `ScreenState::Idle` 且从未 allow → 自动置 `Allowed`（或按策略拒绝 `AllowRequired`）；
4. **保护进程红线**：注入路径只调 `SendInput`，不 EnumWindows/不 SetWindowsHookEx 全局钩子/不碰其他进程——与漫游模式同一注入面，零新增风险。

### 2.5 查看端：viewer 进程升级（tetherly-node/bin/tetherly_viewer.rs）

**stdin 帧流保持不变**（BGRA 呈现管线零改动），新增三条本地 IPC：

```
viewer ──(新 stdout 通道: 事件行 JSONL)──▶ daemon ──▶ send_input_events()
daemon ──(新 stdin 通道: 控制行 JSONL)───▶ viewer （分辨率/会话状态/关闭指令）
```

事件映射（viewer 内 `NSView` 覆写）：

| NSEvent | InputEvent | 映射规则 |
|---|---|---|
| `mouseMoved/LeftMouseDragged` | `Move{x,y}` | `view_point / view_size * SCREEN_UNITS`，**host 回报物理分辨率**，受控端 `move_abs` 已按物理像素解释（platform.rs:357 的 0..65535 归一在 sink 内部完成） |
| `leftMouseDown/Up` | `Button{BTN_LEFT, down}` | mask 复用 core/input.rs 既有 BTN_* 常量 |
| `rightMouseDown/Up` | `Button{BTN_RIGHT, down}` | 同上 |
| `scrollWheel` | `Wheel{dx,dy}` | delta 累积到 ±1 步进，`wheel_speed` 策略照常生效 |
| `keyDown/keyUp` | `Key{code, down, mods}` | CGKeyCode → `MAC_TO_WIN_KEYS` → host VK；mods 走 `KeymapProfile` 修饰符替换 |

**新增约束**：
- `NSTrackingArea` 全视口 + `acceptsFirstResponder=YES` 才能收键；
- 捕获时 `CGAssociateMouseAndMouseCursorPosition(false)` 隐藏本机光标（Esc 退出，复用 `RawInput::EmergencyEscape` 语义）；
- H.264 模式下 viewer 增加解码（openh264 decoder 已在依赖树中）——**Phase 1 简化**：viewer 会话强制走 BGRA/zstd（`/api/screen/start` 时 host 按 codec 协商降档），H.264 直通列入 Phase 2。

### 2.6 编排层：`RemoteSessionController`（tetherly-node/runtime/remote_session.rs 新模块）

```rust
pub async fn remote_session_start(&self, peer: &DeviceId) -> Result<RemoteSessionReport, CoreError> {
    // 1. 治理：本机若是 controller 先 input_tunnel_release（polite，复用 ADR-136）
    // 2. 信令：RemoteSessionRequest → Resp（拿 host 分辨率 + keymap）
    // 3. 开流：open_screen(peer) + /api/screen/start（既有路径）
    // 4. 聚焦：input 通道发 Enter 帧（Session 模式）
    // 5. 起 viewer：spawn tetherly-viewer，stdin/stdin IPC 接线
    // 6. 心跳：复用 TYPE_INPUT_TUNNEL_HEARTBEAT（idle_timeout_secs 沿用 InputTunnelPolicy）
}
```

REST（`uihttp/api_remote_session.rs` 新增，loopback-only）：
- `POST /api/remote/session/start {peer}` → 报告（session_id、viewer PID、分辨率）
- `POST /api/remote/session/stop` → 优雅关闭（关窗 + release + 停流）
- `GET  /api/remote/session` → 状态（复用状态机序列化）

托盘入口（platform/tray.rs 两平台各 +1 菜单项）：「🖥 远程控制…」→ 列出 live peers → 选中即调 `start`。

### 2.7 与漫游模式的互斥（ADR-136 不变式保持）

全局仍至多一条出站输入通道。`RemoteSession` 是第三种 role（`InputTunnelRole` 增补
`SessionController` / `SessionTarget`），约束：
- 进入 Session 前若 `active_tunnel_role() == Some(Controller)` → 先 polite release；
- Session 活跃期间，`link_watcher` 自动 attach 与推边接管全部跳过（方向守卫扩展一个分支）；
- 对端物理键鼠**不锁定**（与向日葵"被控端可见被操作"一致；lock-to-screen 语义列为 Phase 2 可选项，复用 ADR-136 §5 已知限制条目）。

---

## 3. 关键文件清单（Critical Files）

| 文件 | 动作 | 内容 |
|---|---|---|
| `crates/tetherly-core/src/frame.rs` | 改 | +3 个 `TYPE_REMOTE_SESSION_*` 常量 + dispatcher 白名单 |
| `crates/tetherly-core/src/remote_session.rs` | 新 | 3 个帧结构 + `RemoteSessionStateMachine` + 校验器 |
| `crates/tetherly-core/src/input_tunnel.rs` | 改 | `InputTunnelRole` 增补两个 Session 变体 |
| `crates/tetherly-node/src/runtime/remote_session.rs` | 新 | `remote_session_start/stop/status` 编排 |
| `crates/tetherly-node/src/runtime/input.rs` | 改 | `enter_session/exit_session`；方向守卫 Session 分支；`receive_remote_session_request` 处理器 |
| `crates/tetherly-node/src/runtime.rs` | 改 | `Shared.remote_session` 状态 + dispatcher 路由 |
| `crates/tetherly-node/src/bin/tetherly_viewer.rs` | 改 | NSEvent 捕获 + JSONL IPC + 视口映射 |
| `crates/tetherly-node/src/uihttp/api_remote_session.rs` | 新 | 3 个端点 + dto |
| `crates/tetherly-node/src/uihttp.rs` | 改 | 路由注册 |
| `crates/tetherly-node/src/platform/tray.rs` | 改 | 「🖥 远程控制…」菜单（两平台） |
| `tests/phase5.rs` | 改 | m6_* 会话管线测试 |
| `docs/adr/138-remote-desktop-control.md` | 新 | 本文档 |

复用清单（**禁止重写**）：`send_input_events`、`InputClient`、`DualInputSink`、
`open_screen`/`attach_screen_server`/`attach_screen_client`、`ScreenCodec`、
`KeymapProfile`、`InputTunnelValidator`、`InputTunnelPolicy`、viewer 的
`parse_frame`/`build_ui`/`show`。

---

## 4. 里程碑切分（Milestones）

- **M1 — 信令与受控端（纯 daemon，无 UI）**：帧 + 状态机 + `enter_session` 注入路由 + REST + phase5 双节点 wire 测试（MemorySink 断言注入序列）。验收：`POST /api/remote/session/start` 后 MemorySink 收到 Move/Button/Key。
- **M2 — viewer 输入捕获（Mac）**：NSEvent → InputEvent 映射 + JSONL IPC + daemon 转发。验收：真机 Mac 窗口内操作，Windows 光标跟手、键入正确（keymap 验证）。
- **M3 — 编排 + 托盘入口 + 心跳/超时**：一键起会话、关窗即断、idle 超时释放。验收：托盘两击内进入远程控制，关窗后对端 `release_all_inputs` 触发、无卡键。
- **M4（Phase 2）**：Windows 查看窗（Win32 呈现 + 输入，`SystemSink` 已有 GDI present 底子）、H.264 viewer 解码、剪贴板会话联动（既有 clip hub 直接可用）、lock-to-screen。

---

## 5. 测试方案（Verification Plan）

1. **单元（tetherly-core）**：3 帧编解码往返、状态机全转移覆盖、reject 理由序列化契约快照；
2. **管线（tests/phase5.rs 新增 m6_1~m6_6）**：
   - m6_1 请求→接受→MemorySink 收 Enter+Move 序列；
   - m6_2 策略拒绝（`allow_inbound_control=false` → reject_reason=PolicyDisabled）；
   - m6_3 Session 期间漫游守卫跳过（watcher 不 attach）；
   - m6_4 idle 超时 → release + `release_all_inputs`（MemorySink `keys_down` 清空断言）；
   - m6_5 会话中开流失败 → 状态回 Idle 且对端收到 Close；
   - m6_6 键码映射端到端（Mac CGKeyCode 注入 → host MemorySink 收到 VK 重映射码）；
3. **真机 Manual-required**：Mac 控 Windows 全流程手测（含 wheel_speed、Esc 退出、
   保护进程存活复核——部署后 17 点 smoke 中 `SRBMiner/verge-mihomo/komari-agent` PID 检查照常）。

---

## 6. 风险与规避

| 风险 | 规避 |
|---|---|
| viewer JSONL IPC 阻塞呈现线程 | 事件行非阻塞写（有界 channel，满即丢 Move 保留 Button/Key——与屏幕流 backpressure 同哲学） |
| Session 与漫游状态机互踩 | 复用 ADR-136 §7 的"会话仍属我"守卫模式，Session attach 同样携带代际标记 |
| host 物理分辨率与 viewer 视口纵横比不一致 | Resp 回报分辨率，viewer 按比例 letterbox；映射始终用归一化坐标，不受视口缩放影响 |
| Windows 更新进程持有 tetherly.exe | 本功能不改变部署路径；ADR-135 热重启仍为前置推荐件 |
| macOS TCC 权限 | `spawn_input_capture` 已有 accessibility 检查；Session 模式 viewer 侧只捕获**自己窗口**的事件（NSEvent 局部回调），不需要新的全局权限——比漫游模式权限面更小 |

---

## 7. 明确不做（Non-Goals）

- 音频转发（向日葵有；`AudioRoutingFrame 0x0219~0x021C` 底子已在，但 Phase 1 不接）；
- 文件传输进会话窗（已有托盘拖拽 + `/api/files/*`，会话窗内右键发送列为远期）；
- 穿越互联网（本项目 LAN-first，NAT 穿透不在本 ADR）；
- 多查看端并发（单会话不变式，与单通道输入一致）。
