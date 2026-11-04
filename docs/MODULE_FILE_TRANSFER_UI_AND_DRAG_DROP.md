# 跨端文件传输双引擎（系统托盘 Popover / 灵动悬浮岛与跨屏穿透拖拽）开发设计与工程落地指南

- **版本**: v0.1.76 / v2.57
- **模块标识**: `tetherly-ui-file-transfer`
- **对应 ADR**: [ADR-137: 跨端文件传输双引擎架构设计规范](adr/137-cross-platform-file-transfer-ui-popover-and-roaming-drag-drop.md)
- **更新日期**: 2026-11-05
- **责任架构**: 控制端 (macOS) + 受控生产端 (Windows 11, `home-win`)

---

## 1. 模块设计总览 (Executive Summary)

本模块旨在为 Tetherly 打造媲美甚至超越 Apple Universal Control（通用控制）及 Dropover 的新一代跨端文件传输与拖拽交互体系。

针对跨平台（macOS $\leftrightarrow$ Windows）协同操作，系统构建**两套互补、可独立启用、无缝融合的传输引擎**：

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                             Tetherly 文件传输双引擎                                │
├────────────────────────────────────────┬─────────────────────────────────────────┤
│    引擎一：系统托盘 Popover 与边缘悬浮岛     │    引擎二：跨屏漫游文件穿透拖拽（Deskflow+） │
│            （轻量即用即走，Phase 1）         │            （沉浸式跨屏穿透，Phase 2）      │
├────────────────────────────────────────┼─────────────────────────────────────────┤
│ • macOS 菜单栏 / Windows 任务栏常驻图标 │ • 鼠标抓取文件在多屏间物理漫游穿透      │
│ • 支持外部任意文件直接拖入托盘图标松手投递│ • 控制端无模态事件捕获，避免系统级锁死   │
│ • 靠近漫游屏幕边缘自动滑出磨砂玻璃悬浮岛│ • 受控端原生置顶无焦点虚影窗口（Ghost）  │
│ • Popover 查看 Peer 状态、速度环与历史 │ • 松手即时落盘 + 自动打开 Explorer 高亮 │
└────────────────────────────────────────┴─────────────────────────────────────────┘
```

---

## 2. 业界技术深度对比与致命陷阱规避 (Deep Industry Analysis)

在设计本模块前，我们对业内经典项目与现代标杆进行了逆向与源码级调研（包括 Deskflow、Synergy 1.x、Barrier、Input-Leap、Apple Universal Control、Microsoft PowerToys Mouse Without Borders、Dropover）：

### 2.1 Deskflow / Synergy 历史拖拽废弃的根因剖析
Deskflow 与 Input-Leap 在 2025/2026 年全面移除了历史拖拽代码。其致命缺陷包括：
1. **系统模态循环挂死（Modal Loop Hang）**：
   - Windows 的 `DoDragDrop` 与 macOS 的 `NSDraggingSession` 一旦启动，将独占操作系统线程消息循环（Message Pump）。
   - Synergy 尝试在模态循环内部做网络通信与鼠标漫游，导致本地全局输入钩子挂起，光标在屏幕边界陷入“硬锁定”（Hard Lock），无法平滑越界。
2. **时序倒挂与伪造按键破坏系统状态**：
   - 网络推流需要时间，受控端松手时实体文件根本尚未落盘；Windows 接收端窗口（如资源管理器）查询 `IDataObject` 时无法返回真实句柄，导致 Synergy 只能强行落盘到硬编码的 `Desktop`。
   - Synergy 在越界时向系统伪造 `fakeKeyDown(kKeyEscape)` 暴力退出拖放循环，经常引发 Finder 假死或按键反转死锁（按键松开后受控端仍判定按键被按下）。

### 2.2 Tetherly 的创新破局方案
Tetherly 采用**虚拟控制面与实体数据面彻底解耦**的三段式架构：
- **控制面（TCP 45719）**：仅承载轻量状态信令（`DragControlMsg`，$\le 1\text{ KiB}$），控制端不调用本地阻塞式 `DoDragDrop`，而是通过高频输入事件流判断拖拽意图并发送“预飞元数据”；
- **展示面（Ghost Overlay）**：受控端展示完全无焦点的半透明虚影光标，伴随鼠标位移渲染，彻底避免两端操作系统拖拽循环的冲突；
- **数据面（TCP 45718）**：受控端确认松手瞬间触发数据流推流，使用既有的 Noise 加密通道与 64 KiB 分块续传技术（`ResumeTable`），在后台毫秒级落盘并唤起 Shell 高亮。

---

## 3. 核心不变式与安全防护红线 (Invariants & Safety Guarantees)

为确保生产环境的高可用与绝对安全，本模块的所有代码必须无条件遵守以下不变式：

1. **Windows 关键生产任务 100% 零干扰（Absolute Production Invariant）**：
   - 目标 Windows 生产工作站（`home-win`）长期运行核心计算任务：`SRBMiner-MULTI.exe`（挖矿）、`verge-mihomo.exe`（代理）、`komari-agent.exe`（探针）；
   - 所有 UI 窗口、托盘消息泵、文件 I/O 必须在独立的低优先级后台线程执行，严禁调用全局阻塞性 Hook，严禁争抢 CPU/GPU 渲染资源，严禁抢占前台输入焦点。
2. **信道隔离与零阻塞（Channel Decoupling）**：
   - TCP 45719 为毫秒级键鼠控制专属信道，严禁承载任何文件实体二进制数据；
   - 所有文件传输统一经由 TCP 45718 数据中枢或 HTTP `/api/files/send` 发起。
3. **状态幂等与自愈（Graceful Cancellation & Resumption）**：
   - 用户在拖拽漫游期间按 `ESC`、拖回本地屏幕、或遭遇网络抖动时，状态机必须安全回滚至 `Idle`，并在 50ms 内销毁虚影窗口与临时句柄，杜绝文件碎片与僵尸进程。
4. **安全防投毒与只高亮不执行原则（Security & Reveal Only）**：
   - 受控端落地文件仅限于安全隔离沙箱目录 `data/inbox`，严格执行 `sanitize_file_name` 防范路径穿越；
   - 落盘完成后必须为 Windows NTFS 写入 `Zone.Identifier`（Mark of the Web，ZoneId=3），阻断未知脚本恶意自执行；
   - 操作系统通知与文件定位**仅允许调用文件管理器高亮选中（Reveal in File Manager）**，严禁直接通过默认程序打开执行接收到的文件。

---

## 4. 跨平台 UI 线程模型与运行时桥接设计 (Threading & Event Loop Architecture)

这是解决跨平台桌面 UI 稳定性的首要架构基石：

```
                     ┌─────────────────────────────────────────────────────────┐
                     │           tetherly-node Tokio Async Runtime             │
                     │          (MTA Thread Pool, Networking, Core)            │
                     └────────────────┬───────────────────────┬────────────────┘
                                      │                       │
              tokio::sync::mpsc cross-thread channels         │
                                      │                       │
                                      ▼                       ▼
      ┌───────────────────────────────────────────┐ ┌───────────────────────────────────────────┐
      │          macOS AppKit Main Thread         │ │          Windows STA Worker Thread        │
      ├───────────────────────────────────────────┤ ├───────────────────────────────────────────┤
      │ • dispatch_async(dispatch_get_main_queue) │ │ • CoInitializeEx(COINIT_APARTMENTTHREADED)│
      │ • NSStatusItem & NSDraggingDestination    │ │ • OleInitialize(NULL) for OLE D&D       │
      │ • Edge Strip Window (NSWindow)            │ │ • Win32 MsgPump (GetMessage/Dispatch)   │
      │ • CoreAnimation 60/120fps Liquid Glass    │ │ • WS_EX_LAYERED Ghost Drag Overlay Window │
      └───────────────────────────────────────────┘ └───────────────────────────────────────────┘
