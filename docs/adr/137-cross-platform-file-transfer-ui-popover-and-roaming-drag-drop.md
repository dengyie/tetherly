# ADR-137: 跨端文件传输双引擎（系统托盘 Popover / 灵动悬浮岛与跨屏穿透拖拽）开发设计规范

状态: 部分实施（见下方"实施状态"）  
版本: v0.1.76 / v2.57  
日期: 2026-11-05  

---

## 0. 实施状态（Implementation Status，2026-11-05 校准）

为避免文档与代码脱节，明确本期已落地与未实施的边界：

**本期已实施（代码已上线并经测试）：**
- 域层：`DragControlMsg` / `DragPhase` / `DragSession` / `DragStateMachine` / 归一化坐标（`tetherly-core/src/drag.rs`），1 KiB 线长上限 + `TYPE_DRAG_CONTROL` 帧注册。
- 控制面接线：`DragControlMsg` 经 Noise 加密的会话通道（TCP 45717，非 45719——45719 为纯二进制输入通道）双向收发，接收端 `receive_drag_control` dispatcher 路由至 `DragTarget`。
- 控制端：`DragController`（intent/move/cancel/confirm），重复发起自动先回 `RemoteDragCancel`；confirm 采用 peek-validate-commit-rollback，传输失败可回滚重试。
- 接收端：`DragTarget` 状态机 + 幽灵浮层状态模型 + MOTW（`Zone.Identifier`，ZoneId=3）+ 原生文件管理器 reveal（macOS `open -R` / Windows `explorer.exe /select,`）。投递完成经 `FileOffer` claim → `FileDone` settle 钩子触发。
- REST 控制面：`/api/file(s)/drag/{intent,move,cancel,confirm,status}`（loopback-only，Host 白名单）。
- 测试：frame 编解码往返 + 1 KiB 上限、serde snake_case 契约快照、重复发起取消、confirm 失败回滚、双节点 wire 端到端。

**本期未实施（明确列为后续工作，非 Bug）：**
- 原生 UI 外壳：`NSStatusItem` 托盘 dropzone、Win32 `RegisterDragDrop`/OLE `IDropTarget` STA 线程、Edge Strip 置顶窗口、以及 `GhostOverlayVisual` 对应的真实 NSWindow/Win32 幽灵窗口渲染。当前 `GhostOverlayVisual` 仅为状态模型，经 REST `status` 暴露供前端/未来原生层消费。
- 被动 LMB 手势识别（原 `DragIntentDetector`）：Engine I 采用显式 REST 触发，被动检测器无生产驱动方，已按"禁止无调用死码"原则移除。若未来接入 CGEventTap/WH_MOUSE_LL 手势触发，再于 `spawn_input_capture` 处重建。

---

## 1. 背景与产品意图 (Context & Product Intent)

在日常多端协同场景中（Mac 控制端 + Windows 生产工作站 `home-win`），用户高频需要将本地文件秒级传输至对端。  
目前 Tetherly 已经构建并验证了：
1. **TCP 45719 毫秒级输入控制与多向漫游通道**；
2. **TCP 45718 Noise 加密的 P2P 文件传输中枢**（支持 64 GiB 单文件、64 KiB 分块断点续传、SHA-256 校验与双向秒级落盘）；
3. **HTTP OpenAPI 控制面**（`/api/files/send`, `/api/files/inbox`, `/api/files/resume`）。

然而，当前缺少开箱即用、视觉直观的现代桌面级 UI/UX 体验。基于用户指令，本设计确立**两大互补落地形态**：
- **形态一：边缘灵动悬浮岛 / 系统托盘 Popover（轻量即用即走，Phase 1）**：
  为日常办公提供零心智负担的文件投递通道。文件拖入系统状态栏托盘或拖至屏幕边缘即完成投递，无需切换应用。
- **形态二：跨屏漫游文件拖拽穿透（高级 Deskflow 语义，Phase 2）**：
  当鼠标在两台屏幕间无界漫游时，实现本地鼠标拖拽文件“直接穿过屏幕边界、随光标虚影在对端桌面松手落盘”。

---

