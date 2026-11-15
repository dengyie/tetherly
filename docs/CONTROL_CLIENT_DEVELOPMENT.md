# Tetherly 控制客户端开发文档

| 项 | 值 |
|---|---|
| 状态 | 设计稿（已完成调研，尚未实现）。决策见 [ADR-139](adr/139-control-client-architecture.md) |
| 日期 | 2026-11-15 |
| 基线版本 | v0.2.55 |
| 取代 | `ui/index.html`（ADR 前的第一阶段 loopback 控制台）。也改写 `ui/README.md` 中"下一步 Tauri/React 外壳"的旧计划 |
| 关联 | ADR-133（四向拓扑）、ADR-134（托盘）、ADR-137（托盘弹出面板 / 文件拖放）、ADR-138（远程桌面），DEVELOPMENT §126 |

---

## 0. 速读

- **后端 = daemon，前端 = 独立客户端，二者只通过一份有版本、有类型的契约通信。** 契约包括 `/api/v1` 门面和一条事件流。前端不再直接调用约 550 条历史路由。
- **轮询改为推送。** 新增 `GET /api/v1/events` 事件流：先发一次完整快照，再发增量事件，每条事件带递增序号 `seq`，断线后用 `Last-Event-ID` 续接。现有 50 个 `setInterval` 和托盘面板的每秒拉取全部删掉。
- **桌面端（macOS / Windows）用 Tauri 2 + Svelte 5 + TypeScript。** 应用以托盘或菜单栏为入口，主窗口按需打开。token 只保存在 Tauri 的 Rust 侧，网页代码拿不到。
- **daemon 继续内置同一份前端的 Web 构建版本**，作为浏览器兜底入口和无界面主机的入口，替换 `ui/index.html`。前端通过 `Transport` 接口适配两种运行环境，业务代码只有一份。
- **Android 保留 Kotlin 原生应用**，采用相同的信息架构，数据类型从同一份契约自动生成。
- **信息架构按"设备优先"组织**（参考 KDE Connect）：概览 / 设备 / 布局 / 传输 / 动态 / 设置 / 诊断。屏幕诊断、基准测试这类工程向功能放进"诊断"页并默认折叠。
- **客户端不管理 daemon 的生命周期**（吸取 Deskflow CoreProcess 的教训）。客户端崩溃或关闭都不影响键鼠共享。
- **分 6 个阶段（M0–M5）迁移。** 每个阶段都能单独发布，而且现有控制台在 M4 之前一直可用。

---

## 1. 现状盘点（问题清单）

| 维度 | 现状 | 问题 |
|---|---|---|
| 前端形态 | `ui/index.html` 单文件，10,226 行，543 KB，通过 `include_str!` 编进 daemon | 无法模块化、做类型检查或单元测试，任何改动都要重新编译 daemon |
| 页面结构 | 约 30 个 `<section>` 纵向堆叠（handoff、media、battery、locate、action、focus、topology、input-tunnel、virtual-display、audio、camera、mic、bench、profiles、messaging、audit…），标题中英混排 | 没有导航和层级，日常功能与工程诊断混在一起，找一个开关要滚很久 |
| 数据刷新 | 50 个 `setInterval`，间隔 2–3 秒；托盘面板（`platform/popover.rs`）每秒拉取 `/api/peers` 和 `/api/file/transfers` | 空闲时也持续占用 CPU 和锁；状态最多滞后 3 秒；同一份数据被多处重复拉取 |
| API | `uihttp.rs` 7,159 行，加上 `uihttp/api_*.rs` 共约 2 万行，约 555 个路由项。别名大量重复：`/api/msg`·`/api/message`·`/api/messages`，`/api/file/drag/*`·`/api/files/drag/*`，`/api/inbox`·`/api/file/inbox`，`/api/mic`·`/api/microphone`；`status`/`state`/`policy` 的命名也不统一 | 没有版本号和类型契约，前端只能靠猜字段，后端也不敢删任何路由 |
| 事件 | `events.rs` 中 `UiEvent` 有 96 个变体（`#[serde(tag="kind")]`），通过 `/api/events` SSE 推送，每 15 秒发一次心跳 | 已经是很好的基础，但缺少初始快照、序号和续传机制，前端几乎没有用上 |
| 鉴权 | 写操作需要 `X-Tetherly-Token` 或 `Authorization: Bearer`，token 通过 URL fragment `#token=` 传给控制台；会校验 Origin 和 Host（`origin_allowed` 只放行 `127.0.0.1/localhost/[::1]:<ui_port>`） | 读接口不鉴权；独立客户端（Tauri 的 origin 是 `tauri://localhost` 或 `http://tauri.localhost`）会被 Origin 校验拒绝；Windows daemon 以 SYSTEM 身份运行，普通用户进程读不到 `api.token` |
| 托盘 | daemon 进程内运行原生托盘：macOS 用 `NSStatusBar`（`platform/tray.rs`），Windows 用 `Shell_NotifyIconW`；菜单里的状态文字是写死的（如"🟢 运行中"） | 托盘显示的不是真实状态（与 Deskflow #9592 是同类问题），双击只能打开浏览器 |
| Android | `MainActivity.kt` 76 行，手写 `LinearLayout`：地址、PIN、两个按钮，外加一个每秒轮询的状态行 | 只是"配对表单"，没有设备、传输或通知视图 |

**结论**：后端能力已经很完整，缺的是契约层、推送层和一个有信息架构的前端。这次改造主要做重组和收敛，不是新增功能。

---

## 2. 调研结论

### 2.1 参考项目对照

