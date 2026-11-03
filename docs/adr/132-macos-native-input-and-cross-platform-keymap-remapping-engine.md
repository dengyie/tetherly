# ADR-132 macOS 原生键鼠捕获/注入体系与跨平台键位重映射引擎（对齐并替代 Deskflow）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-11-02，v2.46 / v0.1.68）**——在 macOS 原生接入 CoreGraphics `CGEventTap` 全局捕获与 `CGEventPost` 全局模拟注入，构建 `tetherly-core::keymap` 跨平台协议层键位重映射引擎，打通光标边缘漫游、按键本地抑制、自注入防回环与双端 CLI/REST 策略同构，彻底具备淘汰 Deskflow 的完整能力。 |
| 日期 | 2026-11-02 |
| 关联 | ADR-071（输入通道安全策略与物理锁定）、ADR-072（跨端屏幕边缘漫游与光标状态机）、ADR-131（上一轮） |

---

## 动机与背景

在生产环境（macOS 笔记本与 Windows 工作站/挖矿主机）中，用户此前长期依赖 Deskflow（原 Barrier / Synergy）进行跨机器鼠标键盘共享与修饰键互换。虽然 Tetherly 在 Phase 3 中已经实现了基于 Noise_IK 加密的 45719 二进制输入线协议（`TIN1`）、剪贴板剪贴漫游与基于拓扑图的光标边缘穿越状态机，但在实际落地替代 Deskflow 时存在三大关键卡点：

1. **macOS 端输入捕获（Capture）缺失**：
   此前低级系统输入捕获仅在 Windows 侧实现了 `SetWindowsHookExW`；在 macOS 侧，`start_input_hooks` 为未实现的报错占位符。这导致用户无法以 Mac 作为主控输入设备将键盘鼠标平滑漫游至 Windows。
2. **macOS 端输入注入（Injection）为 No-op 空实现**：
   `SystemInputSink` 在非 Windows 平台仅提供空方法。这导致以 Windows 为主控漫游至 Mac 时，Mac 端无法合成物理光标位移与真实击键。
3. **跨平台修饰键习惯严重冲突（Modifier Mapping Disparity）**：
   macOS 与 Windows 在键盘硬件与操作系统习惯上存在结构性差异：
   - Mac 用户高度依赖左下角内侧的 `Command` 键（`Cmd+C`, `Cmd+V`, `Cmd+Z`, `Cmd+W`）；
   - Windows 则依赖最外侧的 `Control` 键（`Ctrl+C`, `Ctrl+V`, `Ctrl+Z`, `Ctrl+W`）；
   - Mac 的 `Option` 键对应 Windows 的 `Alt` 键；
   - 若不提供类似 Deskflow 的修饰键动态映射引擎，Mac 键盘在操控 Windows 时，用户必须强行改变肌肉记忆去按物理 `Control` 键，操作体验严重劣化。

为此，必须在保持生产机非侵入、零编译负载、零凭证泄露的约束下，在 Tetherly 内部原生补齐 macOS 键鼠全局捕获与模拟注入，并实现纯协议层的跨平台键位重映射引擎。

---

## 架构与技术实现

### 1. 协议级无 I/O 键位映射引擎 (`crates/tetherly-core/src/keymap.rs`)

遵循 Tetherly 的核心设计原则（`tetherly-core` 禁止任何 unsafe 代码与平台特定条件编译），键位映射引擎作为无副作用、可测试的纯算子实现：
- **线协议基准标准化**：Tetherly 的 `TIN1` 虚拟按键在线协议统一采用 Win32 Virtual-Key（VK，`u16`）标准。
- **修饰键抽象与位掩码**：
  定义 `enum ModifierKey { Control, Alt, Meta, Shift }` 以及对应的 `KeySide { Left, Right, Both }`。定义协议级 `MOD_CONTROL`, `MOD_ALT`, `MOD_META`, `MOD_SHIFT` 掩码常量。