## 2. 业界方案剖析与历史陷阱规避 (Industry Analysis & Lessons Learned)

### 2.1 传统 KVM 项目的教训（Deskflow / Synergy / Barrier / Input-Leap）
Deskflow / Input-Leap 官方在 2025/2026 年全面放弃并删除了历史继承自 Synergy 1.x 的拖拽功能，深层根因在于其架构设计缺陷：
1. **OS 模态拖放事件循环阻塞（Modal Drag Loop Hang）**：
   Win32 `DoDragDrop` 和 macOS `NSDraggingSession` 启动后会强行独占线程消息泵。若直接在此循环内尝试做网络漫游与事件注入，会导致操作系统事件钩子被挂起，使光标在跨屏边界遭遇“硬锁定（Hard Lock）”。
2. **传输延迟与落盘时序倒挂（Transfer Latency Hazard）**：
   网络传输不是瞬间完成的。当用户在受控端松开鼠标按键时，大文件尚在传输中，对端目标程序（如 Windows 资源管理器）要求立即提供有效文件路径，因此 Synergy 只能退化为“无论拖到对端哪一窗口，一律悄悄存入 Desktop 目录”，毫无位置感知。
3. **按键状态污染与暴力取消**：
   Synergy 在跨屏时通过向本地窗口伪造按键 `fakeKeyDown(kKeyEscape)` 暴力中断本地 OS 拖放循环，经常引发 Finder 假死或受控端左键按键状态反转（认为按键一直按下）。

### 2.2 现代成功标杆的设计精髓
- **Apple Universal Control（WWDC / macOS Monterey 12.4+）**：
  采用三段式事件解耦：`DRAG`（拖拽开始与位移） $\rightarrow$ `SYNC`（热区跨越） $\rightarrow$ `CLIP`（Promise 虚拟引用）。
  受控端先呈现**虚拟拖拽虚影（Ghost Thumbnail）**，用户松手时在后台通过专用信道高速落盘，落盘完毕后通过系统接口高亮展现。
- **Dropover / Yoink / FlowShelf**：
  提供**边缘感应区（Edge Strip）**与**托盘悬停感应（Menu Bar Dropzone）**，把“文件投递”解耦为“就近放入暂存区”，极大地降低了系统拦截的复杂度。

---

## 3. 架构设计与核心不变式 (Invariants & Architecture)

```
┌────────────────────────────────────────────────────────────────────────┐
│                        Tetherly 传输 UI/UX 双引擎                       │
├───────────────────────────────────┬────────────────────────────────────┤
│   形态一：系统托盘 Popover & 灵动悬浮岛  │    形态二：跨屏漫游穿透（高级 Deskflow）   │
│         （轻量即用即走，Phase 1）       │         （沉浸式穿透，Phase 2）        │
├───────────────────────────────────┼────────────────────────────────────┤
│ • macOS 状态栏 / Win 任务栏托盘图标     │ • 鼠标拖拽文件漫游越界触碰边界         │
│ • 支持全局文件直接拖入托盘图标松手落盘   │ • 控制端捕获拖拽，向受控端发预飞元数据 │
│ • 屏幕边缘/拖拽呼出磨砂玻璃悬浮岛      │ • 受控端屏幕渲染伴随光标的半透明文件虚影│
│ • Popover 查看实时进度与秒级重传       │ • 对端松手即落盘 + 自动打开并选中文件  │
└───────────────────────────────────┴────────────────────────────────────┘
```

### 核心不变式 (Core Invariants)
1. **信道隔离与零阻塞（Channel Separation & Non-blocking I/O）**：
   - 45719 端口仅用于高频键鼠事件及轻量拖拽状态信令（报文 $\le 1\text{ KiB}$），绝不传输文件实体；
   - 所有文件块传输 **100% 走专属 TCP 45718 Noise 加密数据隧道**，利用已有的 `ResumeTable` 享受 64 KiB 分块续传与背压控制，杜绝卡死控制通道。