| 项目 | 技术栈 | GUI ↔ 核心通信 | 状态获取 | 值得借鉴 | 需要避开 |
|---|---|---|---|---|---|
| **Deskflow** | Qt 6 Widgets / C++ | `QLocalSocket`（Unix 套接字或命名管道），先握手校验版本；"Service"模式下由守护进程拉起核心 | 推送字符串命令（`connectionState`、`peerFingerprint`…），部分仍靠解析日志 | `ComputerSetupView` 网格布局编辑器（拖入新电脑、拖出删除、双击打开属性）；指纹对比对话框默认焦点在"否"；状态栏显示加密状态；`closeToTray` | GUI 管理核心进程的生命周期，引发 #10179、#10256、#10276；模态框阻塞 IPC 读取；日志控件重绘导致 CPU 100%（#10367）；托盘不显示连接状态（#9592） |
| **Input Leap / Barrier** | Qt Widgets | localhost TCP | 用正则解析日志（源码注释写着 "makes Andrew cry"） | Zeroconf 自动发现、首次运行向导 | 靠解析日志获取状态，这是最差的方案 |
| **PowerToys MWB** | WinUI 3 | StreamJsonRpc 走命名管道，ACL 只允许当前用户 SID | 每 2 秒轮询一次 | 名称加密钥配对；最多 4 台设备的拖拽矩阵，可切换 1×4 和 2×2；UI 里直接提供故障排除操作（添加防火墙规则、重连） | 只用颜色表示状态，无障碍很差；轮询；拖放需要管理员权限 |
| **RustDesk** | Flutter + Rust（flutter_rust_bridge 1.80） | `parity_tokio_ipc`，用一个 serde 大枚举传消息，并校验对端 uid 和可执行文件 | 每秒轮询一次 | 设备卡片配合"最近 / 收藏 / 已发现"分页；独立的"连接管理"窗口，按权限单独开关（键盘、剪贴板、文件） | 桌面和移动端是两棵独立的页面树，功能容易不一致；fork 了大量插件；配置是无类型的字符串字典 |
| **LocalSend** | Flutter | 不需要（应用本身就是服务端） | 应用内状态 | 只有三个页签（接收 / 发送 / 设置）；按宽度断点在 `NavigationRail` 和底部导航之间切换，所有平台共用一棵组件树；内置故障排查页 | 没有常驻进程，不适合键鼠共享 |
| **KDE Connect / GSConnect** | Kirigami QML / GJS | D-Bus，daemon 加多个瘦客户端（应用、托盘、设置面板） | 信号推送 | **设备优先，能力按设备分组**：只有两端都支持的能力才显示对应操作；配对状态机写得很明确（未配对 → 已请求并显示验证码 → 已配对 / 不可达） | — |
| **Tailscale** | Go daemon + 原生 GUI | LocalAPI = HTTP（Unix 套接字或命名管道；macOS 上用 localhost 加 token 文件），读和写分开授权，客户端发送版本头 | `watch-ipn-bus`：同一条长连接先发快照再发增量，断线重连时重新发快照 | **快照加增量的事件流协议**；daemon 另外提供一份 Web UI 给无界面主机；托盘根据事件流重建菜单 | Linux 托盘不稳定，不得不加一个"重建菜单"选项 |

### 2.2 横向经验（作为设计原则）

1. **用推送，不用轮询，也不解析日志。** 事件流采用 Tailscale 的契约：先发快照，再发增量，带序号，断线后重新同步。
2. **GUI 不管理 daemon。** 客户端可以随时关闭、崩溃或替换，键鼠共享不受影响。daemon 的启动和自启动由服务管理器（launchd / Windows 服务）负责。
3. **设备优先，能力按设备分组。** 每台设备有明确的状态机，能力按钮只在两端都支持时显示。
4. **信任交互要克制。** 配对时并排显示验证码，默认按钮是"拒绝"；本机随时可以查看自己的指纹。
5. **布局编辑保持简单。** 用网格加拖拽，并提供键盘操作作为替代；不能只用颜色表达状态。
6. **故障排查做进产品。** 诊断页提供可直接执行的操作，例如端口检测、防火墙规则、重新发现设备。
7. **托盘显示真实状态，而不只是一份菜单。**

### 2.3 技术选型比较

| | Tauri 2 + Web 前端 | Flutter | Qt 6 | 纯 Web（daemon 托管） |
|---|---|---|---|---|
| 体积 | 约 3–10 MB（使用系统 WebView） | 约 20–40 MB | 约 30–60 MB | 不额外增加 |
| 托盘 / 菜单栏 | 内置 API，可以做成只有菜单栏的应用 | 依赖插件，RustDesk 不得不 fork | 原生且成熟 | 不支持 |
| Android | 会取代 Kotlin 应用，原生能力需要重写成插件 | 同样会取代 Kotlin | 很重 | 浏览器可用，但没有后台能力 |
| 开发效率 | 高（热更新、生态大、Rust 侧胶水代码少） | 较高，但桌面多窗口和代码生成有摩擦 | 低 | 最高 |
| 和"daemon 即后端"的契合度 | **最好**：前端只是一个 HTTP/SSE 客户端 | FRB 容易诱导把核心直接嵌进进程，破坏 daemon 分离 | 一般 | 天然契合，但本机暴露面最大 |

**选择：Tauri 2 + Svelte 5 + TypeScript + Vite。** 理由如下：
- Svelte 5 的 runes 是细粒度响应式，适合"事件流不断修改状态"的场景，运行时也小。
- 项目的 Rust 工具链可以直接复用。
- 同一份前端产物还能编译成 daemon 托管的 Web 版本。