- **预设策略模版与解析**：
  ```rust
  pub struct KeyMapProfile {
      pub name: String,
      pub modifiers: HashMap<ModifierKey, ModifierKey>,
      pub custom_keys: HashMap<u16, u16>,
  }
  ```
  - `mac_to_windows()`：`Meta (Command) ↔ Control`、`Control ↔ Meta (Win)`、`Alt (Option) ↔ Alt`。实现 Mac 键盘控制 Windows 时肌肉记忆完全一致（Mac 上的 `Cmd+C` 映射为 Windows 上的 `Ctrl+C` 发送）。
  - `windows_to_mac()`：`Control ↔ Meta (Command)`、`Meta ↔ Control`、`Alt ↔ Option`。实现 Windows 键盘控制 Mac 时的对称重映射。
  - `identity()`：原样透传。
  - `auto()`：编译宿主为 macOS 时自动选用 `mac_to_windows`，为 Windows 时自动选用 `windows_to_mac`。
- **映射变换算法**：
  - `remap_vk(&self, code: u16) -> u16`：优先查找 `custom_keys`，若无则查询修饰键映射表（区分左右修饰键并保持其左右侧物理性一致），非修饰键原样返回。
  - `remap_mods(&self, mods: u8) -> u8`：按照修饰键表重新组合 bitmask 掩码。

### 2. macOS 底层键鼠捕获体系 (`crates/tetherly-node/src/platform.rs`)

- **Win32 VK ↔ macOS CGKeyCode 双向无损转换**：
  基于 CoreGraphics 硬件键码标准，建立详尽的双向映射表（`mac_keycode_to_win_vk` 与 `win_vk_to_mac_keycode`），全面覆盖 ANSI 字母（A-Z）、数字键（0-9）、功能键（F1-F12）、方向键（Left/Right/Up/Down）、编辑键（Enter, Backspace, Space, Tab, Escape, Delete, Home, End, PageUp, PageDown）以及完整的修饰键集合（L/R Shift, L/R Control, L/R Option, L/R Command/Win）。
- **`CGEventTap` 全局事件拦截**：
  - 调用 `CGEventTapCreate(kCGHIDEventTap, kCGHeadInsertEventTap, kCGEventTapOptionDefault, ...)`。
  - 掩码订阅：`MouseMoved`, `LeftMouseDown`, `LeftMouseUp`, `RightMouseDown`, `RightMouseUp`, `OtherMouseDown`, `OtherMouseUp`, `ScrollWheel`, `KeyDown`, `KeyUp`, `FlagsChanged`。
- **防自注入环路保护（Loopback Guard）**：
  本地合成并注入的事件会由系统分发，若不做甄别，全局 Tap 会再次捕获这些事件造成无限事件风暴。
  在注入事件时，统一写入专用签名：
  ```rust
  const TETHERLY_INJECTED_MAGIC: i64 = 0x5445544845524C59; // "TETHERLY"
  CGEvent::set_integer_value_field(Some(&event), CGEventField::EventSourceUserData, TETHERLY_INJECTED_MAGIC);
  ```
  在 Tap 回调中读取 `EventSourceUserData`，若匹配该魔数则直接返回原始指针放行，不向上层通道投递。
- **远程漫游按键本地抑制（Remote Seat Keyboard Suppression）**：
  与 Windows 侧 `HookControl` 机制对齐：
  - 本地座位（`CursorSeat::Local`）：`keyboard_armed == false`，键盘事件不截断、不监听，保护日常敲击隐私；
  - 远程座位（`CursorSeat::Remote`）：`keyboard_armed == true`，光标已越界漫游至远端屏幕，拦截键盘按键并向对端通道发送，同时 Tap 回调直接返回 `NULL` 阻止 macOS 本地分发，使得敲击仅作用于被控机。
- **超时异常自愈机制**：
  macOS 在应用或线程繁忙时可能会产生 `kCGEventTapDisabledByTimeout` 从而静默关闭 Tap。回调函数实时监听该事件类型，一旦捕获立即调用 `CGEventTapEnable(tap, true)` 实现毫秒级原地复活。
- **跨线程非 Send 类型安全隔离**：
  `CFMachPort` 内部包含非线程安全的 raw pointer，无法直接跨线程传递。我们在专用的 `tetherly-input-tap` 线程闭包内本地构造 `Box<MacTapContext>` 并直接接入当前线程的 `CFRunLoop`。外部返回给 Node 运行时的 `HookControl` 仅包装纯线程安全的 `Arc<AtomicBool>`，从类型系统根源上规避多线程并发违规。