2. **Windows 生产守护红线（Production Safety Guard）**：
   - Windows 工作站关键生产进程（`SRBMiner-MULTI.exe`, `verge-mihomo.exe`, `komari-agent.exe`）不受任何 UI/传输组件的干扰；
   - 托盘与浮动窗口在 Windows SYSTEM 交互会话（Session 2）下以最高兼容性无边框渲染，绝不触发前台抢占焦点。
3. **状态幂等性与优雅取消（Graceful Cancellation & Idempotency）**：
   - 用户在跨屏漫游途中按下 `ESC` 或拖回本地屏幕，两端状态机立即释放虚拟光标并废弃临时会话，不留下任何僵尸句柄或垃圾文件碎片。
4. **安全防投毒与只高亮不执行原则（Security & Reveal Only）**：
   - 接收端落盘完成后自动写入 NTFS `Zone.Identifier`（Mark of the Web，ZoneId=3）；
   - 落地后调用系统 Shell **仅进行文件聚焦高亮（Reveal in File Manager）**，严禁直接通过默认程序执行接收文件。

### 运行时线程模型（Threading Architecture）
- **macOS**：AppKit 窗口、托盘与 `NSDraggingDestination` 绑定在 OS 主线程（`dispatch_get_main_queue`），与 Tokio 运行时解耦；
- **Windows**：Win32 `RegisterDragDrop` 与 OLE `IDropTarget` 运行于独立的 **Win32 STA (Single-Threaded Apartment) UI 工作线程**，杜绝在 Tokio MTA 线程初始化 COM 导致的 `RPC_E_CHANGED_MODE` 崩溃。

---

## 4. 引擎一：系统托盘 Popover 与边缘灵动悬浮岛 (Phase 1)

### 4.1 系统托盘拖拽投递（Tray Dropzone）
- **macOS 实现**：
  为 `Tetherly.app` 的 `NSStatusItem` 绑定拖拽目标协议：
  ```swift
  // AppKit / objc2 桥接
  statusItem.button?.registerForDraggedTypes([.fileURL])
  ```
  实现 `NSDraggingDestination` 协议：
  - `draggingEntered:`: 触发状态栏图标外围微光高亮动画（Pulse Glow），返回 `NSDragOperationCopy`；
  - `draggingExited:`: 恢复默认图标外观；
  - `performDragOperation:`: 提取 `NSPasteboard` 中的文件 URL 列表，通过 Loopback API 发送任务：
    `POST http://127.0.0.1:45716/api/files/send { "target_peer": "home-win", "file_paths": [...] }`。
- **Windows 实现**：
  通过 Win32 `Shell_NotifyIcon` 维护托盘，同时伴随一个置顶透明的分层接收微窗（Layered DropTarget Window），注册 OLE `IDropTarget` 接口，解析 `CF_HDROP` 提取本地文件。

### 4.2 边缘灵动悬浮岛（Edge Strip Dropzone）
- **感应触发**：
  在主屏靠近漫游方向的边缘（如 Mac 控制端屏幕右侧边缘）部署一条宽度为 2px 的全透明置顶响应窗口（Edge Strip）。
- **动效展开**：
  当用户在本地屏幕拖动文件撞击右边缘时，感应条自动展开为宽度为 120px 的磨砂玻璃卡片（Liquid Glass 材质），卡片展示：
  `🖥️ home-win (Windows) | 松手直投`。
- **松手投递**：
  用户若在悬浮岛内松手，直接触发 45718 文件推送，无需让光标越境；若用户继续加速推边，则平滑进入形态二（跨屏漫游穿透）。

### 4.3 托盘 Popover 交互面板
单击托盘图标时展开微型 Popover，包含：
- **Peer 状态栏**：连接状态（🟢 home-win 在线，Ping: 0.3ms）；
- **快速投递区**：中间虚线大拖拽框（支持点击浏览文件或外部拖拽放入）；
- **实时传输列表**：显示当前传输任务、动态环形百分比进度条、传输速率（如 `112.4 MB/s`）；
- **操作快捷项**：对端接收目录一键跳转（「在访达中显示 / 在资源管理器中显示」）、历史任务重试。

---

## 5. 引擎二：跨屏漫游文件拖拽穿透 (Phase 2)