不选 Flutter：不打算放弃 Kotlin 应用，Flutter 在移动端的优势用不上。
不选 Qt：迭代最慢，对 Android 也没有帮助。

---

## 3. 目标架构

```
┌──────────────────────── 桌面（用户会话） ────────────────────────┐
│ Tetherly.app / Tetherly.exe  (Tauri 2)                           │
│  ┌───────── WebView (Svelte SPA) ─────────┐  ┌── Rust 壳 ───────┐│
│  │ views ← stores ← api client ← Transport│◀▶│ invoke 代理       ││
│  └────────────────────────────────────────┘  │ 事件流转发 emit   ││
│                                              │ 托盘 / 自启 / 通知 ││
│                                              │ token 持有         ││
│                                              └────────┬─────────┘│
└───────────────────────────────────────────────────────┼──────────┘
                                                        │ HTTP + SSE（loopback）
┌──────────────────────── daemon（tetherly-node） ──────▼──────────┐
│ uihttp:  /api/v1/*（新的门面，类型化）   /api/*（旧路由，冻结）  │
│          /api/v1/events（快照 + 增量 + seq）                     │
│          /  → 内置 SPA 的 Web 构建（替换 ui/index.html）         │
│ Node / runtime / net / ...（不改动）                             │
└──────────────────────────────────────────────────────────────────┘
          ▲ 契约单一来源：crates/tetherly-api（DTO + 事件）
          ├─ ts-rs → client/src/lib/api/gen/*.ts
          └─ schemars JSON Schema → Android Kotlin 数据类
```

### 3.1 分层与职责

| 层 | 位置 | 职责 | 禁止 |
|---|---|---|---|
| 契约 | `crates/tetherly-api` | 定义 v1 的请求 DTO、响应 DTO、`Event`、`Snapshot` 和错误码，并导出 TS 类型和 JSON Schema | 依赖 `tetherly-node` 的内部类型 |
| 门面 | `crates/tetherly-node/src/uihttp/v1/*.rs` | 把 v1 请求转换成对 `Node` 现有方法的调用，再把结果转换成 DTO | 写业务逻辑（业务逻辑留在 runtime） |
| 事件 | `crates/tetherly-node/src/events.rs` + `uihttp/v1/events.rs` | `UiEvent` 到 v1 `Event` 的投影、快照组装、seq 环形缓冲 | 改变已有 `UiEvent` 的语义 |
| Transport | `client/src/lib/transport/` | 提供 `request()` 和 `subscribe()` 两个接口，分别由 `HttpTransport`（浏览器）和 `TauriTransport`（invoke 加事件）实现 | 掺入业务逻辑 |
| API 客户端 | `client/src/lib/api/` | 类型化的方法（`peers.list()`、`files.accept(id)`…）和错误归一 | 让 UI 直接调用 `fetch` |
| Store | `client/src/lib/stores/` | 按领域划分的状态，由快照加事件 reduce 得到，并提供命令方法 | 让页面绕过 store 自己维护服务端状态 |
| 视图 | `client/src/routes/`、`client/src/lib/components/` | 渲染和交互 | 页面里出现 `setInterval` 拉数据 |
| 桌面壳 | `client/src-tauri/` | 托盘、窗口、自启、系统通知、token、请求代理、事件流转发、深链接 | 拉起、重启或杀掉 daemon |

### 3.2 前后端分离的边界

- 前端只依赖 `/api/v1` 和 `/api/v1/events`。
- 旧的 `/api/*` 从 M1 起**冻结**：不再增加路由，只修 bug，供 CLI、旧控制台和 Android 过渡使用。M5 逐步下线别名（见 §9）。
- 后端增加新功能时，先改 `tetherly-api` 契约，再写门面，最后写前端。CI 检查生成的类型有没有提交并保持最新（漂移检查，见 §10）。

---

## 4. 后端：`/api/v1` 契约

### 4.1 通用约定

- **资源化命名**：名词用复数，动作用 `POST /<资源>/{id}/<动词>`，名称全部用 kebab-case，不设别名。
- **统一响应外壳**：成功时直接返回 DTO（HTTP 200/201/202）；失败时返回 `{"error":{"code":"peer_not_found","message":"…","details":{}}}`，同时带上正确的 4xx/5xx 状态码。错误码是 `tetherly-api` 中定义的枚举。
- **耗时操作**（发送文件、连接、基准测试）返回 `202 {"op_id":"…"}`，后续进度通过事件流中的 `op.*` 事件推送。
- **版本协商**：响应头带 `Tetherly-Api: 1` 和 `Tetherly-Version: 0.x.y`；客户端发送 `Tetherly-Client: desktop/1.0.0`。主版本不一致时，客户端提示用户升级。
- **鉴权**：v1 的**所有**路由都需要 token，只有 `GET /api/v1/health` 例外。读和写分两级：`read` 用于 GET 和事件流，`write` 用于其余操作；`admin` 用于 daemon 重启、日志级别、安全设置。token 文件里记录它属于哪一级。

### 4.2 资源清单（首版约 45 个端点）