```

### 4.1 macOS AppKit 调度规范
- `NSStatusItem`、`NSWindow`、`NSVisualEffectView` 必须运行在操作系统主线程；
- `tetherly-node` 如果作为 CLI/守护进程在非主线程启动 Tokio 运行时，与 AppKit 的通信必须统一封装在 `crates/tetherly-node/src/ui/macos_dispatch.rs` 中：
  ```rust
  #[cfg(target_os = "macos")]
  pub fn run_on_main<F: FnOnce() + Send + 'static>(f: F) {
      use block2::RcBlock;
      // 封装 dispatch_async 到 main queue
      dispatch::Queue::main().exec_async(f);
  }
  ```

### 4.2 Windows Win32 STA 线程规范
- Win32 的 `RegisterDragDrop` 与 OLE `IDropTarget` 必须在 **STA (Single-Threaded Apartment)** 线程上创建，并维持专属的标准消息循环；
- 如果直接在 Tokio MTA 异步工作线程调用 `OleInitialize`，会导致 `RPC_E_CHANGED_MODE` 报错且 OLE 拖放静默失效；
- **实现模型**：
  在 `crates/tetherly-node/src/ui/windows_sta.rs` 中，派生一个专用的 STA UI 线程：
  ```rust
  #[cfg(target_os = "windows")]
  pub struct Win32StaWorker {
      tx_cmd: std::sync::mpsc::Sender<StaCommand>,
  }

  impl Win32StaWorker {
      pub fn spawn() -> Self {
          let (tx_cmd, rx_cmd) = std::sync::mpsc::channel();
          std::thread::Builder::new()
              .name("tetherly-win32-sta-ui".into())
              .spawn(move || {
                  unsafe {
                      windows::Win32::System::Com::CoInitializeEx(
                          None,
                          windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                      ).expect("CoInitializeEx STA");
                      windows::Win32::System::Ole::OleInitialize(None)
                          .expect("OleInitialize");
                  }
                  
                  // 初始化托盘、透明 DropTarget 接收窗、Ghost Overlay
                  let mut ui_context = Win32UiContext::init();
                  
                  // 标准 Win32 消息泵 + 跨线程通道监听
                  loop {
                      while let Ok(cmd) = rx_cmd.try_recv() {
                          ui_context.handle_cmd(cmd);
                      }
                      
                      let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
                      unsafe {
                          if windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
                              &mut msg, None, 0, 0,
                              windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
                          ).as_bool() {
                              if msg.message == windows::Win32::UI::WindowsAndMessaging::WM_QUIT {
                                  break;
                              }
                              windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
                              windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
                          } else {
                              std::thread::sleep(std::time::Duration::from_millis(5));
                          }
                      }
                  }
              }).expect("spawn win32 sta thread");
          Self { tx_cmd }
      }
  }
  ```

---

## 5. 拖拽识别与边缘网关设计 (Edge Drag Gateway)

### 5.1 痛点与深度发现：为什么 CGEventTap 拿不到文件？
在 macOS 底层，`CGEventTap` 只能获取原始硬件输入事件（`kCGEventLeftMouseDown`, `kCGEventLeftMouseDragged`），**操作系统出于安全隐私隔离，绝不会在全局输入事件中暴露正在被拖动的 Finder 文件句柄或 Pasteboard 数据**。

**关键突破**：将**边缘灵动感应条（Edge Strip）**确立为**跨端拖拽的官方网关（Drag Capture Gateway）**！

```
                     macOS 本地桌面 (Finder)
  [用户选中文件拖动] ────────────► [光标撞击主屏右侧边缘]
                                            │
                                            ▼
                           ┌──────────────────────────────────┐
                           │   Edge Strip 2px 透明置顶响应窗   │
                           │     (注册 NSDraggingDestination)  │
                           └────────────────┬─────────────────┘
                                            │
                                            ├─► 系统回调 draggingEntered(sender)
                                            │   官方合法提取 sender.draggingPasteboard
                                            │
                                            ├─► [速度慢/悬停] ──► 展开 Liquid Glass 悬浮岛 (松手即直投)
                                            │
                                            └─► [持续推边越界] ──► 触发 45719 预飞信令 RemoteDragEnter
                                                                   光标穿透，Windows 虚影现身！
