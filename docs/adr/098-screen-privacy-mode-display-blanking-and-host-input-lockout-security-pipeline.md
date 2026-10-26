# ADR-098 屏幕串流隐私模式、物理显示黑屏遮罩与宿主机物理键鼠锁定安全防护流水线（Screen Streaming Privacy Mode, Display Blanking & Host Input Lockout Security Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-27，v2.12 / v0.1.34）**——实现屏幕串流远程控制会话下的端到端隐私防护与物理安全加固流水线（`ScreenPrivacyReport`、`ControlMsg::SetPrivacyMode`）；在屏幕控制通道引入专用安全信令（Envelope Tag = 5，14 字节定长线协议包头，携带 `enabled: bool` 与 `lock_input: bool` 编码至 64 位大端整数载荷）；在被控端（ScreenSender / Host）建立动态物理显示黑屏遮罩（Display Blanking / Virtual Overlay）与物理键鼠锁定（Host Input Lockout）机制，阻断现场旁观偷窥与本地键盘/鼠标物理误触；确立**故障安全断连恢复机制（Fail-Safe Disconnect Recovery）**：当远程连接断开、会话异常终止或守护进程退出时，原子强制解除黑屏遮罩与键鼠锁定，绝不造成宿主机永久失联锁死；全面集成至 `ScreenSender`、`ScreenReceiver`、`tetherly-node` 守护进程与回环 REST API（`GET /api/screen/privacy`、`POST /api/screen/privacy`、`POST /api/screen/privacy/enable`、`POST /api/screen/privacy/disable`）；实现 100% 严格全同构双端 CLI（`tetherly screen privacy`、`tetherly-cli screen privacy` 及 `enable` / `disable` / `status` 子命令与 `--lock-input` 参数）；新增核心单元测试、节点级测试与端到端自动化集成测试 `m5_22_screen_privacy_mode_and_input_lockout_pipeline` |
| 日期 | 2026-10-27 |
| 关联 | ADR-073「虚拟屏幕驱动与虚拟显示器创建（Virtual Display Driver）」、ADR-079「屏幕流有损编码协商」、ADR-086「屏幕流4D全自主协同自适应流水线」、ADR-088「硬件光标解耦与独立控制通道」、ADR-089「脏矩形检测与自适应屏幕静止抑制（Adaptive Squelch）」、ADR-097「屏幕串流多显示器枚举与虚拟显示器流复用及动态切换流水线」 |

## 动机

1. **远程协助与桌面管控中的现实威胁模型（Threat Model）**：
   - **肩窥与旁观偷窥（Shoulder Surfing / Physical Snooping）**：在远程办公、IT 运维、敏感数据处理或跨屏协助场景下，远程操作者（Visitor）正在操作宿主机（Host）。此时，宿主机物理显示器若处于正常点亮状态，办公室周围人员或路过人员可直接旁观屏幕上的敏感代码、财务凭据或个人隐私信息；
   - **本地物理键鼠冲突与恶意篡改（Physical Tampering & Input Conflicts）**：当远程操作者正在进行精密输入（如命令敲击、光标拖拽、重要配置保存）时，如果宿主机现场有人触碰物理键盘或滑动鼠标，会与远程输入事件产生严重的并发竞争、焦点夺取与意想不到的操作灾难。

2. **传统方案的弊端与死锁风险**：
   - 商业软件常用的方案要么严重依赖特定显卡驱动闭源扩展（兼容性极差），要么在远程网络波动断线时未能妥善解除物理锁定，导致宿主机现场人员屏幕全黑且键鼠失灵，只能被迫长按电源键强制关机，破坏未保存工作；
   - 必须在协议层、核心状态机层与网络层建立严格的**零信任安全控制**与**故障安全（Fail-Safe）保证**。

3. **双端 CLI 同构与现代控制面要求**：
   - 依据 Tetherly 的核心工程规范，所有核心控制能力必须在本地回环 REST API、SSE 遥测事件总线以及 `tetherly` / `tetherly-cli` 双端命令行中保持 100% 字节级同构，供脚本自动化、CI 流水线和桌面 GUI 统一无缝调用。

---

## 决定

1. **线协议与控制信令设计（`ControlMsg::SetPrivacyMode`）**：
   - 在既有的 14 字节定长屏幕控制信令信封（`TYPE_SCREEN_CONTROL = 0x0502`）中引入 Tag 5：
     - `magic: [u8; 4] = *b"TMV1"`：4 字节协议魔数；
     - `proto: u8 = 0x01`：协议版本；
     - `tag: u8 = 5`：`ControlMsg::SetPrivacyMode { enabled: bool, lock_input: bool }` 安全控制标签；
     - `payload: u64`：大端序 64 位无符号整数：
       - `payload & 0xFF`: `enabled: bool`（1 表示启用隐私遮罩，0 表示禁用）；
       - `(payload >> 8) & 0xFF`: `lock_input: bool`（1 表示锁定物理输入，0 表示解锁）。
   - 严格向下兼容 Tag 0 (`Start`)、Tag 1 (`Stop`)、Tag 2 (`Ack`)、Tag 3 (`KeyframeRequest`) 与 Tag 4 (`SwitchDisplay`)。