- **辅助功能权限探测**：
  封装 `AXIsProcessTrusted()`，若无系统授权则返回明确友好的引导指引，避免黑盒崩溃。

### 3. macOS 底层事件模拟注入体系 (`SystemInputSink`)

- **光标位移合成 (`move_abs`)**：
  读取主显示器绝对物理分辨率，将 Tetherly 规范的 `0..=1000` 虚拟百分比坐标线性换算为当前屏幕的全局像素点，调用 `CGEvent::new_mouse_event(None, CGEventType::MouseMoved, pt, CGMouseButton::Left)`，打上魔数标记后通过 `CGEvent::post(CGEventTapLocation::HID, Some(&event))` 提交。
- **鼠标按键合成 (`button`)**：
  解析按键掩码（`BTN_LEFT`, `BTN_RIGHT`, `BTN_MIDDLE`），分别生成对应的 Down/Up 鼠标事件并投递。
- **滚轮合成 (`wheel`)**：
  调用 `CGEvent::new_scroll_wheel_event2(None, CGScrollEventUnit::Pixel, wheel_count, dy as i32, dx as i32, 0)` 支持水平与垂直高精度平滑滚动。
- **按键合成 (`key`)**：
  将传入的 Win32 VK 转为 macOS `CGKeyCode`，调用 `CGEvent::new_keyboard_event`，根据修饰键掩码合成 `CGEventFlags`，投递至 HID 队列。

### 4. 控制面、持久化策略与双端 CLI 全同构

- **策略存储**：
  在 `InputTunnelPolicy` 中新增 `keymap: Option<String>`，支持 `auto`、`mac_to_windows`、`windows_to_mac`、`identity` 等字符串配置，持久化落盘至 `input_policy.json`。
- **REST 路由适配**：
  `/api/input/policy`（GET/POST）新增入参和出参字段 `keymap`。
- **CLI 命令行全同构**：
  - `tetherly input policy --keymap mac-to-windows`（支持 `crates/tetherly-node/src/bin/tetherly` 与 `bins/tetherly-cli`）；
  - `input policy` 查看输出统一格式化打印 `Keymap Profile: {keymap}`。

---

## 验证与验收

1. **协议层与映射逻辑验证**：
   - `crates/tetherly-core/src/keymap.rs` 包含 6 项单元测试全部通过：
     - `mac_to_windows_maps_meta_to_control`：验证 Command 映射为 Control，且左右修饰键侧别保持；
     - `windows_to_mac_maps_control_to_meta`：验证 Control 映射为 Command；
     - `identity_preserves_keys_and_mods`：验证透传无损；
     - `custom_keys_override_modifiers`：验证自定义单键映射高优先级覆盖；
     - `preset_parsing` & `serde_roundtrip`：验证配置文件序列化与反序列化。
2. **硬件键码双向转换验证**：
   - `crates/tetherly-node/src/platform.rs` 新增 `mac_win_keycodes_bidirectional` 单元测试，全面检验 A-Z、0-9、F1-F12、方向键、常用控制键与修饰键的无损双向转换。
3. **跨端端到端集成测试**：
   - `tests/phase3.rs` 新增 `m3_4_input_keymap_remapping`：
     - 启动两节点建立 45719 加密输入通道；
     - 配置服务端策略 `keymap = Some("mac_to_windows")`；
     - 触发边缘跨屏进入 `Remote` 座位；
     - 注入 `VK_LWIN`（Mac Command 键），断言接收端 `InputSink` 成功收到被重映射为 `VK_LCONTROL` 的按键事件。
4. **全工程质量闸门**：
   - `cargo fmt --all -- --check`：全工程格式零偏差；
   - `cargo clippy --workspace --all-targets -- -D warnings`：零警告通过；
   - `cargo test --workspace --locked`：全工作区测试套件（含 core、crypto、net、loopback、node、cli 及 phase1~phase5 集成测试）全绿。