```

### 5.2 Edge Strip 网关的生命周期事件
1. **`draggingEntered:(id<NSDraggingInfo>)sender`**：
   - 提取 Pasteboard 中的 URL 列表：
     ```objc
     NSArray<NSURL *> *urls = [sender.draggingPasteboard
         readObjectsForClasses:@[[NSURL class]]
         options:@{NSPasteboardURLReadingFileURLsOnlyKey: @YES}];
     ```
   - 快速获取：文件数量、首文件名、文件预估总大小；
   - 启动本地动画计时器与漫游推边判定。
2. **`draggingUpdated:(id<NSDraggingInfo>)sender`**：
   - 若用户光标只是在屏幕边缘轻微晃动：Edge Strip 向内平滑展开 120px 磨砂玻璃卡片（悬浮岛形态）；
   - 若用户光标加速越过右边界（越界漫游阈值）：
     - 立即向 TCP 45719 发送 `ControlMsg::RemoteDragEnter`；
     - 本地光标进入隐藏/漫游锁边态，控制权交予受控端。
3. **`performDragOperation:(id<NSDraggingInfo>)sender`**：
   - 若用户直接在展开的悬浮岛内松手：触发本地 Engine I 直投，无需跨屏。

---

## 6. 协议层报文设计与坐标归一化 (Protocol Specification)

在 `crates/tetherly-core/src/input/control_msg.rs` 中扩充：

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DragControlMsg {
    /// 控制端拖拽穿透边界进入受控端（预飞元数据信令）
    RemoteDragEnter {
        /// 全局唯一拖拽会话 ID
        drag_id: u64,
        /// 本次拖拽包含的文件数量
        file_count: u32,
        /// 总字节数 (用于受控端虚影展示及落盘磁盘空间预检)
        total_bytes: u64,
        /// 第一个文件名 (用于受控端虚影悬浮提示)
        first_file_name: String,
        /// 进入受控端屏幕时的归一化坐标 [0.0, 1.0]
        norm_x: f32,
        norm_y: f32,
        /// 是否包含文件夹/多层目录
        has_directory: bool,
    },

    /// 受控端光标位移更新 (高频同步虚影位置，采用归一化坐标消解 Retina/4K DPI 差异)
    RemoteDragMove {
        drag_id: u64,
        norm_x: f32,
        norm_y: f32,
    },

    /// 拖拽取消 (用户按 ESC、光标拖回本地屏幕、或网络断开)
    RemoteDragCancel {
        drag_id: u64,
        reason: DragCancelReason,
    },

    /// 受控端松开鼠标按键 (LeftMouseUp)，确认落盘位置并请求控制端推流
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

### 6.1 高分屏 DPI 缩放与多显示器坐标系归一化 (High-DPI Alignment)
- 控制端（MacBook Pro Retina，2x 缩放）与受控端（Windows 4K，150% 缩放）物理像素差异巨大；
- **规范**：
  - 协议层报文 `norm_x, norm_y` 统一采用**归一化比例坐标 $[0.0, 1.0]$**，相对于受控端活动显示器的可见工作区；
  - Windows 端显式声明开启 `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`；
  - 受控端收到 `(norm_x, norm_y)` 后，乘以当前屏幕的物理分辨率计算窗口位移：
    $$X_{\text{window}} = \text{ScreenLeft} + \lfloor \text{norm\_x} \times \text{ScreenWidth} \rfloor$$
    $$Y_{\text{window}} = \text{ScreenTop} + \lfloor \text{norm\_y} \times \text{ScreenHeight} \rfloor$$
  - 保证虚影窗口在任何 DPI 缩放下 100% 紧贴鼠标尖端，不产生位移抖动。

---

## 7. 多文件与文件夹递归处理管道 (Folder & Multi-File Pipeline)

当用户拖拽多个文件或整个工程目录时：

```
                    用户拖入实体 (Files / Folders)
                                 │
                                 ▼
                     判断实体类型 (Type Check)
                     ┌───────────┴───────────┐
                     │                       │
              [单个/多个普通文件]       [包含文件夹/目录]
                     │                       │
                     │                       ├─► 方案 A: 递归生成相对路径清单 (Manifest)
                     │                       │   逐层创建子目录并在 45718 流式推流
                     │                       │
                     │                       └─► 方案 B (推荐): 零拷贝流式归档
                     │                           自动封装为 folder.tetherly-bundle
                     │                           受控端落盘后在安全沙箱内还原目录树
                     ▼                       ▼
            TCP 45718 Noise 加密 64 KiB 分块断点续传隧道 (ResumeTable)