| 领域 | 端点 | 对应旧路由（合并） |
|---|---|---|
| 系统 | `GET /health`、`GET /system`（版本、设备 ID、能力、端口）、`POST /system/restart`（admin） | `/api/health` `/api/sys` `/api/system` `/api/caps` `/api/daemon/*` |
| 设备 | `GET /devices`、`GET /devices/{id}`、`PATCH /devices/{id}`（别名、标签）、`POST /devices/{id}/connect`、`POST /devices/{id}/block`、`DELETE /devices/{id}/block`、`DELETE /devices/{id}`（忘记设备） | `/api/peers*` `/api/peer/*` `/api/discovered` `/api/profiles` `/api/link` |
| 配对 | `GET /pairing`（待处理请求、本机 PIN 和指纹）、`POST /pairing/{req}/accept`、`POST /pairing/{req}/reject`、`POST /pairing/pin/rotate` | `/api/accept` `/api/reject` `/api/pin*` `/api/otp*` |
| 布局 | `GET /layout`、`PUT /layout`（整体替换，带 `rev` 做乐观锁）、`GET/PATCH /layout/policy` | `/api/topology/*` |
| 输入 | `GET /input`（当前谁控制输入、隧道状态、权限）、`POST /input/release`、`PATCH /input/settings`（键位映射、锁边） | `/api/input/*` `/api/input/tunnel/*` |
| 传输 | `GET /transfers`、`POST /transfers`（发送文件或文件夹，可指定多台目标设备）、`POST /transfers/{id}/accept`、`POST /transfers/{id}/reject`、`POST /transfers/{id}/cancel`、`GET /inbox`、`DELETE /inbox/{name}`、`POST /inbox/{name}/open`（**仅限交互式会话**）、`PATCH /transfers/policy`（自动接收） | `/api/file*` `/api/files*` `/api/inbox` `/api/outbox` `/api/file/drag/*` |
| 剪贴板 | `GET /clipboard`（策略、最近同步记录）、`PATCH /clipboard/policy`、`POST /clipboard/push` | `/api/clip*` |
| 动态 | `GET /activity?cursor=`（通知、消息、候选验证码、handoff 合并成一条时间线）、`POST /activity/{id}/copy`·`/insert`·`/dismiss`·`/reply`、`POST /messages` | `/api/notify*` `/api/candidates*` `/api/msg*` `/api/handoff*` |
| 媒体与外设 | `GET/PATCH /media`、`GET /battery`、`POST /devices/{id}/locate`、`GET/PATCH /audio`、`GET/PATCH /camera`、`GET/PATCH /mic` | 对应旧路由 |
| 屏幕 / 远程 | `GET /screen`（会话列表、能力）、`POST /screen/sessions`、`DELETE /screen/sessions/{id}`、`GET/PATCH /screen/virtual-display` | `/api/screen/*`（诊断类子路由放到"诊断"下） |
| 设置 | `GET /settings`、`PATCH /settings`（JSON Merge Patch，字段按领域分组：`general` `input` `clipboard` `transfer` `notify` `security`） | 各个 `*/policy`、`*/config`、`/api/config` |
| 诊断 | `GET /diagnostics`（链路质量、延迟、流量、内存）、`GET /diagnostics/logs?level=&tail=`、`PATCH /diagnostics/log-level`（admin）、`GET /diagnostics/audit`、`POST /diagnostics/bench`、`GET /diagnostics/screen/*`、`POST /diagnostics/checks/{name}`（端口、防火墙、发现） | `/api/log/*` `/api/metrics` `/api/traffic` `/api/net` `/api/bench` `/api/audit` `/api/security/audit` `/api/screen/diag*` |

> 门面实现的原则：每个 v1 handler 调用旧 handler 背后的同一个 `Node` 方法，不复制业务逻辑。门面写完后，旧路由的 handler 改为"薄转发到同一个 service 函数"，两边共享实现。

### 4.3 事件流 `GET /api/v1/events`

**协议**：选用 SSE（`text/event-stream`），因为 daemon 已经实现了它，浏览器原生支持 `EventSource`，Tauri 的 Rust 侧也很容易解析。

```
id: 1042
event: snapshot
data: {"seq":1042,"system":{…},"devices":[…],"layout":{…},"input":{…},"transfers":[…],"pairing":{…},"settings":{…}}

id: 1043
event: device.updated
data: {"seq":1043,"device":{"id":"tdev_…","online":true,"latency_ms":3,…}}

id: 1044
event: transfer.progress
data: {"seq":1044,"id":"tx_…","bytes":1048576,"total":8388608}
```

**规则：**
1. 连接建立后，第一条一定是 `snapshot`，包含当前完整状态，`seq` 等于当前最大序号。
2. 之后按 `seq` 严格递增发送增量事件。客户端只要发现序号不连续，就主动断开重连。
3. 断线重连时客户端带上 `Last-Event-ID: <seq>`：
   - 如果服务端环形缓冲（默认 1,024 条，或最近 60 秒）里还有这个序号之后的全部事件，就直接补发；
   - 否则重新发送一次 `snapshot`。客户端收到 `snapshot` 时**整体替换**本地状态。
4. 可选的 `?topics=devices,transfers` 用于过滤，托盘弹出面板只订阅需要的主题。
5. 每 15 秒发送一次心跳（沿用现状）。客户端超过 45 秒没收到任何数据，就判定断线。
6. **高频事件要节流**：`transfer.progress` 每个传输最多每秒 4 条，`device.link-quality` 最多每秒 1 条。节流在服务端投影层完成，内部的 `UiEvent` 广播不变。

**事件类别（v1 首版）**：