### 5.1 拖拽生命周期与状态机转换
```
[Idle]
  │ (检测到 LeftMouseDown + 累积位移 > 5px)
  ▼
[LocalDragCandidate]
  │ (光标抵达屏幕边缘，触发越界条件)
  ▼
[CrossingBorder] ──(提取本地 NSPasteboard/OLE 文件元数据)
  │
  ├─ 发送 ControlMsg::RemoteDragEnter ─► 受控端 Windows 呈现虚拟拖拽虚影
  ▼
[RemoteDragActive] ──(光标在 Windows 屏幕漫游，虚影跟随移动)
  │
  ├─ [用户按下 ESC 或光标拖回 Mac] ──► 发送 ControlMsg::RemoteDragCancel (清除虚影，状态回滚)
  │
  ▼ (用户在 Windows 桌面/窗口松开 LeftMouseUp)
[RemoteDropping]
  │
  ├─ 受控端发送 ControlMsg::RemoteDropConfirm { drop_pos }
  ├─ 控制端立即通过 45718 端口启动高优先级并行推流
  │
  ▼ (45718 传输完成落盘)
[RemoteDelivered]
  │
  └─ 受控端调用 Shell API 自动高亮新文件，托盘提示浮出
```

### 5.2 协议层扩展（TCP 45719 Framing）
在 `crates/tetherly-core/src/input/control_msg.rs` 中新增报文：

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DragControlMsg {
    /// 拖拽穿透进入受控端（预飞元数据信令）
    RemoteDragEnter {
        drag_id: u64,
        file_count: u32,
        total_bytes: u64,
        first_file_name: String,
        norm_x: f32,
        norm_y: f32,
        has_directory: bool,
    },
    /// 受控端虚拟光标位移更新（归一化坐标消解高分屏 DPI 差异）
    RemoteDragMove {
        drag_id: u64,
        norm_x: f32,
        norm_y: f32,
    },
    /// 拖拽取消（ESC、拖回本地或断线）
    RemoteDragCancel {
        drag_id: u64,
        reason: DragCancelReason,
    },
    /// 受控端松手确认，要求控制端立即推流
    RemoteDropConfirm {
        drag_id: u64,
        drop_norm_x: f32,
        drop_norm_y: f32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DragCancelReason {
    UserEscPressed,
    DraggedBackToLocal,
    SessionTimeout,
    NetworkDisconnected,
}
```

### 5.3 受控端（Windows）虚拟拖拽虚影实现
受控端收到 `RemoteDragEnter` 后，在 Session 2 下激活一个轻量、穿透的置顶分层窗口（Ghost Overlay）：
- **Win32 特性**：
  使用 `CreateWindowExW` 创建带 `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOPMOST` 属性的窗口；
- **视觉呈现**：
  半透明背景（Alpha = 200），渲染通用文件小图标、文件名（如 `📊 annual_report.pdf`）以及文件大小；
- **跟随位移**：
  伴随 45719 的 `MouseMove` 事件极速更新窗口位置（`SetWindowPos`），零延迟跟随光标；
- **松手后消失**：
  收到 `LeftMouseUp` 瞬间，虚影切换为带旋转 Spinner 的微型进度卡片，推流完毕后淡出。

### 5.4 物理落盘与 Windows Shell 自动唤起定位
- 45718 传输引擎将文件直接落盘至 Windows 工作站隔离目录：
  `E:\code\tetherly\data\inbox\<filename>`
- 落盘完成校验 SHA-256 无误后，调用 Win32 Shell API：
  ```cpp
  // 自动打开资源管理器并高亮选中该文件
  PIDLIST_ABSOLUTE pidl = ILCreateFromPathW(filePath);
  if (pidl) {
      SHOpenFolderAndSelectItems(pidl, 0, NULL, 0);
      ILFree(pidl);
  }
  ```
  实现“松手 $\rightarrow$ 毫秒落盘 $\rightarrow$ 资源管理器自动呈现所选文件”的极致丝滑体验。

---

## 6. 模块代码划分与工程组织 (Module Organization)

```
crates/tetherly-core/
├── src/
│   ├── input/
│   │   ├── control_msg.rs        # 新增 DragControlMsg 协议报文
│   │   └── drag_state.rs         # 拖拽会话状态机定义与超时治理
│   └── p2p/
│       └── file_channel.rs       # 45718 P2P 隧道传输接口

crates/tetherly-node/
├── src/
│   ├── runtime/
│   │   ├── drag_controller.rs    # 控制端拖拽识别、越界检测与元数据提取
│   │   ├── drag_target.rs        # 受控端虚影渲染、松手握手与落盘通知
│   │   └── input.rs              # 集成拖拽状态判断至现存 CGEventTap / WH_MOUSE_LL
│   └── ui/
│       ├── tray/                 # 跨平台系统托盘与拖拽直投
│       │   ├── macos.rs          # NSStatusItem + NSDraggingDestination
│       │   └── windows.rs        # Shell_NotifyIcon + IDropTarget
│       ├── popover/              # 轻量 Web/原生 Popover 管理面板
│       └── dropzone/             # 边缘灵动悬浮岛（Edge Strip Shelf）
```

---

## 7. 异常处理与边界容错矩阵 (Edge Cases & Fault Tolerance)

| 场景 | 潜在风险 | 防御与自愈策略 |
| :--- | :--- | :--- |
| **中途拖拽取消** | 用户在受控端晃动后按 `ESC` 或直接拖回控制端 | 发送 `RemoteDragCancel`，受控端瞬间销毁 Ghost 窗口，控制端重置拖拽候选状态，无任何残留 |
| **网络偶发抖动** | 拖拽在受控端松手时，TCP 45719 出现毫秒级重连 | 借力 v0.1.75 的 2.5s 优雅重连缓冲，握手包携带 `drag_id` 自动幂等重放，不会打断已确认的推流 |
| **大文件推流中途退出** | 传输 50GB 文件耗时较长，用户关闭了控制机 | 45718 `ResumeTable` 自动保存已落盘的 64 KiB 块校验状态，下次开机/连线直接断点续传 |
| **Windows UAC / 提权** | 受控端运行在 SYSTEM 账户或提升权限窗口 | 虚影窗口与托盘基于当前登录用户的交互桌面会话（Session 2）初始化，规避跨 Session UI 隔离 |
| **文件同名冲突** | 目标目录已存在同名文件 | 采用原子递增命名（如 `report (1).pdf`），确保不覆盖对端原有资产 |

---

## 8. 实施与验证规范 (Verification Plan)

### 8.1 阶段性交付节点
- **M1 (Phase 1)**: 系统托盘与 Popover 基础架构落地，完成 macOS 状态栏图标与 Windows 任务栏图标创建；
- **M2 (Phase 1)**: 托盘拖拽直投跑通：拖拽文件到托盘图标松手，自动调用 45718 传送到对端 `inbox`；
- **M3 (Phase 1)**: 边缘灵动悬浮条实现，靠近屏幕边缘自动滑出磨砂玻璃投递岛；
- **M4 (Phase 2)**: 跨屏漫游穿透信令打通（`RemoteDragEnter` $\rightarrow$ `RemoteDropConfirm`）；
- **M5 (Phase 2)**: Windows Ghost Overlay 虚影光标渲染与落盘自动高亮完成；
- **M6**: 全链路冒烟测试与保护进程（SRBMiner 等）审计无干扰验证。

### 8.2 自动化测试用例
1. `test_drag_control_msg_codec`：验证拖拽控制信令序列化与反序列化二进制一致性；
2. `test_drag_state_machine_cancel_and_timeout`：验证拖拽超时（10s 无动作）及 ESC 取消时状态机正确回滚至 `Idle`；
3. `test_tray_drop_to_p2p_pipeline`：模拟本地注入文件拖入托盘事件，断言对端 45718 `inbox` 秒级生成对应文件及 SHA-256 匹配；
4. `test_protected_process_zero_interference`：在文件拖拽与落盘全程持续审计 Windows 生产进程（`SRBMiner-MULTI`、`verge-mihomo`、`komari-agent`），确保其 CPU 与 WorkingSet 100% 零扰动。
