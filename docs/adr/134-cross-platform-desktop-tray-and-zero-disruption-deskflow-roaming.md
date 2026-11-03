# ADR-134 跨平台桌面系统托盘图标、零扰民权限治理与硬件位移无界漫游 (Cross-Platform Desktop Tray, Zero-Prompt Permission Governance, and Hardware Delta Roaming)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-11-02，v2.48 / v0.1.70）**——完整实现 macOS 顶部菜单栏 `NSStatusBar`（`⇄ Tetherly`）与 Windows 任务栏右下角 `Shell_NotifyIconW` 原生系统托盘；彻底根除 macOS 辅助功能系统弹窗风暴，改用静默探测 + 优雅降级 + `/Applications/Tetherly.app` 永久凭据；打通硬件相对位移增量（`MouseEventDeltaX/Y`）与 `local_move_delta` 累加引擎，解决锁边时光标冻结问题；消除端口冲突，实现 45719 端口 Noise 加密数据链路毫秒级自愈。 |
| 日期 | 2026-11-02 |
| 关联 | ADR-071（输入通道安全策略）、ADR-072（屏幕边缘漫游）、ADR-132（macOS 原生键鼠与重映射引擎）、ADR-133（四向空间漫游与拓扑对齐） |

---

## 动机与背景

在用户关闭第三方工具（Deskflow）并全面切换到 Tetherly 作为生产环境跨屏键鼠漫游与剪贴板中枢的实机演练中，暴露出以下关键系统级断点：
1. **桌面交互存在感缺失**：
   用户反馈软件在后台运行缺乏类似 Deskflow / Flow 的桌面托盘图标与状态指示，无法直观判断运行态与当前对端，也缺少双击直达控制台的快捷入口。
2. **系统权限弹窗风暴**：
   macOS `CGEventTapCreate` 在未授权辅助功能时反复触发系统权限弹窗，打扰生产机正常办公。同时，反复重构的命令行裸二进制会导致 macOS TCC 权限哈希频繁失效，需要频繁重新勾选。
3. **跨屏漫游光标坐标冻结**：
   当光标漫游至 Windows 受控端时，macOS 端为抑制本地物理光标移动，在 `CGEventTap` 回调中返回 `null_mut()`。然而 WindowServer 抑制移动后，绝对坐标 `CGEvent::location` 将停滞在屏幕上边缘（`y = 0`），导致相对位移无法向 Windows 持续发送，光标在 Windows 屏幕上无法自由移动。
4. **端口冲突与数据流阻断**：
   启动参数与默认端口配置偶发重叠，导致底层输入端口（45719）绑定冲突或数据流未随控制隧道同步联通。

---

## 架构与技术实现

### 1. 原生跨平台系统托盘与常驻进程体系 (`crates/tetherly-node/src/platform/tray.rs`)

无需引入庞大的第三方 GUI 框架，直接复用既有系统绑定特性实现轻量原生托盘：
- **macOS 顶部菜单栏 (`NSStatusBar`)**：
  * 主线程托管 `NSApplication`，设置为 `Accessory` 模式（`LSUIElement = 1`），不在 Dock 栏显示冗余大图标，仅在顶部菜单栏常驻 `⇄ Tetherly` 图标。
  * 动态上下文菜单：
    - `🟢 Tetherly: 运行中 (设备别名)`
    - `🌐 打开管理控制台 (127.0.0.1:45716)`（调用系统默认浏览器）
    - `🖥️ 漫游拓扑: Deskflow 对齐 (边缘探测)`
    - `⌨️ 键位映射: Mac ↔ Windows (Cmd ↔ Ctrl)`
    - `🔒 辅助功能权限: 已授权 / ⚠️ 未授权 (点击设置)`（未授权时高亮且可一键唤起系统设置）
    - `❌ 退出 Tetherly`
- **Windows 任务栏通知区 (`Shell_NotifyIconW`)**：
  * 在 Session 2 交互上下文中注册轻量隐藏窗口（`WNDCLASSEXW`）与消息泵循环（`GetMessageW` / `DispatchMessageW`）。
  * 单击/双击托盘图标：拉起默认浏览器访问 `http://127.0.0.1:45716`。
  * 右键弹出原生菜单（`TrackPopupMenuEx`）：提供状态指示、拓扑指示、控制台直达与安全退出。