| 主题 | 事件 | 来源 `UiEvent` |
|---|---|---|
| devices | `device.added` `device.updated` `device.removed` | PeerUp/PeerDown/PeerDiscovered/PeerExpired/PeerAliasChanged/PeerCapsChanged/PeerBlocked/PeerUnblocked/PeerLatency/PeerLinkQualityRefreshed/PeerProfileUpdated |
| pairing | `pairing.requested` `pairing.resolved` | PairingNeeded/PinStateChanged |
| input | `input.changed` | InputSeatChanged/Overlay/OverlayStateChanged |
| layout | `layout.changed` | ConfigChanged（拓扑部分）/RouteChanged |
| transfers | `transfer.added` `transfer.progress` `transfer.finished` `inbox.changed` | FileOffered/FileRejected/FileDone/InboxFileDeleted/InboxCleared/OutboxUpdated… |
| activity | `activity.added` `activity.removed` | Candidate*/ReplyReceived/MessageReceived/MessageSent/ResourceHandoff*/NotificationBannerSent |
| clipboard | `clipboard.synced` `clipboard.blocked` | ClipboardSynced/ClipboardSyncBlocked/ClipboardCleared |
| settings | `settings.changed` | 各类 `*PolicyChanged` 和 `*ConfigChanged` |
| system | `system.changed` `diagnostics.metrics` | Status/SystemMetricsRefreshed/NetworkRefreshed/ScreenStateChanged… |
| ops | `op.progress` `op.done` `op.failed` | 对应耗时操作 |

> 实现方式：用一个纯函数 `fn project(ev: &UiEvent, state: &mut ProjectionState) -> Vec<v1::Event>` 把 96 种内部事件投影成约 25 种对外事件；`ProjectionState` 同时负责组装快照。投影是纯函数，可以直接写单元测试。没有对应关系的内部事件不对外暴露。

### 4.4 鉴权与跨进程信任

| 场景 | 方案 |
|---|---|
| 浏览器访问 daemon 托管的 SPA | 沿用现有做法：托盘或 `tetherly dashboard` 打开 `http://127.0.0.1:45716/#token=…`，前端读出 token 后立即用 `history.replaceState` 清掉 fragment，token 只放在 `sessionStorage`；同源请求，Origin 校验不需要改 |
| Tauri 客户端（macOS，daemon 以用户身份运行） | Rust 侧读取 `~/Library/Application Support/tetherly/api.token`（0600）；WebView 里的 JS **拿不到 token**，所有请求都通过 `invoke("api", {...})` 由 Rust 侧代理发出 |
| Tauri 客户端（Windows，daemon 以 SYSTEM 运行） | daemon 为当前交互用户另外签发一个**客户端 token**，写入 `%LOCALAPPDATA%\Tetherly\client.token`，ACL 只允许该用户 SID 和 SYSTEM；权限级别为 `read+write`，**不含 `admin`**。admin 操作需要 UAC 提权，或者改用 CLI。思路借鉴 Tailscale 的 sameuserproof 和 MWB 的命名管道 ACL |
| Origin 校验 | Tauri 的请求由 Rust 侧发出，不带 Origin 头，现有逻辑直接放行；**不放宽** `origin_allowed` |
| 能力降级 | 有些操作只能在交互式会话中执行（例如 `inbox/{name}/open`，SYSTEM 会话会弹出窗口），daemon 不在 SYSTEM 会话里执行，而是返回 `409 needs_interactive_session`，由 Tauri 客户端在用户会话里通过 `opener` 插件打开 |

---

## 5. 前端工程

### 5.1 目录

```
client/
├─ package.json            # pnpm, vite, svelte 5, typescript, vitest, playwright
├─ src/
│  ├─ app.html / main.ts
│  ├─ lib/
│  │  ├─ transport/        # Transport 接口 + HttpTransport + TauriTransport + MockTransport
│  │  ├─ api/
│  │  │  ├─ gen/           # ts-rs 生成的类型（提交到仓库，CI 检查漂移）
│  │  │  └─ client.ts      # 类型化方法、错误归一
│  │  ├─ stores/           # system / devices / layout / input / transfers / activity / settings / connection
│  │  ├─ components/       # 设计系统：Button, Toggle, Card, List, Badge, StatusDot, Sheet, Dialog, Toast, EmptyState, Skeleton
│  │  ├─ i18n/             # zh-CN（默认）、en
│  │  └─ theme/            # 设计 token（颜色、间距、圆角、字号），亮色和暗色
│  └─ routes/              # 每个页面一个目录，见 §6
├─ src-tauri/
│  ├─ Cargo.toml           # 依赖 tetherly-api（只用类型），不依赖 tetherly-node
│  ├─ src/{main.rs,proxy.rs,events.rs,tray.rs,token.rs,deeplink.rs}
│  └─ capabilities/        # Tauri 2 权限最小化：只开放 api/subscribe/window/notification/opener/autostart
└─ tests/                  # e2e（Playwright，对接 mock daemon 和真实 daemon）
```

`client/` 放进现有仓库，和 Rust workspace 并列，方便 CI 统一管理。`src-tauri` 作为 workspace 的独立成员，但不进入 daemon 的构建图。

### 5.2 Transport 抽象

```ts
export interface Transport {
  request<T>(method: Method, path: string, body?: unknown): Promise<T>; // path 相对 /api/v1
  subscribe(lastSeq: number | null, on: (e: ServerEvent) => void,
            onState: (s: LinkState) => void): () => void;              // 返回取消订阅函数
}
```

- `HttpTransport`：用同源 `fetch` 带上 `Authorization: Bearer`；事件流用 `fetch` 加 `ReadableStream` 自行解析 SSE，这样可以带鉴权头（原生 `EventSource` 不支持自定义请求头），同时自己管理 `Last-Event-ID` 和指数退避重连（1s → 2s → 5s → 10s，有上限）。
- `TauriTransport`：用 `invoke('api', {method, path, body})` 发请求；事件流由 Rust 侧维护唯一一条 SSE 连接，用 `emit('tetherly://event', e)` 推给所有窗口（主窗口和托盘弹出面板共用这一条连接）。
- `MockTransport`：用于测试和 Storybook 式的组件预览，可以按脚本回放事件。

