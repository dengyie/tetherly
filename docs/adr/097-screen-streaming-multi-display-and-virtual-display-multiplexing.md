# ADR-097 屏幕串流多显示器枚举与虚拟显示器流复用及动态切换流水线（Screen Streaming Multi-Display & Virtual Display Stream Multiplexing & Dynamic Switch Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-26，v2.11 / v0.1.33）**——实现面向多物理显示器与虚拟扩展屏（ADR-073 Virtual Display）的动态多屏枚举、流复用与零重连动态切换流水线（`ScreenDisplayInfo`、`ScreenDisplayReport`、`ControlMsg::SwitchDisplay`）；在屏幕控制通道引入专用切换信令（Envelope Tag = 4，14 字节定长线协议包头，携带目标 `display_id: u8` 映射至 64 位大端整数）；在被控端（ScreenSender / Host）建立动态拓扑感知机制，捕获源在接收到切换信令或 API 指令后无缝切换捕获目标（GDI/DXGI/Virtual Desktop），并联动自适应静止抑制器（ADR-089 Squelch Controller）强制唤醒并立即注入强制关键帧（IDR Keyframe Request），确保访客端（Visitor/Decoder）在不重建 Noise 加密会话、不断开 UDP 传输套接字、不中断远程控制通道的前提下实现亚帧级瞬时切屏呈现；全面集成至 `ScreenSender`、`ScreenReceiver`、`tetherly-node` 守护进程与回环 REST API（`GET /api/screen/displays` 与 `POST /api/screen/switch`）；实现 100% 严格全同构双端 CLI（`tetherly screen displays`、`tetherly screen switch <id>`、`tetherly-cli screen displays`、`tetherly-cli screen switch <id>`、`--screen-display <id>` 启动参数及 `screen status` 状态遥测）；新增核心单元测试、节点级测试与端到端自动化集成测试 `m5_21_screen_multi_display_and_dynamic_switch_pipeline` |
| 日期 | 2026-10-26 |
| 关联 | ADR-073「虚拟屏幕驱动与虚拟显示器创建（Virtual Display Driver）」、ADR-079「屏幕流有损编码协商」、ADR-086「屏幕流4D全自主协同自适应流水线」、ADR-088「硬件光标解耦与独立控制通道」、ADR-089「脏矩形检测与自适应屏幕静止抑制（Adaptive Squelch）」、ADR-095「屏幕流带宽主动探测与链路容量快速爬升流水线」、ADR-096「屏幕与音频播放音画同步与硬件时钟漂移补偿流水线」 |

## 动机

1. **解决现代工作站多物理显示器与虚拟扩展屏（Virtual Displays）环境下的远程流串流痛点**：
   - 现代开发与办公工作站普遍配备双屏或三屏物理显示器；同时，借由 ADR-073「虚拟屏幕驱动（Virtual Display Driver）」，Tetherly 支持在宿主机上虚拟出任意分辨率和刷新率的高清拓展屏（如 1920x1200、2560x1600 等）；
   - 在旧有架构中，屏幕捕获源默认只能固定捕获主显示器（Primary Display / Display ID 0）。当远控访客需要查看辅助屏幕或将工作窗口拖拽至虚拟拓展屏协同操作时，缺乏动态切屏与多屏枚举机制；
   - 若通过“断开当前远程会话 -> 重新启动远控并指定屏幕”的传统做法，会导致 Noise 会话密钥重新协商、UDP 传输通道重连、音视频对齐基准丢失以及严重的画面卡顿与黑屏闪烁，完全违背了超低时延、无缝协同的设计哲学。

2. **零中断动态切屏与关键帧主动注入（Zero-Downtime Dynamic Switch Pipeline）**：
   - 动态切屏必须在既有的加密传输通道与串流链路内部完成。当切换目标显示器时，捕获源尺寸（宽、高、像素格式、DPI 缩放比例）可能发生突变（例如从 1920x1080 切换至 2560x1440 或 4K）；
   - 画面尺寸或显示器上下文切换后，编码器如果继续发送依赖上一帧参考的 P 帧（Predicted Frame），接收端解码器将产生严重的像素花屏、色彩错位与撕裂伪影。因此，切换流水线必须联动编码器与静止抑制器（Squelch Filter），在切屏发生的瞬间强制产生 IDR 关键帧（Instantaneous Decoder Refresh），强制重置解码参考基准。

3. **双向拓扑同步与显示器元数据标准化（Display Topology Discovery）**：
   - 串流发起端与接收端需要统一的显示器元数据模型（`ScreenDisplayInfo`），包含：显示器 ID（`display_id: u8`）、人类可读名称（如 `Display 0 (Primary)`、`Virtual Display (ADR-073)`）、分辨率宽度与高度、DPI 缩放比例（`scale_factor_pct`）、主显示器标识（`is_primary`）以及是否为虚拟扩展显示器（`is_virtual`）；
   - 访客端可以通过控制信令即时获取主机的显示器拓扑结构，并按需切换；主机管理员也可以通过本地 CLI 或 REST API 动态调整串流目标。

4. **100% 严格双端命令行同构与企业级遥测支持（Dual-CLI Parity）**：
   - 秉持 Dual-CLI 架构准则，`tetherly`（节点守护进程）与 `tetherly-cli`（远控客户端）提供参数命名、终端人机交互展示、JSON 遥测结构全同构的子命令：`screen displays` 与 `screen switch <display_id>`，以及启动参数 `--screen-display <display_id>`。

---

## 决定