2. **核心元数据模型与状态机（`crates/tetherly-core/src/screen.rs`）**：
   - **`ScreenPrivacyReport`**：
     ```rust
     #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
     pub struct ScreenPrivacyReport {
         pub ok: bool,
         pub enabled: bool,
         pub lock_input: bool,
         pub blanking_method: String,
         pub last_change_us: u64,
     }
     ```
   - **`ScreenReceiver` / `ScreenSender` 状态协同**：
     - 维护 `privacy_enabled`、`privacy_lock_input`、`privacy_blanking_method`（默认为 `"virtual_overlay"`）与 `privacy_last_change_us`；
     - 提供 `privacy_report(&self) -> ScreenPrivacyReport` 与 `set_privacy_mode(&mut self, enabled: bool, lock_input: bool, now_us: u64) -> ScreenPrivacyReport`；
     - 构造器支持 `with_privacy(mut self, enabled: bool, lock_input: bool, method: impl Into<String>)`。

3. **故障安全断连恢复机制（Fail-Safe Disconnect Recovery）**：
   - 在 `ScreenReceiver::on_peer_gone()` 中：
     ```rust
     pub fn on_peer_gone(&mut self) {
         self.last_cursor_hash = 0;
         self.cursor_frames_received = 0;
         self.last_peer_rtt_us = 0;
         // Fail-safe Disconnect Recovery: Automatically disengage privacy mode and unlock input
         self.privacy_enabled = false;
         self.privacy_lock_input = false;
         self.playout_buffer.reset();
     }
     ```
   - 在宿主机服务循环 `attach_screen_server` 及客户端循环 `attach_screen_client` 的退出清理阶段（无论因对端关闭、网络超时或错误中断），统一调用 `screen_set_privacy_mode(false, false)`，确保物理屏幕遮罩瞬间解除、物理输入立即恢复。

4. **节点运行时系统与状态集成（`crates/tetherly-node/src/runtime.rs`）**：
   - `NodeShared` 新增原子变量与并发守卫：
     - `screen_privacy_enabled: AtomicBool`（默认 `false`）
     - `screen_privacy_lock_input: AtomicBool`（默认 `false`）
     - `screen_privacy_blanking_method: Mutex<String>`（默认 `"virtual_overlay"`）
     - `screen_privacy_last_change_us: AtomicU64`（默认 `0`）
   - `ScreenStatusReport` 扩充遥测字段：
     - `privacy_enabled: bool`
     - `privacy_input_locked: bool`
     - `privacy_blanking_method: String`
   - 暴露公共 API：
     - `pub fn screen_privacy_report(&self) -> ScreenPrivacyReport`
     - `pub fn screen_set_privacy_mode(&self, enabled: bool, lock_input: bool) -> Result<ScreenPrivacyReport, CoreError>`
   - 在 `attach_screen_server` 控制通道循环中监听 `ControlMsg::SetPrivacyMode { enabled, lock_input }`，实时响应对端发来的隐私控制指令。

5. **回环 REST 控制面 API（`crates/tetherly-node/src/uihttp.rs`）**：
   - `GET /api/screen/privacy`：返回当前隐私模式状态 `ScreenPrivacyReport`；
   - `POST /api/screen/privacy`：接收 JSON 请求体（`{"enabled": true, "lock_input": true}`）或 Query 参数（`?enabled=true&lock_input=true`）；
   - `POST /api/screen/privacy/enable`：便捷端点，支持可选 `?lock_input=true`；
   - `POST /api/screen/privacy/disable`：便捷端点，一键解除遮罩与输入锁定；
   - 严格进行 `127.0.0.1` 本地回环检查与 `Host` 头安全校验，防御外部 DNS 重绑定与非法跨站请求。

6. **100% 严格双端 CLI 同构（`crates/tetherly-node/src/bin/tetherly.rs` 与 `bins/tetherly-cli/src/main.rs`）**：
   - 语法定义：
     ```bash
     tetherly screen privacy [status] [--json] [--ui-port <port>]
     tetherly screen privacy enable [--lock-input] [--json] [--ui-port <port>]
     tetherly screen privacy disable [--json] [--ui-port <port>]
     tetherly screen privacy [--enable] [--disable] [--lock-input] [--json] [--ui-port <port>]
     ```
   - `tetherly-cli` 提供完全一致的参数、帮助说明、格式化终端高亮及 `--json` 原始序列化输出。

---

## 验证与测试

1. **核心协议单元测试**（`crates/tetherly-core/src/screen.rs`）：
   - `screen_privacy_mode_and_input_lockout_pipeline`：验证 Tag 5 线协议编码与解码、状态报告获取、动态启闭切换，以及在 `on_peer_gone()` 触发时的故障安全自动复位机制。
2. **REST API 控制面单元测试**（`crates/tetherly-node/src/uihttp.rs`）：
   - `test_uihttp_screen_privacy_endpoints`：验证 `GET /api/screen/privacy`、`POST /api/screen/privacy`（JSON 与 QueryString）、`/enable` 与 `/disable` 端点、400 畸形参数拦截与 403 跨源 Host 头拦截。
3. **端到端 Phase 5 集成测试**（`tests/phase5.rs`）：
   - `m5_22_screen_privacy_mode_and_input_lockout_pipeline`：两节点 Noise 握手建立连接 -> 开启屏幕串流 -> 验证初态 -> 调用 REST 启用隐私模式与输入锁定 -> 验证串流帧正常解码传输 -> 验证状态遥测上报 -> 调用 `/disable` 与 `/enable` -> 停止屏幕会话并断开 -> 严格断言故障安全机制生效（宿主机自动复位为 `enabled=false`、`lock_input=false`）。