### 5.3 状态管理

- 每个领域一个 store：`$state` 保存数据，`$derived` 计算派生视图。所有 store 只通过 `applySnapshot(s)` 和 `applyEvent(e)` 修改，这两个都是**纯 reducer**，单元测试直接覆盖。
- `connection` store 记录 `connecting | live | resyncing | offline` 和最后的 `seq`。顶栏根据它显示"daemon 未运行，正在重连…"这类状态，不弹出报错。
- **命令和数据分离**：页面调用 `devices.rename(id, name)` → 发 HTTP → 等 200 → **不手动改本地状态**，等事件流里的 `device.updated` 回来再更新。对延迟敏感的开关（如剪贴板同步）可以先乐观更新，同时保存回滚值：请求失败就回滚并弹出提示。
- 禁止在页面里写轮询。唯一允许的定时器是界面计时，比如"3 分钟前"这类相对时间的刷新，每 30 秒一次，不发网络请求。

### 5.4 设计系统

- 风格**简洁、克制**：一个主色，其余用中性灰；状态色只有 4 种（在线 / 连接中 / 离线 / 错误），**颜色一定配文字或图标**（避免 MWB 只靠颜色的问题）。
- 字体使用系统字体栈（`-apple-system, "Segoe UI", "PingFang SC", "Microsoft YaHei"`）；字号只用 12 / 13 / 15 / 20 / 28 五档；间距以 4 为单位。
- 亮色和暗色跟随系统，也可以手动切换；macOS 窗口启用 vibrancy（Tauri `windowEffects`），Windows 用 Mica。效果不可用时退化为纯色背景。
- 界面文案默认中文，统一使用"设备、布局、传输、动态"等术语，不在标题里中英混排。英文作为第二语言放在 i18n 资源里。
- 无障碍：所有可交互元素都能用键盘操作，焦点环清晰可见，使用语义化标签加 ARIA，WCAG AA 对比度。

---

## 6. 信息架构与页面逻辑

### 6.1 导航

```
┌ 托盘弹出面板（360×480） ┐     ┌──────────── 主窗口 ─────────────┐
│ 本机 · 输入在 Windows ▸  │     │ ◉ 概览                          │
│ ─ 设备 ─                 │     │ ▢ 设备                          │
│ ● Windows  3ms  [▸][⇪]   │     │ ⊞ 布局                          │
│ ○ Pixel    离线          │     │ ⇅ 传输     (2)                  │
│ ─ 传输 ─                 │     │ ✉ 动态     (5)                  │
│ report.pdf ▓▓▓░ 62%      │     │ ───                             │
│ [暂停共享] [打开主窗口]  │     │ ⚙ 设置                          │
└──────────────────────────┘     │ ⚕ 诊断                          │
                                  └─────────────────────────────────┘
```

| 页面 | 回答的问题 | 内容 | 主要操作 |
|---|---|---|---|
| **概览** | "现在一切正常吗？" | 本机卡片（名称、版本、地址、链路 lan/relay）；输入归属（"键鼠当前在 Windows"）；在线设备横排；进行中的传输；最多 3 条需要处理的事项（配对请求、待接收文件、权限缺失） | 暂停或恢复共享、收回输入、跳转到具体事项 |
| **设备** | "我连了哪些设备，各能做什么？" | 列表分为已配对、附近发现、已屏蔽三组。详情页依次显示：头部（名称、平台、在线状态、延迟、电量），然后是能力分区（输入、剪贴板、文件、屏幕、通知、媒体），**只显示两端都支持的能力**（参考 KDE Connect） | 重命名、发送文件、推送剪贴板、定位、远程屏幕、屏蔽、忘记 |
| **布局** | "屏幕之间怎么摆？" | Deskflow 式网格画布：本机固定在中间，其他设备可以拖到上下左右；每条边显示"到达 X"；右侧属性面板设置锁边、切换延迟、角落死区 | 拖动摆放、键盘方向键移动选中设备、撤销、应用（`PUT /layout` 带 `rev`） |
| **传输** | "文件到哪了？" | 进行中（进度、速度、剩余时间）、待接收（接收 / 拒绝）、收件箱（打开、在文件夹中显示、删除）、历史；整个页面都可以拖入文件 | 发送（选择文件后选择目标设备，可多选）、取消、自动接收策略的快捷开关 |
| **动态** | "别的设备给我发了什么？" | 通知、消息、验证码候选、handoff、剪贴板同步记录合并成一条时间线，可以按类型或设备筛选 | 复制、插入、回复、忽略、全部清除 |
| **设置** | "怎么调整行为？" | 分组：通用（名称、开机自启、语言、主题、关闭到托盘）、输入（键位映射 Cmd↔Ctrl、锁边、热键）、剪贴板（同步方向、大小上限、敏感内容过滤）、传输（保存目录、自动接收规则）、通知（来源过滤、ANCS 规则）、安全（本机指纹、PIN 轮换、已授权设备、API token 重新生成）、高级（端口、日志级别、实验功能） | 每一项都即时生效，不需要"保存"按钮；危险操作二次确认 |
| **诊断** | "为什么不好用？" | 健康检查清单（daemon、端口、权限、防火墙、发现、链路），每一项都附带"修复"按钮；实时日志（虚拟列表，可按级别筛选，**每秒最多刷新 10 帧**，避免 Deskflow #10367 那样的重绘死循环）；指标（延迟、丢包、CPU、内存）；审计记录；基准测试；屏幕诊断（默认折叠在"高级诊断"里）；"复制诊断包"一键导出 | 运行检查、导出、调整日志级别 |