1. **线协议与控制信令设计（`ControlMsg::SwitchDisplay`）**：
   - 在既有的 14 字节定长屏幕控制信令信封（`TYPE_SCREEN_CONTROL = 0x0502`）中引入 Tag 4：
     - `magic: [u8; 4] = *b"TMV1"`：4 字节协议魔数；
     - `proto: u8 = 0x01`：协议版本；
     - `tag: u8 = 4`：`ControlMsg::SwitchDisplay` 控制标签；
     - `payload: u64`：大端序 64 位无符号整数，低 8 位存放目标 `display_id: u8`。
   - 严格向下兼容 Tag 0 (`Start`)、Tag 1 (`Stop`)、Tag 2 (`Ack`) 与 Tag 3 (`KeyframeRequest`)。

2. **核心元数据模型与拓扑抽象（`crates/tetherly-core/src/screen.rs`）**：
   - **`ScreenDisplayInfo`**：
     ```rust
     pub struct ScreenDisplayInfo {
         pub display_id: u8,
         pub title: String,
         pub width: u32,
         pub height: u32,
         pub scale_factor_pct: u32,
         pub is_primary: bool,
         pub is_virtual: bool,
     }
     ```
   - **`ScreenDisplayReport`**：
     ```rust
     pub struct ScreenDisplayReport {
         pub ok: bool,
         pub active_display_id: u8,
         pub displays: Vec<ScreenDisplayInfo>,
         pub switches_count: u64,
         pub last_switch_us: u64,
     }
     ```
   - **`ScreenSource` Trait 扩充**：
     为支持多显示器枚举与动态切换，`ScreenSource` 特征补充：
     - `fn displays(&self) -> Vec<ScreenDisplayInfo>`：枚举当前系统所有物理和虚拟屏幕；
     - `fn active_display_id(&self) -> u8`：获取当前捕获中的显示器 ID；
     - `fn switch_display(&self, display_id: u8) -> Result<(), CoreError>`：将捕获硬件指针切换至目标屏幕。

3. **发送端与接收端状态机协同（`ScreenSender` / `ScreenReceiver`）**：
   - **`ScreenSender`**：
     - 维护 `active_display_id`、`displays`、`display_switches_count` 与 `last_switch_us`；
     - 当收到 `ControlMsg::SwitchDisplay { display_id }` 或直接调用 `switch_display(display_id, now_us)` 时：
       1. 验证目标 `display_id` 是否存在于拓扑列表中；
       2. 更新 `active_display_id`，递增 `display_switches_count`，记录切换时间戳；
       3. 联动 ADR-089 静止抑制器执行 `squelch_controller.force_wake(now_us)`，避免因新屏幕静止而被抑制发包；
       4. 强制请求编码器注入 IDR 关键帧（`self.stats.keyframes_requested += 1`）。
   - **`ScreenReceiver`**：
     - 维护远端显示器拓扑镜像与当前活跃屏幕标识；
     - 当切换信令生效后，将随流到来的新分辨率与关键帧无缝提交至解码与渲染管道。

4. **全链路运行时整合（`crates/tetherly-node/src/runtime.rs`）**：
   - `NodeConfig` 新增 `screen_display_id: u8`（默认 0）；
   - `Node` 内部维护原子状态：`screen_active_display_id: AtomicU8`、`screen_display_switches_count: AtomicU64`、`screen_last_display_switch_us: AtomicU64` 及 `screen_displays: Mutex<Vec<ScreenDisplayInfo>>`；
   - 在 `attach_screen_server` 的控制流事件循环中，监听来自对端的 `ControlMsg::SwitchDisplay`，联动 `screen_source.switch_display(display_id)` 并刷新状态；
   - 暴露公共运行时方法：`screen_active_display_id()`、`screen_displays()`、`screen_set_displays()`、`screen_display_switches_count()`、`screen_display_report()` 与异步切换函数 `screen_switch_display(display_id).await`。

5. **双端命令行与 REST 接口同构（Dual-CLI & UI HTTP API）**：
   - **HTTP 接口**：
     - `GET /api/screen/displays`：返回 `ScreenDisplayReport`；
     - `POST /api/screen/switch`：支持 JSON 请求体 `{"display_id": 1}` 或 Query 参数 `?display_id=1`，执行动态切换并返回最新的 `ScreenDisplayReport`；若目标 ID 不存在返回 404 Not Found；
     - `GET /api/screen`：状态遥测新增 `active_display_id`、`display_count`、`display_switches_count`。
   - **CLI 命令**：
     - `tetherly screen displays` / `tetherly-cli screen displays`：表格化枚举屏幕列表；
     - `tetherly screen switch <display_id>` / `tetherly-cli screen switch <display_id>`：即时切换屏幕；
     - 启动参数：`tetherly run --screen-display <id>`；
     - 遥测展示：在 `screen status` 中输出 `Display: #0 (total: 2, switches: 1)`。

---

## 验证与效果

1. **核心单元测试覆盖**：
   - `crates/tetherly-core/src/screen.rs::screen_multi_display_and_dynamic_switch_pipeline`：验证 `ControlMsg::SwitchDisplay` 编解码信封、`ScreenSender` 切换与 IDR 关键帧强制注入、`ScreenReceiver` 拓扑镜像及 PeerGone 状态清理；
   - `crates/tetherly-node/src/uihttp.rs::test_uihttp_screen_display_and_switch_endpoints`：验证 HTTP GET 与 POST 路由解析、Query 参数与 JSON 请求体兼容性、404 未知屏幕容错及 Host 头安全校验。

2. **端到端集成测试覆盖**：
   - `tests/phase5.rs::m5_21_screen_multi_display_and_dynamic_switch_pipeline`：测试两个真实配对节点在 Noise 加密会话与屏幕串流运行态下，执行多物理屏及 ADR-073 虚拟扩展屏的动态无缝切换，验证画面接收不中断且状态统计精准递增。