```

1. **单文件与多文件**：直接通过已有的 `Vec<FileMeta>` 队列按序推流；
2. **文件夹（Directory）**：
   - 发送端在内存中利用 `tar`/`zstd` 极速流式归档为单个临时实体；
   - 接收端落盘至 `data/inbox/<dirname>` 时自动校验并展开，保证单次会话与断点续传的一致性。

---

## 8. 安全防投毒防护与系统 Shell 深度集成 (Security & System Shell Integration)

### 8.1 绝不执行原则 (Never Execute Directly)
- 历史软件往往在落盘后直接调用 `ShellExecuteW(..., L"open", ...)`，这存在严重安全漏洞（若传入恶意 `.bat`, `.cmd`, `.exe` 会立即运行）；
- **Tetherly 规范**：严禁调用默认打开程序，统一调用**系统文件管理器聚焦选中接口（Reveal / Select in File Manager）**！

### 8.2 Windows 原生高亮与 MOTW 标记实现
在 Windows 受控端完成落盘并校验 SHA-256 后：
1. **写入 Mark of the Web (Zone.Identifier)**：
   ```rust
   #[cfg(target_os = "windows")]
   pub fn apply_motw(file_path: &std::path::Path) -> std::io::Result<()> {
       let zone_path = format!("{}:Zone.Identifier", file_path.display());
       std::fs::write(
           zone_path,
           "[ZoneTransfer]\r\nZoneId=3\r\nReferrerUrl=tetherly://peer/mac\r\nHostUrl=tetherly://peer/mac\r\n",
       )
   }
   ```
2. **调用 Shell 聚焦选中**：
   优先通过 COM 调用 `SHOpenFolderAndSelectItems`；若处于无 COM 会话环境，平滑降级执行 `explorer.exe /select,"<path>"`：
   ```rust
   #[cfg(target_os = "windows")]
   pub fn reveal_in_explorer(file_path: &std::path::Path) {
       let path_str = file_path.to_string_lossy();
       let _ = std::process::Command::new("explorer.exe")
           .arg(format!("/select,{}", path_str))
           .spawn();
   }
   ```

### 8.3 macOS 原生定位实现
在 macOS 端完成落盘后：
```rust
#[cfg(target_os = "macos")]
pub fn reveal_in_finder(file_path: &std::path::Path) {
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(file_path)
        .spawn();
}
```

---

## 9. 状态转换矩阵与极端边界防御 (Fault Tolerance & Edge Cases)

| 场景 | 触发条件 | 控制端行为 | 受控端行为 | 状态机自愈保障 |
| :--- | :--- | :--- | :--- | :--- |
| **正常松手交付** | 受控端松开 LMB | 收到 `DropConfirm`，立即发起 45718 高速推流 | 虚影切换为 Spinner 进度条，推流完毕后调用 Shell 选中文件并淡出 | 完整落盘并写入 MOTW，记录审计日志 |
| **中途按 ESC** | 受控端在任意位置按 ESC | 收到 `Cancel`，取消推流任务，恢复鼠标控制 | 16ms 瞬间销毁 Ghost Overlay 窗口 | 双方无残留句柄，不触发任何文件写入 |
| **折返回本地屏幕** | 拖拽途中鼠标滑回 Mac 屏 | 识别光标越过本地边界，发送 `RemoteDragCancel` | 立即销毁 Ghost 窗口 | 恢复本地光标可见性与拖拽取消 |
| **网络偶发瞬断** | TCP 45719 出现异常断开 | 进入 2.5s 优雅重连宽限期，本地锁边缓冲 | 保持虚影当前位置等待 2.5s | 若 2.5s 内重连成功则恢复；超时则双方安全回滚至 `Idle` |
| **落盘同名冲突** | `inbox` 已存在同名文件 | 正常传输，保留原始源文件名 | 自动执行原子递增命名：`spec (1).pdf` | 绝不静默覆盖已有文件，确保资产安全 |
| **恶意路径穿越攻击** | 报文携带 `../../System32/cmd.exe` | 拒绝发送 | 严格触发 `sanitize_file_name`，剥离全部路径仅留合规文件名 | 杜绝任意路径写入攻击 |

---

## 10. 自动化测试与生产防护审计方案 (TDD & Verification)

### 10.1 单元测试集 (`crates/tetherly-core/src/input/drag_state.rs`)
1. `test_drag_control_msg_roundtrip_codec`：验证 `DragControlMsg` 的 Serde 二进制/JSON 序列化与反序列化一致性；
2. `test_drag_state_transition_matrix`：模拟完整生命周期：`Idle` $\rightarrow$ `LocalCandidate` $\rightarrow$ `CrossingBorder` $\rightarrow$ `RemoteDragActive` $\rightarrow$ `Dropping` $\rightarrow$ `Delivered`；
3. `test_drag_timeout_and_cancel_invariants`：验证 10 秒超时未松手自动回滚至 `Idle`，ESC 取消时各端资源正确释放。

### 10.2 跨平台集成测试与生产守护审计
- 运行测试用例时，持续审计 Windows 生产工作站（`home-win`）：
  ```powershell
  # 生产关键进程审计检查 (严禁中断)
  Get-Process SRBMiner-MULTI, verge-mihomo, komari-agent -ErrorAction Stop
  ```
- 验证在连续 100 次拖拽文件（包括 10MB、1GB、10GB 大文件）的推流与落盘过程中，Windows 生产进程的 CPU、内存与网络吞吐 100% 零扰动。