### 6.2 关键流程

1. **首次运行**：检测 daemon。如果没在运行，显示"安装或启动服务"的引导，并给出命令或安装器；客户端自己不拉起 daemon。接着检测权限（macOS 辅助功能、Windows 会话），然后进入设备发现，最后完成配对。
2. **配对**（参考 Deskflow 和 KDE Connect）：
   - 收到 `pairing.requested` 后，托盘发系统通知，主窗口弹出对话框；
   - 对话框并排显示对方设备名、平台和 8 位 PIN 或短指纹，以及本机对应的值；
   - 文案写"请确认两边显示一致"，**默认焦点在"拒绝"**；
   - 60 秒没有操作就自动拒绝。
3. **输入归属切换**：`input.changed` 实时更新概览和托盘图标。托盘图标有三种状态：本机控制、被远端控制、暂停；同时托盘提示文字说明"键鼠在 Windows"。
4. **文件拖放**：拖到托盘图标、主窗口或某台设备卡片上，都调用 `POST /transfers`。拖到托盘图标沿用 ADR-137 的投放区语义。
5. **daemon 断开**：顶部出现非阻塞的横幅"连接中断，正在重连"，页面保留最后状态并变灰，写操作按钮禁用；重新连接后收到快照，界面直接恢复，不刷新整页。

### 6.3 托盘和菜单栏

- 由 Tauri 壳统一实现（`tray-icon`）。图标有四种状态：正常、控制中、被控、异常，每种都有亮色和暗色版本。
- 左键单击打开**托盘弹出面板**：一个无边框 WebView 窗口，运行同一个 SPA 的 `/tray` 路由。它取代 `platform/popover.rs` 用 NSTextField 拼出的文本行，以及 Windows 的置顶弹窗。
- 右键菜单根据事件流动态生成：当前状态（真实数据）、各设备的快捷操作、暂停共享、打开主窗口、诊断、退出客户端。退出只关闭客户端，**不影响 daemon**。
- **迁移期间两个托盘不会同时出现**：客户端启动后通过 `PATCH /settings {general:{tray_owner:"client"}}` 通知 daemon 隐藏内置托盘；客户端退出或连接断开超过 10 秒，daemon 自动恢复内置托盘作为兜底（daemon 内置托盘保留到 M5 后再评估是否移除）。

---

## 7. 多端适配

| 端 | 形态 | 布局 |
|---|---|---|
| macOS | Tauri 应用，`LSUIElement`（只显示菜单栏图标，打开主窗口时才出现在 Dock），沿用 ADR-135 的签名身份 | 主窗口最小 720×480 |
| Windows | Tauri 应用（WebView2，Win10/11 自带），在用户会话中运行，开机自启写注册表 `Run` 键 | 同上，跟随 DPI 缩放 |
| 浏览器 | daemon 托管 SPA（`/`），功能与桌面版相同，只是没有托盘、系统通知和自启 | 同一套响应式布局 |
| Android | 保留 Kotlin 应用，逐步迁移到 Jetpack Compose，采用相同的信息架构（概览 / 设备 / 传输 / 动态 / 设置；没有布局编辑和诊断的完整版），数据类由 JSON Schema 生成 | 底部导航 |

**响应式断点**（SPA 内部，参考 LocalSend 的单组件树）：

| 宽度 | 导航 | 内容 |
|---|---|---|
| < 480（托盘面板、手机浏览器） | 不显示导航，只有紧凑视图 | 单列卡片 |
| 480–899 | 底部导航（5 项，设置和诊断收进"更多"） | 单列 |
| 900–1279 | 左侧图标导航栏 | 列表加详情跳转 |
| ≥ 1280 | 左侧带文字的导航栏 | 列表和详情双栏并排 |

---

## 8. 迁移计划与里程碑

| 阶段 | 内容 | 交付和验收 |
|---|---|---|
| **M0 契约基础** ✅ v0.2.56（§127；schemars 推迟到 M5） | 新建 `tetherly-api` crate（DTO、`Event`、`Snapshot`、错误码），接入 `ts-rs` 和 `schemars`；`/api/v1/health` 和 `/system`；v1 的读写两级鉴权；CI 加类型漂移检查 | `cargo test` 覆盖 DTO 序列化往返；CI 绿 |
| **M1 事件流** ✅ v0.2.57（§128；旧路由计数随 M2 diagnostics） | `project()` 投影函数、`ProjectionState` 快照、`seq` 环形缓冲、`Last-Event-ID` 续传、主题过滤、高频节流；旧 `/api/*` 冻结 | 单元测试：投影纯函数、序号连续、缓冲溢出后重新发快照、节流；集成测试：断线续传 |
| **M2 门面 + Web SPA** | 实现 §4.2 全部端点；`client/` 工程搭好，包括 Transport、store、设计系统和 7 个页面；daemon 在 `/next/` 托管新 SPA，旧控制台留在 `/` | vitest 覆盖 reducer 和 store；Playwright 对接 MockTransport 跑主要流程；在真实 daemon 上冒烟测试 |
| **M3 Tauri 桌面壳** | `src-tauri`：请求代理、事件流转发、token 读取（含 Windows 客户端 token 签发）、托盘、托盘面板、自启、系统通知、深链接；CI 的 release 产物增加 `.dmg` 和 `.msi` | 两端安装运行；托盘状态和事件同步；daemon 重启后客户端自动恢复；内存占用 < 120 MB |
| **M4 切换** | `/` 改为新 SPA，旧控制台移到 `/legacy/` 保留两个版本；daemon 内置托盘的双击动作改为优先唤起客户端（深链接 `tetherly://open`），失败时再打开浏览器；`popover.rs` 退役 | 生产两端连续使用一周没有回退 |
| **M5 收敛** | 删除 `ui/index.html` 和 `/legacy/`；旧 `/api/*` 别名逐个下线（先在日志中记录调用方，确认 CLI 和 Android 已迁移后再删）；Android Compose 改版 | 路由数从约 555 降到约 60（v1）加 CLI 必需路由；`uihttp.rs` 显著变小 |