### 2. 零扰民权限治理与持久化 App Bundle 规范

- **只读静默探测**：严格使用 `AXIsProcessTrusted()` 进行非侵入式权限查询，严禁无休止后台重试创建 `CGEventTap`，杜绝一切非用户主动触发的系统权限弹窗。
- **持久化 Application Bundle 封装**：
  * 在 `/Applications/Tetherly.app` 下部署标准化 macOS 应用程序包，配置固定 `CFBundleIdentifier`（`dev.tetherly.tetherly`）与 Ad-hoc 代码签名。
  * 彻底避免了因二进制重新编译导致的 TCC 权限哈希漂移，用户只需在系统设置中授权一次即可永久生效。
- **一键引导与自愈激活**：
  * Web 控制台与托盘菜单集成一键打开辅助功能偏好设置 API（`POST /api/input/accessibility/open`）。
  * 状态轮询时自动感知授权就绪态，热加载 `ensure_input_capture()`，无需重启守护进程。

### 3. 硬件位移增量驱动（Delta Roaming）与光标锁边

- **解决绝对坐标停滞根因**：
  ```rust
  let dx = CGEvent::integer_value_field(Some(cg_event), CGEventField::MouseEventDeltaX) as i32;
  let dy = CGEvent::integer_value_field(Some(cg_event), CGEventField::MouseEventDeltaY) as i32;
  ```
  在 macOS 事件捕获回调中，提取物理鼠标硬件位移增量 `dx, dy`，与物理绝对坐标一并封装为 `RawInput::Move { x, y, dx, dy }`。
- **远程席位位移累加引擎 (`crates/tetherly-core/src/input.rs`)**：
  在 `InputServer::local_move_delta` 中引入 `rem_x` 与 `rem_y` 虚拟逻辑坐标。当光标越过边缘处于远程受控状态（`CursorSeat::Remote`）时，持续累加硬件位移增量（按分辨率自适应归一化）：
  ```rust
  let delta_x = (dx as i64 * SCREEN_UNITS as i64 / self.width.max(1) as i64) as i32;
  let delta_y = (dy as i64 * SCREEN_UNITS as i64 / self.height.max(1) as i64) as i32;
  self.rem_x = (self.rem_x + step_x).clamp(0, SCREEN_UNITS);
  self.rem_y = (self.rem_y + step_y).clamp(0, SCREEN_UNITS);
  ```
  实现了 Mac 本地物理光标抑制锁边与 Windows 远端光标全屏平滑无阻漫游的完美结合。

### 4. 端口正规化与双端联通流收敛

- **规范独立端口分配**：
  UI: `45716`, Control: `45717`, File: `45718`, Input: `45719`, Screen: `45720`，彻底消除端口混用冲突。
- **隧道控制与数据通道联动**：
  `input_tunnel_attach` 成功建立元数据会话后，主动检查并拉通底层 `open_input` TCP 数据连接，保证 `self.shared.input_out` 始终处于就绪态。
- **网卡枚举缓存（5 秒 TTL）**：
  在 `crates/tetherly-node/src/lan.rs` 引入 `CACHED_REPORT`，解决高频 Web 轮询引发的内核系统调用风暴，Mac CPU 占用从 30% 降至 <0.1%。

---

## 验证与验收结果

1. **测试用例与工作区回归**：
   - `tetherly-core` 279 项单元测试 + 50 项解码/状态机测试全部通过。
   - `phase3` 8 项跨端输入/拓扑/四向漫游端到端集成测试全绿。
2. **生产机运行指标复核**：
   - **Mac 笔记本 (`192.168.1.4` / `10.144.144.100`)**：`/Applications/Tetherly.app` 运行稳定，顶部菜单栏正常渲染 `⇄ Tetherly`，CPU 占用稳定在 0.1% 左右。
   - **Windows 工作站 (`192.168.1.6` / `10.144.144.10`)**：`tetherly.exe` (PID 264812) 运行在 Session 2，任务栏通知区正常驻留，内存仅 18 MB，CPU 占用 0%，GPU 挖矿与网络代理完全零影响。
   - **数据链路连通性**：两机通过 EasyTier overlay 建立 TCP 45719 加密数据流，会话双向同步确认成功。