每个阶段都按项目现有的发布流程执行：TDD、fmt、clippy（含 windows-gnu）、test、版本号、CI、tag、deploy 和两端验证；两端都只部署 CI 产物。Tauri 客户端是新增产物，**不替换 daemon 二进制**，daemon 的部署流程不变。

---

## 9. 旧 API 下线策略

1. M1 起，`uihttp` 对每个**旧**路由的命中做计数并按调用方 UA 分类，可以在 `GET /api/v1/diagnostics` 的 `legacy_routes` 字段查看。
2. 列出别名映射表（`docs/` 附录维护），例如 `/api/message`、`/api/messages` → `/api/msg`。M5 先删掉别名，只保留一个规范名；再按调用方确认已迁移到 v1 后，逐组删除。
3. CLI（`bins/tetherly-cli` 和 `tetherly` 子命令）在 M2–M3 期间逐步改用 v1 客户端（Rust 侧直接复用 `tetherly-api` 的类型）。
4. Android：在 M5 Compose 改版时迁移；迁移之前，它用到的旧路由不删除。

---

## 10. 测试策略

| 层 | 工具 | 覆盖内容 |
|---|---|---|
| 契约 | `cargo test`（tetherly-api） | DTO 和事件的 JSON 往返；生成的 TS 和 Schema 与提交版本一致（CI 运行 `cargo run -p tetherly-api --bin gen && git diff --exit-code`） |
| 门面 | `cargo test`（tetherly-node，沿用 `temp_test_node`） | 每个端点的正常和错误路径；鉴权分级（无 token / read / write / admin）；Host/Origin 拒绝；`409 needs_interactive_session` |
| 事件 | `cargo test` | `project()` 对每类 `UiEvent` 的映射；快照和增量合起来与重新拉取快照的结果一致（性质测试：对随机事件序列，`fold(snapshot0, events) == snapshot_n`）；续传和重新发快照；节流 |
| 前端单元 | vitest | reducer 是纯函数；store 的命令和事件确认流程；Transport 的序号缺口检测与退避 |
| 组件 / 页面 | vitest + @testing-library/svelte | 空状态、加载、错误、断线时变灰显示 |
| 端到端 | Playwright | 用 MockTransport 回放脚本（配对、拖文件、布局调整、断线恢复）；在真实 daemon 上冒烟测试（只读操作） |
| 桌面壳 | `cargo test`（src-tauri） | 请求代理的 path 白名单（只允许 `/api/v1/`）；token 读取与权限；SSE 解析器 |
| 无障碍 | axe-core（Playwright 中运行） | 每个页面零严重问题 |
| 性能 | 手动 + 脚本 | 空闲时客户端 CPU ≈ 0%（没有轮询）；事件突发时（1,000 条/秒）界面帧率 ≥ 50；daemon 在没有客户端时开销不变 |

---

## 11. 风险与对策

| 风险 | 对策 |
|---|---|
| 门面和旧路由行为不一致 | 门面只调用共享的 service 函数；门面测试复用旧路由测试的输入和输出 |
| 事件投影遗漏，界面状态过期 | 性质测试 `fold(snapshot, events) == snapshot`；客户端发现序号缺口就重新同步；诊断页提供"强制重新同步"按钮 |
| Windows SYSTEM daemon 与用户会话客户端之间的 token 安全 | 客户端 token 不含 admin 权限；文件 ACL 只允许该用户；客户端 token 可以单独吊销和轮换；所有写操作都进审计记录 |
| 迁移期间两个托盘同时出现或互相抢占 | 用 `tray_owner` 设置加 10 秒超时兜底；托盘归属变更写进事件流 |
| WebView 差异（WKWebView 与 WebView2） | 不使用实验性 CSS；Playwright 在 WebKit 和 Chromium 两个引擎上都跑 |
| 工作量大 | 每个阶段独立发布、可以随时停在某个阶段；旧控制台到 M4 前一直可用 |
| 新增依赖（ts-rs、schemars、tauri、前端工具链） | 都要通过 `cargo deny`；前端锁定 `pnpm-lock.yaml`；Tauri 只放在 `client/src-tauri`，不进入 daemon 的依赖图 |

---

## 12. 待定事项（实现时决定，已给出默认值）

| 事项 | 默认 |
|---|---|
| 前端框架 | Svelte 5；如果更看重团队熟悉度，可以换 React，Transport、store、契约三层不受影响 |
| 事件流格式 | SSE；如果以后需要双向通信（例如远程桌面控制通道），再单独加 WebSocket 端点，不复用事件流 |
| 客户端 token 签发时机 | daemon 检测到交互用户登录时签发；没有交互用户时不签发 |
| Android 改造 | M5 再做；在此之前只同步术语和信息架构的文档 |
