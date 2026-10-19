# Session log

## 2026-10-19 (59) - LAN device discovery & beacon governance, discovery observability & instant pairing parity (v1.61 / ADR-054)
- Task: 实现局域网设备自发现治理、信标可观测性与一键拨号配对闭环 (LAN Device Discovery & Beacon Governance, Discovery Observability & Instant Pairing Parity)，终结既有系统单向广播自身但从未浏览/监听对端信标的“监听黑盒”，提供强类型 DTO 画像、动态重新探测、智能拨号寻址引擎、零明文事件即时分发、双端 CLI 完整同构与 Web UI 一键直连交互面板。
- **背景与缺陷根因**:
  在 Tetherly 陆续完成跨端收件箱生命周期治理与安全删除 (v1.60 / ADR-053)、批量文件提议与断点续传可观测性画像 (v1.59 / ADR-052) 及全桌面 ANCS GATT 接入闭环 (v1.58 / ADR-051) 之后，核心数据面与传输面已高度可靠。然而，在局域网设备自发现治理与拨号链路层面，系统长期存在控制与用户体验盲区：
  1. 单向广播与发现监听黑盒（One-Way Advertising without Browsing Blackout）：节点虽然能通过 `crates/tetherly-net/src/mdns.rs` 广播自身，但从未启动服务浏览监听对端信标，导致同一局域网设备彼此无法感知；
  2. 缺乏局域网设备发现画像与控制面查询端点（Discovery Observability Blackout）：缺乏 `GET /api/discovery` 与 `POST /api/discovery/rescan`，无法查询局域网设备列表、IP 候选集、端口、指纹与能力；
  3. 拨号流程割裂与必须手动输入 IP:Port 痛点（Manual IP Dial Friction）：用户必须手动查询并输入 IP:Port，无法按设备名或 ID 智能寻址；
  4. 事件总线局域网发现事件断层（Event Bus Gap for Discovered Peers）：设备上线或离线时缺乏事件广播；必须恪守 Zero-Plaintext 铁律（严禁泄露密钥）；
  5. 双端 CLI 与 Web UI 缺少自发现全同构支持：缺少双端同构的 `discover` 子命令与智能 `dial --peer`；Web UI 缺少发现面板与一键直连。
- **架构设计与落地 (ADR-054)**:
  1. 网络层 mDNS 浏览与注销原语 (`crates/tetherly-net/src/mdns.rs` & `lib.rs`):
     - 封装 `mdns_sd::ServiceDaemon` 的浏览能力：`browse` 与 `stop_browse` 原语，处理 `ServiceResolved` 与 `ServiceRemoved`；非 `lan` 特性提供安全桩；注册时 TXT 记录加入 `("name", hostname)`；
  2. 核心模型设计与 DTO 抽象 (`crates/tetherly-node/src/runtime.rs` & `lib.rs`):
     - 定义结构化 DTO：`DiscoveredPeerDto`、`DiscoveryReport`、`DiscoveryRescanReport`；
     - 维护线程安全 `discovered: Mutex<HashMap<String, DiscoveredPeerInfo>>`，过滤本机设备，计算在线与受信任状态，按活跃时间排序；
     - 实现 `Node::discovered_peers` 与 `Node::rescan_discovery`；
     - 实现 `Node::dial_peer` 智能寻址引擎（按物理地址、发现表、路由表优先级解析，配备 2s 单目标快速超时）；
  3. 事件总线扩展与安全红线守卫 (`crates/tetherly-node/src/events.rs`):
     - 新增 `UiEvent::PeerDiscovered` 与 `UiEvent::PeerExpired`，严格遵循 Zero-Plaintext 铁律；
  4. 轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`):
     - 新增 `GET /api/discovery`（别名 `/api/peers/discovered`）、`POST /api/discovery/rescan`（别名 `/api/peers/discover`），增强 `POST /api/dial` 原生支持 `peer` 智能寻址，严格执行 127.0.0.1 物理绑定与 `Host: 127.0.0.1|localhost` 403 阻断；
  5. CLI 双端全同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`):
     - 增设 `discover [--rescan] [--json]` 子命令；升级 `dial [--addr] [--peer <name|id>]`；SSE `events` 监听器高亮 `[peer_discovered]` 与 `[peer_expired]`；
  6. Web UI 局域网自发现交互面板 (`ui/index.html`):
     - 新增「局域网自动发现设备 (Discovered LAN Devices)」卡片，支持实时设备列表、能力特性徽标、一键地址填入、一键快捷拨号与 SSE 联动无感局部重绘；
  7. 端到端集成测试全覆盖 (`tests/phase1.rs`):
     - 新增 `lan_discovery_and_beacon_governance_via_loopback_api` 完备验证单测/集成测试，全部 273+ 项测试 100% 全绿，确立 ADR-054，SSOT 升至 v1.61。

## 2026-10-19 (58) - Inbox file lifecycle, multi-dimensional query & safe purge governance (v1.60 / ADR-053)
- Task: 实现跨端收件箱生命周期治理、已接收文件多维检索与安全审计清除全闭环 (Inbox File Lifecycle, Multi-Dimensional Query & Safe Purge Governance)，解决收件箱落盘后细粒度画像盲区、缺乏单文件与批量销毁原语及事件总线断层痛点，确立严格路径穿越拦截与零明文审计红线，提供双端 CLI/Web UI 全同构控制面。
- **背景与缺陷根因**:
  在 Tetherly 完成跨端多文件批量发送提议、断点续传可观测性画像与传输治理全闭环 (v1.59 / ADR-052) 之后，文件协同网络在发送端提议、网络层流式分块、断点续传和存储统计上已达成强一致性。然而，在收件箱（Received Files / Inbox）文件生命周期与安全治理层面，依然暴露出显著的功能盲区与审计空白：
  1. 收件箱文件生命周期与细粒度画像盲区（Inbox File Inspection Blackout）：
     - 当对端推送的文件传输完成并原子校验入库落盘在 `<data_dir>/inbox/` 目录后，既有系统仅能在 `GET /api/storage` 中提供粗粒度的总量概览（`inbox_files_count` 与 `inbox_total_bytes`）；外部调用方、CLI 终端与 Web UI 无法探查收件箱内具体存有哪些文件（文件名、大小、修改/入库时间戳、文件后缀扩展名、安全文件名合规标记），缺乏多维条件过滤（按文件名搜索 `search`、按最近修改时间倒序、按数量分页 `limit`）；
  2. 缺乏已接收文件的安全治理与生命周期销毁原语（Secure Deletion & Safe Purge）：
     - 目前系统仅有针对断点续传临时分块的 `POST /api/storage/clean`（仅清理 `.<id>.part` 与 `.<id>.resume.json`），缺乏针对已落盘完成文件的安全管理机制；当用户接收了敏感临时文件或需要释放存储空间时，无法通过回环控制面或 CLI 安全删除指定文件或一键清空收件箱；
  3. 路径遍历安全红线守卫（Path Traversal Guard）：
     - 文件删除操作必须严格限制在 `inbox/` 根目录内部，调用 `tetherly_core::sanitize_file_name` 严密防范包含 `..`、绝对路径、隐藏文件或 NUL 字节的任意文件删除与目录逃逸攻击；
  4. 收件箱事件总线生命周期断层（Event Blackout on File Receipt & Removal）：
     - 文件从收件箱删除或清空时，未向事件总线派发任何事件；Web UI 与 SSE 订阅者无法实现无缝即时局部刷新；必须严格恪守 Zero-Plaintext 铁律：事件载荷仅允许携带脱敏元数据（`name`, `freed_bytes`, `timestamp_ms` 等），严禁携带文件物理路径或内部内容；
  5. 双端 CLI 与 Web UI 缺少全同构治理入口：
     - 缺少双端同构的 `inbox` 子命令族（`inbox list`、`inbox delete <name>`、`inbox clear`）；Web UI 缺乏已接收文件治理面板、检索框与行内删除/一键清空交互。
- **架构设计与落地 (ADR-053)**:
  1. 核心模型设计与 DTO 抽象 (`crates/tetherly-node/src/runtime.rs` & `lib.rs`):
     - 定义结构化 DTO：`InboxFileDto`（`name`、`size`、`modified_at_secs`、`is_safe`、`extension`）、`InboxReport`、`InboxDeleteReport` 与 `InboxClearReport`；
     - 实现 `Node::inbox_report(&self, search: Option<&str>, limit: Option<usize>) -> InboxReport`：单遍扫描 `<data_dir>/inbox` 目录，跳过隐藏/侧车文件，计算安全合规性，支持 search 不区分大小写搜索与时间倒序排序与分页 limit 截断；
     - 实现 `Node::delete_inbox_file(&self, name: &str) -> Result<InboxDeleteReport, CoreError>`：严密执行路径穿越阻断（拒绝包含 `..`、`/`、`\`、NUL 或以 `.` 开头的文件名），严格校验 `clean_name == name`，物理删除目标文件，释放字节并向事件总线广播 `UiEvent::InboxFileDeleted`；
     - 实现 `Node::clear_inbox(&self, before_secs: Option<u64>) -> Result<InboxClearReport, CoreError>`：扫描并安全删除常规文件（保护断点续传侧车文件），支持 `before_secs` 时间截断，统计释放容量并向事件总线广播 `UiEvent::InboxCleared`；
  2. 事件总线扩展与安全红线守卫 (`crates/tetherly-node/src/events.rs`):
     - 新增 `UiEvent::InboxFileDeleted` 与 `UiEvent::InboxCleared` 事件变体，严格遵守 Zero-Plaintext 铁律，不携带任何文件正文与物理磁盘路径；
  3. 轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`):
     - 新增 `GET /api/file/inbox`（与别名 `GET /api/inbox`），支持 `?search=` 与 `?limit=`；
     - 新增 `POST /api/file/inbox/delete`（与别名 `DELETE /api/file/inbox`），支持 body 或 query 传参，文件不存在返回 404；
     - 新增 `POST /api/file/inbox/clear`（与别名 `DELETE /api/file/inbox/all`），支持可选 `before_secs`；
     - 严格 127.0.0.1 回环绑定与 `Host: 127.0.0.1|localhost` 403 校验；
  4. CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs` 与 `crates/tetherly-node/src/bin/tetherly.rs`):
     - 增设 `inbox` 子命令族（`inbox list`、`inbox delete`、`inbox clear`）；
     - SSE `events` 监听器高亮 `[inbox_file_deleted]` 与 `[inbox_cleared]`；
  5. Web UI 已收文件治理交互面板 (`ui/index.html`):
     - 新增「已接收文件与收件箱治理 (Inbox)」卡片，支持实时搜索过滤、单项删除与一键清空，SSE 自动局部重绘；
  6. 端到端集成测试全覆盖 (`tests/phase1.rs`):
     - 新增 `inbox_file_lifecycle_query_deletion_and_safe_purge_via_loopback_api` 完备验证单测/集成测试，全部 342+ 项测试 100% 全绿，确立 ADR-053，SSOT 升至 v1.60。

## 2026-10-19 (57) - Batch file offer lifecycle, multi-file pipeline & deep resume observability (v1.59 / ADR-052)
- Task: 实现跨端多文件批量发送提议、断点续传可观测性画像与传输治理全闭环 (Batch File Offer Lifecycle, Multi-File Pipeline & Deep Resume Observability)，解决此前单文件逐个提议导致的对端弹窗洪泛与缺乏批次聚合统计痛点，打破分块断点续传侧车黑盒，提供双端 CLI/Web UI 全同构控制面与互斥锁自死锁解耦。
- **背景与缺陷根因**:
  在 Tetherly 完成三大桌面平台 ANCS 硬件级接入闭环 (v1.58 / ADR-051) 后，跨端数据协同网络底层稳固。但在高频跨端文件分发与断点续传治理中暴露出三个核心痛点：
  1. 批量文件提议体验缺失（Missing Batch File Offer Pipeline）：
     - 现存文件发送控制端点仅支持单个物理路径或单个 Base64 载荷提议（`POST /api/file/send` 与 `tetherly send <path>`）；用户在向对端发送多个文件时，必须逐个调用，导致对端产生大量孤立的提议弹窗，无法一次性呈现批次文件清单（`files: Vec<FileDesc>`）与总尺寸累计；
     - 缺乏全局唯一的批次标识符与聚合状态回报（`BatchSendReport`、`total_files`、`successful_files`、`failed_files`、`total_bytes`、各文件独立状态）；
  2. 断点续传深度可观测性黑盒（Resume Markers Observability Blackout）：
     - 节点虽然在底层实现了 64 KiB 分块流式加密与断点续传恢复（`ResumeTable`、`inbox/.<transfer_id>.resume.json` 与 `.{transfer_id}.part`），但上层控制面处于完全黑盒状态；
     - 外部监控探针、CLI 与 Web UI 无法直观查询当前系统中存在哪些可续传分块、已就绪偏移量、块对齐特征（`chunk_aligned`）以及完成百分比（`progress_percent`）；
  3. 存储统计与互斥锁潜在自死锁隐患（Mutex Self-Deadlock in `storage_report`）：
     - `storage_report()` 在扫描收件箱与分块时持有了 `self.shared.hubs.lock()`，若在此锁作用域内直接调用 `self.resume_markers_report()`（后者同样需要获取 `hubs` 锁以反解对端友好别名），在不可重入的 `std::sync::Mutex` 下将直接引发致命的自死锁；
  4. 回环端点、CLI 与 Web UI 缺乏规范化同构升级：
     - 缺少 `POST /api/file/send-batch`（及别名 `/api/file/batch`）与 `GET /api/file/resume`（及别名 `/api/storage/resume`）；
     - CLI `send` 子命令仅接受单路径，缺少原生多路径 `send <paths...>` 批次汇总；缺少 `resume` 续传分块查询子命令；
     - Web UI（`ui/index.html`）`<input id="sendfile">` 缺少 `multiple` 批量多选支持与断点续传徽标画像。
- **架构设计与落地 (ADR-052)**:
  1. 批量文件提议核心模型与原子流式管道 (`crates/tetherly-node/src/runtime.rs`):
     - 定义 `BatchSendRequest`、`BatchSendItem`、`BatchSendReport` 与 `BatchTransferItemReport`；
     - 自动生成符合规范的全局唯一批次标识 `tbt_<hex12>`（tetherly batch transfer）；
     - 实现 `Node::send_files_batch` 与 `Node::send_batch_items`，原生支持磁盘物理路径与 Base64 内存缓冲混合提议，严格执行 $O(1)$ 内存流式分块哈希（64 KiB 增量 SHA-256 计算）与总字节数、成功/失败计数原子回报；
  2. 深度断点续传可观测性画像与互斥锁安全解耦 (`crates/tetherly-node/src/runtime.rs`):
     - 定义 `ResumeMarkerDto` 与 `ResumeMarkersReport`，计算块对齐特征（`offset_bytes % 65536 == 0`）与精确完成百分比（`progress_percent`）；
     - 实现 `Node::resume_markers_report`：双重检索内存 `ResumeTable` 与磁盘 `inbox/` 目录中的 `.<transfer_id>.resume.json` 与 `.part` 实体，反解对端友好别名；
     - 解耦 `storage_report()` 互斥锁作用域，先在紧凑临界区内提取收件箱指标，锁释放后再检索断点画像，根除自死锁隐患；
  3. 轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`):
     - 新增 `POST /api/file/send-batch` 与 `POST /api/file/batch`；
     - 新增 `GET /api/file/resume` 与 `GET /api/storage/resume`；
     - 严格 127.0.0.1 回环绑定与 `Host: 127.0.0.1|localhost` 403 校验；
  4. CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs` 与 `crates/tetherly-node/src/bin/tetherly.rs`):
     - 升级 `send` 子命令支持原生接收多个路径（`paths: Vec<PathBuf>`），以格式化表格展示批次聚合统计；
     - 新增 `resume` 命令支持 ASCII 进度表格排版与 `--json` 原生管道输出；
  5. Web UI 多文件提议与续传状态画像 (`ui/index.html`):
     - `<input id="sendfile" multiple>` 升级原生支持批量文件选择与多任务 Base64 聚合提议；
     - 传输卡片增设断点续传绿色徽标（`Resumed`）画像；
  6. 端到端集成测试全覆盖 (`tests/phase1.rs`):
     - 新增 `batch_file_offer_pipeline_and_resume_observability_via_loopback_api` 完备验证单测/集成测试，全部 341+ 项测试 100% 全绿，确立 ADR-052，SSOT 升至 v1.59。

## 2026-10-19 (56) - Linux BlueZ D-Bus GATT transport stack architecture & cross-platform tripartite ANCS parity (v1.58 / ADR-051)
- Task: 实现 Linux BlueZ D-Bus GATT 传输栈架构与跨平台三端 ANCS 全闭环 (Windows / macOS / Linux)，终结 Linux 桌面与工作站平台此前硬编码返回 `Ok(None)` 的平台断层，达成三大桌面操作系统原生硬件级 ANCS 接入能力的 100% 同构全闭环。
- **背景与缺陷根因**:
  在 Tetherly 陆续完成 ANCS 苹果通知服务全生命周期治理与事件总线闭环 (v1.56 / ADR-049)、macOS 原生 CoreBluetooth 传输栈与跨平台地址抽象 (v1.57 / ADR-050) 后，Tetherly 在 Windows（WinRT GATT）与 macOS（CoreBluetooth）均已具备原生硬件级 ANCS 接入能力。然而，在 **Linux 桌面与工作站平台（`#[cfg(all(unix, not(target_os = "macos")))]`）** 上，底层传输层仍存在显著断层：
  1. Linux 平台传输栈缺失（Missing Linux GATT Transport）：`crates/tetherly-node/src/ancs.rs` 中针对 Linux 依然硬编码返回 `Ok(None)`，守护进程在 Linux 上启动时若传入 `--ancs <addr>`，只能打印 `ANCS is not implemented on this platform; iPhone notifications stay off` 并直接丢弃通知接入能力；
  2. 三端架构异构统一的最后一公里：Windows 采用 WinRT MTA 异步驱动 (`win_gatt.rs`)；macOS 采用 CoreBluetooth + GCD 串行调度队列驱动 (`macos.rs`)；Linux 平台缺乏对应的标准 BlueZ D-Bus GATT 驱动；需要依托 Linux 标准 System D-Bus（`org.bluez`）接口，实现基于纯 Rust 异步 D-Bus 客户端（`zbus`）的高性能自包含传输栈，完成 Windows / macOS / Linux 三大桌面平台 ANCS GATT 驱动的 100% 同构全闭环。
- **架构设计与落地 (ADR-051)**:
  1. 零 Lockfile 扰动纯 Rust D-Bus 依赖架构 (`crates/tetherly-node/Cargo.toml`):
     - 在 `[target.'cfg(unix)'.dependencies]` 中配置 `zbus`：
       ```toml
       [target.'cfg(unix)'.dependencies]
       zbus = { version = "4", default-features = false, features = ["tokio"] }
       ```
     - 彻底复用 `Cargo.lock` 已锁定的 `zbus 4.4.0`（源自 `secret-service` 底层依赖），杜绝引入任何新的第三方 C/C++ 共享库或 AGPL/非许可协议包，`deny.toml` 检验 100% 纯净通过。
  2. 前端契约与线程隔离架构 (`crates/tetherly-node/src/ancs/bluez.rs`):
     - 前端结构体 `BluezAncsTransport` 仅持有 `std::sync::mpsc::Sender<Cmd>`，天然满足 `Send + Sync` 契约；
     - 内部同步方法在 `ACK_TIMEOUT = Duration::from_secs(10)` 内等待后台响应，超时映射为 `CoreError::AncsSubscribe` 或 `CoreError::AncsWrite`；
     - 实现 `Drop` 自动向后台派发 `Cmd::Stop` 信号触发优雅退出；
     - 后台专用线程 `"tetherly-ancs-bluez"` 拥有独立的单线程 Tokio Runtime，避免与全局或上层异步运行时产生嵌套死锁。
  3. BlueZ D-Bus 对象树拓扑与 48 位 MAC 寻址 (`dev_path_from_mac`):
     - 提供标准转换函数：
       ```rust
       pub fn dev_path_from_mac(mac: &[u8; 6]) -> String {
           format!(
               "/org/bluez/hci0/dev_{:02X}_{:02X}_{:02X}_{:02X}_{:02X}_{:02X}",
               mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
           )
       }
       ```
     - 连接系统 D-Bus (`Connection::system()`)，通过 `org.freedesktop.DBus.ObjectManager.GetManagedObjects` 检索设备路径及其子路径下的 ANCS 服务与 3 个特征。
  4. 严格恪守 SSOT §9.4 硬约束:
     - 严格按顺序订阅：首先订阅 Data Source (`StartNotify`)，再订阅 Notification Source (`StartNotify`)；
     - 监听 D-Bus `PropertiesChanged` 信号，解析 `ay`（字节数组）属性推入 `AncsInbound::NotificationSource` 与 `AncsInbound::DataSource`，严禁在信号监听线程反向写入 Control Point；
     - Control Point 写入命令通过 BlueZ `WriteValue(bytes, {"type": "request"})` 严格串行执行并等待 ATT Write Response 确认；
     - 监听设备 `Connected == false` 突变推入 `AncsInbound::Disconnected`，退出驱动循环触发指数退避重连 (`ancs_backoff_delay`)。
  5. 跨平台统一装配与路由 (`crates/tetherly-node/src/ancs.rs`):
     - Linux 平台 (`#[cfg(all(unix, not(target_os = "macos")))]`) 正式接入 `bluez::open(address).map(Some)`，达成三端原生 GATT 栈全闭环。
  6. 单元测试与跨平台验证:
     - 内置 5 项单元测试：MAC 地址 BlueZ 路径格式化、断开响应转换、命令往返 ACK、D-Bus `ay` 字节向量安全解包以及非法地址拒收；
     - `cargo fmt --all --check` 干净通过；
     - `cargo clippy --workspace --all-targets -- -D warnings` 零告警通过；
     - `cargo test --workspace` 全部 325+ 项测试 100% 全绿；
     - `cargo deny check`（advisories, bans, licenses, sources）全部通过。

## 2026-10-19 (55) - macOS ANCS CoreBluetooth transport stack architecture, cross-platform BLE address abstraction & daemon startup wiring fix (v1.57 / ADR-050)
- Task: 构建 macOS 原生 CoreBluetooth GATT 传输栈架构，确立跨平台强类型 `BleAddress` 外设地址抽象，彻底修复守护进程启动时静默丢弃 ANCS 传输层的关键断层。
- **背景与缺陷根因**:
  在 Tetherly 陆续完成 ANCS 苹果通知服务全生命周期治理与事件总线闭环 (v1.56 / ADR-049) 后，底层 ANCS 物理传输栈与跨平台寻址暴露出三个核心缺陷：
  1. 守护进程启动链路缺陷：在 `Cmd::Run` 中，用户指定 `--ancs <addr>` 时，调用 `open` 返回的 `transport` 被未使用的局部变量丢弃，`cfg.ancs` 从未被赋值，导致守护进程启动后内部状态机恒为 `None`，通知流完全无法工作；
  2. 跨平台外设寻址断层：Windows WinRT GATT 暴露 48 位物理 MAC 地址，而 macOS CoreBluetooth 出于隐私严格隐藏硬件 MAC 仅暴露 128 位 `NSUUID`，既有地址解析逻辑硬编码 48 位校验直接报错拒收；
  3. macOS CoreBluetooth 传输栈缺失：非 Windows 平台此前硬编码返回 `Ok(None)`。
- **架构设计与落地 (ADR-050)**:
  1. 跨平台外设地址抽象与解析器 (`crates/tetherly-core/src/ancs.rs`): 确立强类型 `BleAddress` 枚举（`Mac([u8; 6])` 与 `Uuid([u8; 16])`），实现通用解析器 `parse_ble_address` 与规范化字符串序列化 `to_canonical_string`；
  2. 守护进程启动链路与状态机所有权修复 (`crates/tetherly-node/src/bin/tetherly.rs`): 彻底修复丢弃 `transport` 的缺陷，在 `Node::start` 前将 `cfg.ancs = Some(transport)` 与 `cfg.ancs_peripheral = addr` 注入配置，打通 `spawn_ancs_pump` 与 `ancs_connect` 链路；
  3. macOS 原生 CoreBluetooth GATT 传输栈构建 (`crates/tetherly-node/src/ancs/macos.rs`): 基于 `objc2-core-bluetooth` 与 GCD 串行调度队列构建自包含 Objective-C 代理 `AncsDelegate`，独立后台线程运行 `CBCentralManager`，流式推送到 `AncsInbound`，串行化带响应写入 Control Point，断线退避重连；
  4. 运行时可观测性与控制面透视 (`crates/tetherly-node/src/runtime.rs`): `AncsReport` 扩充 `peripheral: Option<String>` 字段；
  5. 端到端集成与回归验证 (`tests/phase4.rs`): 新增 `test_ble_address_and_daemon_ancs_wiring_regression` 验证跨平台外设地址解析、配置注入与守护进程状态机初始化。
- **全套验证**: 全工作区 320+ 项测试全绿，`deny.toml` 检验通过，提交记录 `c62b66f`。

## 2026-10-19 (54) - ANCS full lifecycle governance, allowlist dynamic CRUD, event bus observability & dual-CLI isomorphism (v1.56 / ADR-049)
- Task: 落地 ANCS 苹果通知中心服务全生命周期治理、白名单规则动态增删查、状态机事件总线闭环与双端 CLI 全同构，消除 ANCS 链路状态与白名单配置变更的事件盲区，提供强类型 DTO 报告、动态管理控制原语、双端 CLI 完整子命令与 Web UI 局部即时刷新。
- **背景与缺陷根因**:
  在 Tetherly 陆续完成网络拓扑透视 (v1.55 / ADR-048)、远程屏幕与键鼠治理 (v1.54 / ADR-047)、文件提议治理 (v1.53 / ADR-046)、剪贴板审计 (v1.52 / ADR-045) 后，底层 ANCS 苹果通知中心服务与本地打开应用白名单治理暴露出一系列痛点：
  1. 事件总线静默脱节（Event Blackout on ANCS Transitions & Allowlist Mutations）：当 ANCS 客户端在 `Idle`、`Ready`、`Backoff` 之间流转，或者用户动态新增规则、移除规则、一键清空白名单时，底层从未广播任何 `UiEvent`，导致 Web UI 与 CLI 事件监听器失明；
  2. 白名单单向只增不减与治理原语缺失（Allowlist Immutability Defect）：`OpenAllowlist` 此前仅提供 `push` 追加操作，缺乏按 `app_id` 精准移除单条规则（`remove`）与全量清空规则（`clear`）的原语，导致用户配置错误规则后无法撤销或重置，造成潜在的恶意 URL scheme 驻留风险；
  3. 控制面缺乏结构化强类型 DTO 抽象（Missing Strongly-Typed DTOs）：`/api/ancs` 接口此前返回内联构造的松散 JSON，缺乏类型严谨的 `OpenRuleDto`、`AncsConnectReport`、`AncsDisconnectReport`、`AncsRuleReport`、`AncsClearRulesReport` 统一结构定义，未在 `lib.rs` 对外导出；
  4. 终端全同构控制与局部渲染闭环缺失：CLI 缺乏 `ancs disconnect`、`ancs remove`、`ancs clear` 等核心治理子命令；Web UI 缺少主动断开、行内删除规则与一键清空按键，且未基于 SSE 事件总线实施局部就绪刷新。
- **架构设计与落地 (ADR-049)**:
  1. 核心白名单原语扩展与 URL 注入守卫 (`crates/tetherly-core/src/allowlist.rs` & `error.rs`):
     - `OpenRule` 派生 `Serialize, Deserialize, PartialEq, Eq`；
     - `OpenAllowlist` 扩展 `remove(&mut self, app_id: &str) -> Option<OpenRule>` 与 `clear(&mut self) -> usize` 原语；
     - 严格遵循 ADR-008「打开应用的 URL 只来自本机规则」，仅允许本地自定义协议，拒绝 `http://`、`https://`、`file://` 等可能被外部恶意 payload 诱导打开的非本地 Scheme；
     - 核心错误新增 `CoreError::NoAncsTransport` 变体；
  2. 事件总线扩展与安全红线守卫 (`crates/tetherly-node/src/events.rs`):
     - 新增结构化变体 `UiEvent::AncsStateChanged { state: String, attached: bool, retry_at_ms: Option<u64> }`；
     - 新增结构化变体 `UiEvent::AncsRuleChanged { action: String, app_id: Option<String>, url: Option<String>, rules_count: usize }`；
     - 严格遵守 Zero-Plaintext 铁律，不携带任何苹果通知消息正文、标题或敏感属性，仅传递状态、附着标记、退避时间戳、动作与规则计数；
     - 提供 `ancs_state_changed` 与 `ancs_rule_changed` 构造辅助函数及关联方法；
  3. 结构化 DTO 抽象与运行时控制原语 (`crates/tetherly-node/src/runtime.rs`, `lib.rs`):
     - 定义并导出 `OpenRuleDto`、`AncsConnectReport`、`AncsDisconnectReport`、`AncsRuleReport`、`AncsClearRulesReport`，升级 `AncsReport` 支持 `rules: Vec<OpenRuleDto>`；
     - 实现 `ancs_connect_reported`、`ancs_disconnect_reported`、`add_open_rule_reported`、`remove_open_rule_reported`、`clear_open_rules_reported`，在发生状态流转或白名单变更时全生命周期广播 `UiEvent`；
  4. 轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`):
     - `GET /api/ancs` 重构为返回强类型 `AncsReport`；
     - `POST /api/ancs/connect` 返回强类型 `AncsConnectReport`；
     - `POST /api/ancs/disconnect` 返回强类型 `AncsDisconnectReport`；
     - `POST /api/ancs/allow` (及 `POST /api/ancs/rules/add`) 支持添加白名单规则并返回强类型 `AncsRuleReport`；
     - `DELETE /api/ancs/allow` (及 `POST /api/ancs/rules/remove` 与 `DELETE /api/ancs/rules`) 支持按 Query 或 JSON 体删除规则；
     - `POST /api/ancs/rules/clear` (及 `DELETE /api/ancs/rules/clear`) 支持一键清空规则；
     - 严格遵循 `127.0.0.1` 回环绑定与 `Host` 头守卫，拦截非回环请求并返回 HTTP 403 Forbidden (`loopback only`)；
  5. CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs`, `crates/tetherly-node/src/bin/tetherly.rs`):
     - `ancs` 命令族扩展 `status`、`connect`、`disconnect`、`allow`、`remove`、`clear` 子命令；
     - 格式化表格呈现与 `--json` 管道输出支持；
     - SSE `events` 监听器新增对 `[ancs_state_changed]` 与 `[ancs_rule_changed]` 的格式化着色打印；
  6. Web UI 实时动态刷新与状态徽标 (`ui/index.html`):
     - 新增「断开 ANCS」操作按键、规则列表行内即时删除标签 (`×`) 与「清空白名单」按钮；
     - 实现 `updateAncsStatus()` 轻量级局部渲染，在 SSE 分发循环中捕获 `ancs_state_changed` 与 `ancs_rule_changed` 驱动局部就绪刷新；
  7. 端到端集成测试全覆盖 (`tests/phase4.rs`):
     - 增设 `ancs_lifecycle_governance_rule_management_and_events_via_loopback_api`：完备验证 Host 403 阻断、SSE 事件订阅、GET 初始透视、POST 连接与状态变更广播、规则添加/删除/清空与规则事件广播、主动断开与退避重试事件广播全链路；
  8. 全工作区检验通过：
     - `cargo fmt --all --check` 干净通过；
     - `cargo clippy --workspace --all-targets -- -D warnings` 零警告通过；
     - `cargo test --workspace` 全部 308 项测试 100% 全绿；
     - `cargo deny check`（advisories, bans, licenses, sources）全部通过。

## 2026-10-19 (53) - v1.55 release matrix build, Windows CI broadcast race fix & online VPS live verification
- Task: 修复 Windows CI 并发 SSE 事件竞争条件，推进 v1.55 (ADR-048) 全平台 Release 构建与线上 VPS 生产环境端到端验证及环境清理。
- **Windows CI 竞争条件根因与修复**:
  - 在 GitHub Actions Windows 调度环境下，多线程运行时下的 `overlay_scan_loop` 与 `POST /api/net/refresh` 并发执行，`UiEvent::Overlay` 偶发抢先到达测试消费者的 SSE 流，导致 `tests/phase2.rs` 的单步匹配断言失败；
  - 增设 `wait_for_network_refreshed` 过滤轮询辅助函数，平滑过滤非目标广播事件并限定超时，消除并发时序竞争。
- **发布版本构建与多端矩阵 (v0.1.5 / SSOT v1.55)**:
  - 升级 `Cargo.toml` 与 `Cargo.lock` 版本至 `0.1.5`；
  - 触发 GitHub Actions CI (run `39174094878`): `android`、`check (macos-latest)`、`check (ubuntu-latest)`、`check (windows-latest)`、`deny` 全部通过；
  - 打标签 `v0.1.5` 并触发 GitHub Actions Release 工作流 (run `39174517893`)，四端 Release 产物 100% 成功构建并发布至 GitHub Release `v0.1.5`：
    - `tetherly-linux-x86_64`, `tetherly-cli-linux-x86_64` (Debian 11 bullseye glibc 2.31 容器编译)
    - `tetherly-windows-x86_64.exe`, `tetherly-cli-windows-x86_64.exe` (Windows x86_64)
    - `tetherly-macos-arm64`, `tetherly-cli-macos-arm64` (macOS Apple Silicon arm64)
    - `tetherly-android-arm64-v8a.apk`, `libtetherly_android-arm64-v8a.so` (Android arm64-v8a NDK)
- **线上 VPS 生产端到端验证与现场清理**:
  - 选取生产 VPS `aws-tokyo` (Debian 12 x86_64, glibc 2.36)；
  - 下载 Release `v0.1.5` 的 `tetherly-linux-x86_64` 与 `tetherly-cli-linux-x86_64`；
  - 启动隔离测试节点（端口 46717-46721，独立临时目录）；
  - 验证健康检查 `/api/health` 正常就绪；
  - 严格校验安全红线：伪造外部 `Host: evil.attacker.com` 请求被回环控制面阻断并返回 `403 Forbidden` (`loopback only`)；
  - 验证 `GET /api/net`、`POST /api/net/refresh`、`POST /api/net/probe` 返回强类型结构化 JSON 报告；
  - 验证 CLI 子命令：`tetherly net`、`tetherly net --refresh --json`、`tetherly screen status --json`、`tetherly input status --json`、`tetherly peers --json` 及其 `tetherly-cli` 镜像命令完全同构通过；
  - 验证 `tetherly stop` 优雅停止节点；
  - 严格清理现场：彻底移除 `/tmp/tetherly-test-v155`，核实无残留后台进程或文件。

## 2026-10-19 (52) - Network topology telemetry, dynamic NIC rescan governance, EasyTier sidecar observability & event bus synchronization (v1.55 / ADR-048)
- Task: 实现网络拓扑全景透视、网卡物理分类动态重探治理、EasyTier 侧车穿透可观测性与事件总线闭环，消除网络与虚拟网卡通道的事件盲区，提供强类型 DTO 报告、动态重探控制原语、双端 CLI 结构化排版及 Web UI 局部即时渲染。
- **背景与缺陷根因**:
  在 Tetherly 陆续完成远程屏幕与键鼠治理 (v1.54 / ADR-047)、文件提议治理 (v1.53 / ADR-046)、剪贴板审计 (v1.52 / ADR-045)、通知双向同步 (v1.51 / ADR-044) 后，底层网络接口拓扑与 EasyTier 侧车穿透可观测性暴露出一系列治理痛点：
  1. 事件总线静默脱节（Event Blackout on Network & Overlay Changes）：当宿主机经历网卡热插拔、WiFi/以太网热切换、VPN 拨号建立/断开，或者 EasyTier 侧车守护进程完成启动、穿透建立、节点发现或异常离线时，底层从不向节点事件总线广播任何 `UiEvent`，导致 Web UI 与 CLI 事件监听器失明，必须依赖高开销的主动轮询；
  2. 网卡物理分类静态锁定与动态重探能力缺失（Dynamic Rescan Primitive Missing）：节点启动时虽然基于 `lan.rs` 进行了网卡分类，但缺乏在不重启节点的前提下主动触发「即时全量网卡重探与 EasyTier 侧车再连接」的控制原语；
  3. 控制面缺乏结构化强类型 DTO 抽象（Missing Strongly-Typed DTOs）：`/api/net` 接口此前返回内联构造的松散 JSON，缺乏类型严谨的 `NetworkReport`、`IfaceReportDto`、`OverlayStatusReport` 与 `NetworkRefreshReport` 统一结构定义，未在 `lib.rs` 对外导出，导致 CLI 与外部集成客户端无法获得严格的类型校验保证；
  4. 终端全同构控制与局部渲染闭环缺失：CLI 缺乏 `net --refresh` 显式重探管道；Web UI 缺乏网络重探控制按钮与解耦全局轮询的局部即时渲染支持。
- **架构设计与落地 (ADR-048)**:
  1. 事件总线扩展与安全红线守卫 (`crates/tetherly-node/src/events.rs`):
     - 新增结构化变体 `UiEvent::NetworkRefreshed { iface_count: usize, listen_count: usize, overlay_present: bool, overlay_source: Option<String> }`；
     - 新增结构化变体 `UiEvent::OverlayStateChanged { present: bool, source: String, local_ips: Vec<String>, peer_count: usize }`；
     - 严格遵守 Zero-Plaintext 铁律，不泄露私网敏感信息，仅传递网卡分类统计、监听计数、侧车状态及对端节点数量；
     - 提供 `network_refreshed` 与 `overlay_state_changed` 构造辅助函数及关联方法；
  2. 结构化 DTO 抽象与运行时控制原语 (`crates/tetherly-node/src/runtime.rs`, `lib.rs`):
     - 定义并导出 `IfaceReportDto`、`OverlayStatusReport`、`NetworkReport`、`NetworkRefreshReport`；
     - 实现核心控制原语 `node.network_report_dto() -> NetworkReport` 与 `node.refresh_network().await -> NetworkRefreshReport`；
     - 运行时主动执行物理网卡重新枚举与 EasyTier 侧车连接探测，更新 `Shared.overlay`，并在发生状态突变时广播 `UiEvent::OverlayStateChanged`，在刷新完成时广播 `UiEvent::NetworkRefreshed`；
     - 后台 `overlay_scan_loop` 同步在侧车状态演进时派发 `UiEvent::OverlayStateChanged`；
  3. 轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`):
     - 重构 `GET /api/net` 返回强类型 `NetworkReport`；
     - 新增 `POST /api/net/refresh` 及别名 `POST /api/net/probe`，触发主动重探并返回强类型 `NetworkRefreshReport`；
     - 严格遵循 `127.0.0.1` 回环绑定与 `Host` 头守卫，拦截非回环请求并返回 HTTP 403 Forbidden (`loopback only`)；
  4. CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs`, `crates/tetherly-node/src/bin/tetherly.rs`):
     - `tetherly net` 与 `tetherly-cli net` 增设 `--refresh` (`-r`) 参数，支持主动触发网卡分类与 EasyTier 侧车重探；
     - 保持与现存输出及 `--json` 管道的完美后向兼容性，清晰打印分类网卡、监听状态、跳过原因、EasyTier 侧车状态、虚拟 IP 与 Peers；
     - SSE `events` 监听器新增对 `[network_refreshed]` 与 `[overlay_state_changed]` 的格式化着色打印；
  5. Web UI 实时动态刷新与状态徽标 (`ui/index.html`):
     - 在「网络与接口诊断」面板新增「探测刷新」(`netrefreshbtn`) 按钮与异步操作状态反馈；
     - 实现 `updateNetworkStatus()` 轻量级局部渲染，解耦全局状态轮询；
     - 在 SSE 事件流中捕获 `network_refreshed` 与 `overlay_state_changed` 事件，驱动网络与侧车卡片毫秒级自动就绪刷新；
  6. 端到端集成测试全覆盖 (`tests/phase2.rs`):
     - 增设 `dynamic_network_rescan_and_overlay_events_via_loopback_api`：完备验证初始透视、非回环 Host 403 守卫拦截、POST 动态探测重探、SSE 事件捕获与关联解析、Rust API 直接调用与节点优雅停机，确立 ADR-048，SSOT 升至 v1.55。

## 2026-10-19 (51) - Remote screen & shared input streaming lifecycle governance, real-time telemetry observability & event bus integration (v1.54 / ADR-047)
- Task: 实现远程屏幕与键鼠共享全生命周期治理、实时遥测可观测性与事件总线闭环，消除屏幕与键鼠通道的事件静默脱节与状态盲区，提供强类型 DTO 报告、活动对端追踪、双端 CLI 结构化排版及 Web UI 局部即时渲染。
- **背景与缺陷根因**:
  在 Tetherly 陆续完成文件提议治理 (v1.53 / ADR-046)、剪贴板审计 (v1.52 / ADR-045)、通知双向同步 (v1.51 / ADR-044)、黑名单治理 (v1.50 / ADR-043) 后，核心多媒体与控制通道——远程屏幕串流 (Phase 5 / 45720) 与键鼠输入共享 (Phase 3 / 45719) 成为仅存的状态黑盒与事件盲区：
  1. 事件总线静默脱节（Event Blackout）：底层状态在 `Idle`、`Requested`、`Allowed`、`Streaming` 间流转，或者键鼠共享连线/断开/焦点在 `Local` 与 `Remote` 之间切换（`input_seat`）时，底层从不向节点事件总线广播任何 `UiEvent`，导致 Web UI 与 CLI 事件监听器失明；
  2. 缺乏结构化遥测报告 DTO（Missing Typed Reports）：`/api/screen` 与 `/api/input` 接口此前返回松散内联 JSON，缺乏类型严谨的 `ScreenStatusReport`、`ScreenControlReport`、`InputStatusReport`、`InputControlReport` 定义；
  3. 活动对端与双向遥测透视缺失（Peer Tracking & Telemetry Blindspots）：Host 端未记录连入的受信任访客（Visitor），访客端未记录查看的宿主（Host），控制器与受控端未记录焦点对端 ID；发送端与接收端度量指标未统一归一化；
  4. 终端全同构控制与可观测性闭环缺失：CLI 缺少对结构化报告的整洁呈现与统一 `--json` 管道输出，SSE 事件监听器缺少 `[screen_state_changed]` 与 `[input_seat_changed]` 格式化高亮。
- **架构设计与落地 (ADR-047)**:
  1. 事件总线扩展与安全红线守卫 (`crates/tetherly-node/src/events.rs`):
     - 新增结构化变体 `UiEvent::ScreenStateChanged { state: String, allowed: bool, peer: Option<String> }`；
     - 新增结构化变体 `UiEvent::InputSeatChanged { seat: String, channel_open: bool, peer: Option<String> }`；
     - 严格遵守 Zero-Plaintext 铁律，不携带像素缓冲区、按键扫描码或光标敏感轨迹；提供 associated functions 与 standalone 辅助函数；
  2. 结构化 DTO 抽象与运行时控制原语 (`crates/tetherly-node/src/runtime.rs`, `lib.rs`):
     - 定义并导出 `ScreenStatusReport`、`ScreenControlReport`、`InputStatusReport`、`InputControlReport`；
     - 在 `Shared` 状态维护活动对端追踪锁 `screen_peer: Mutex<Option<DeviceId>>` 与 `input_peer: Mutex<Option<DeviceId>>`；
     - 实现 `screen_status_report`、`screen_allow_reported`、`screen_revoke_reported`、`screen_start_reported`、`screen_stop_reported`、`input_status_report`、`input_start_reported`、`input_stop_reported` 控制原语；
     - 在握手连入、许可授权、撤销授权、推流启停、键鼠连线断开与边缘跨越时全生命周期广播 `UiEvent`；
  3. 轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`):
     - 重构 `/api/screen` 与 `/api/input` 响应，完全由强类型 DTO 驱动，返回静态导出的结构化 JSON；
     - 严格 127.0.0.1 回环绑定与 `Host` 校验（403 on mismatch）；
  4. CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs`, `crates/tetherly-node/src/bin/tetherly.rs`):
     - `screen status` 与 `input status` 支持富文本高亮排版与 `--json` 原生管道输出；
     - `screen allow/revoke/start/stop` 与 `input start/stop` 打印操作确认与活动对端；
     - `events` 监听器新增对 `[screen_state_changed]` 与 `[input_seat_changed]` 的格式化着色打印；
  5. Web UI 实时动态刷新与状态徽标 (`ui/index.html`):
     - 在 SSE 事件总线分发循环中挂载 `screen_state_changed` 与 `input_seat_changed` 监听器；
     - 解耦重量级全节点快照 `render(s)`，支持屏幕与键鼠卡片根据 SSE 事件驱动局部即时渲染；
  6. 端到端集成测试全覆盖 (`tests/phase5.rs`):
     - 增设 `screen_and_input_lifecycle_governance_and_events_via_loopback_api`：完备验证初始透视、访客握手接入、宿主授权许可及事件派发、屏幕串流与遥测统计、授权撤销与事件派发、键鼠通道连接断开与席位变更、错误边界与优雅退出，确立 ADR-047，SSOT 升至 v1.54。

## 2026-10-18 (50) - File offer lifecycle governance, lightweight offer introspection, and auto-accept closed-loop (v1.53 / ADR-046)
- Task: 实现跨端文件传输提议生命周期治理、轻量级提议透视与终端全同构控制闭环，消除提议状态黑盒与重载全快照序列化开销，补齐受信任对端自动接收无人值守策略与双端 CLI/Web UI 批量治理。
- **背景与缺陷根因**:
  在 Tetherly v1.52 实现了跨端剪贴板生命周期治理、元数据遥测审计与历史可观测性闭环后，数据协同与安全体系得到大幅提升。但在核心应用层的跨端文件传输系统（File Ingress & Offer Lifecycle）中，暴露出提议状态黑盒、轻量级提议可观测性断层、批量决策能力缺失与无人值守自动化空白：
  1. 待确认文件提议缺乏轻量级透视端点（Heavy Snapshot vs Lightweight Offers Introspection）：
     - 此前客户端或外部工具若要感知「当前有哪些待确认的接收提议」，必须调用重量级的全节点状态快照 `GET /api/state`。该接口会序列化整个节点的所有会话、候选通知、路由表、全局指标与剪贴板状态，在轮询与低资源场景下开销巨大；
     - 缺乏专用的 `GET /api/file/offers`，无法针对单一提议执行按 `transfer_id` 的 $O(1)$ 精准透视，亦无法按来源对端 `peer` 进行定向筛选；
  2. 提议决策动作缺乏零明文结构化报告（Zero-Plaintext Structured Decision Reports）：
     - 现存接收（`POST /api/file/accept`）与拒绝（`POST /api/file/reject`）接口返回松散字典，未提供可追溯的 `FileDecisionReport` 与 `FileBatchDecisionReport`；控制端无法审计被确认或拒绝的文件名、字节大小、断点续传偏移与来源对端；
  3. 缺乏面向生产环境的批量提议治理原语（Batch Decision Governance）：
     - 当对端批量投递多份文件，或多个协作设备同时发起传输时，用户或守护进程缺乏一键「全部接收」（`--all`）、「全部拒绝」或按特定设备定向批处理的能力；
  4. 无人值守与受信任对端自动接收策略空白（Auto-Accept Policy for Trusted Peers）：
     - 在 NAS、无头运行（Headless Linux/macOS/Windows Daemon）或已严格通过 Noise 密钥配对的个人多设备协同场景下，每次对端发来文件都需要人工干预调用 API 确认，极大阻碍了无人值守工作流；
     - 严格遵守安全红线铁律（Verbatim Security Red Line）：「信任库只在 Noise transport 成功后写」与「仅允许受信任对端触发自动接收」。因此，自动接收策略必须严格受限于 `is_trusted(peer) == true`，严禁未经鉴权的外部设备绕过确认机制；
  5. 事件总线生命周期缺失（Missing Event Bus Lifecycle for Rejection & Policy Change）：
     - 当文件提议被拒绝（本地拒绝或对端拒绝）时，系统此前处于完全静默状态，从未派发任何 `UiEvent`；同时自动接收策略变更亦未在事件流中广播，导致 Web UI、SSE 订阅客户端与 CLI 监听器无法感知；
  6. 双端 CLI 与 Web UI 全同构控制闭环缺失：
     - `tetherly` 与 `tetherly-cli` 缺少 `offers`、`accept`、`reject`、`auto-accept` 命令群；Web UI 缺少待确认文件治理面板与自动接收配置开关。
- **架构设计与落地 (ADR-046)**:
  1. 结构化 DTO 报告设计 (`crates/tetherly-node/src/runtime.rs`, `lib.rs`):
     - `FileOfferDto`：封装 `transfer_id`、`filename`、`total_bytes`、`resume_from`、`source` 与 `timestamp_ms`；
     - `FileOffersReport`：封装 `ok`、`count`、`offers: Vec<FileOfferDto>` 与 `auto_accept_trusted: bool`；
     - `FileDecisionReport`：封装单项决策结果 `transfer_id`、`filename`、`total_bytes`、`resume_from`、`peer`、`accepted` 与 `timestamp_ms`；
     - `FileBatchDecisionReport`：封装批量决策结果 `accepted_count` / `rejected_count`、`total_evaluated` 与 `decisions` 列表；
     - `AutoAcceptReport`：封装自动接收策略状态 `enabled`、`auto_accepted_count` 与 `target: "trusted_peers_only"`；
  2. 事件总线扩展 (`crates/tetherly-node/src/events.rs`):
     - 新增 `UiEvent::FileRejected { transfer_id, source, reason }` 与 `UiEvent::FileAutoAcceptChanged { enabled }`；
  3. 运行时控制原语与受信对端自动接收策略 (`crates/tetherly-node/src/runtime.rs`):
     - 在 `Shared` 状态中增设原子控制位 `auto_accept_trusted: AtomicBool` 与原子计数器 `auto_accepted_count: AtomicU64`；
     - 实现 `Node::is_trusted(&self, peer: &str) -> bool`，基于 `trust_table` 判定 Noise 鉴权受信状态；
     - 在 `SessionEvent::FileOffer` 核心处理循环中集成自动接收判定：若 `auto_accept_trusted` 开启且 `is_trusted(peer)`，则自动触发异步 `user_accept_file` 并将 `auto_accepted_count` 递增；
     - 在 `SessionEvent::FileReject` 与本地主动拒绝时广播 `UiEvent::FileRejected`；
     - 增设 `file_offers`、`user_accept_file_reported`、`user_reject_file_reported`、`batch_accept_files`、`batch_reject_files`、`auto_accept_status` 与 `set_auto_accept` 原语；
  4. 轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`):
     - `GET /api/file/offers`：支持按 `?id=tfl_...`（404 on missing）与 `?peer=...` 查询过滤；
     - `POST /api/file/accept`：支持按 `id` 单项接收或 `{ "all": true, "peer": ... }` 批量接收；
     - `POST /api/file/reject`：支持按 `id` 单项拒绝或 `{ "all": true, "peer": ... }` 批量拒绝；
     - `GET /api/file/auto-accept` 与 `POST /api/file/auto-accept`：支持查询与切换受信对端自动接收策略；
     - 严格 127.0.0.1 回环绑定与 `Host: 127.0.0.1|localhost` 校验（403 on mismatch）；
  5. CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs` 与 `crates/tetherly-node/src/bin/tetherly.rs`):
     - `offers [--id <id>] [--peer <peer>] [--json] [--ui-port <port>]`：轻量级检索待确认文件提议；
     - `accept [--id <id>] [--all] [--peer <peer>] [--json] [--ui-port <port>]`：单项或批量接收文件提议；
     - `reject [--id <id>] [--all] [--peer <peer>] [--reason <r>] [--json] [--ui-port <port>]`：单项或批量拒绝文件提议；
     - `auto-accept [--enable] [--disable] [--json] [--ui-port <port>]`：查询或切换受信对端自动接收；
     - `events` 监听器增设 `[file_rejected]` 与 `[file_auto_accept_changed]` 格式化打印；
  6. Web UI 实时交互与治理看板 (`ui/index.html`):
     - 增设「待确认文件与提议治理」卡片，显示待确认文件列表、文件名、大小、断点续传标记与来源对端；
     - 增设单项接收/拒绝按钮、一键「全部接收」与「全部拒绝」按钮；
     - 增设「自动接收受信任对端」复选框开关与自动接收累计计数徽标；
     - SSE 监听器对接 `file_rejected` 与 `file_auto_accept_changed`，动态局部重绘；
  7. 端到端集成测试全覆盖 (`tests/phase1.rs`):
     - 增设 `file_offer_lifecycle_and_terminal_governance_via_loopback_api`：完备验证初始态、提议到达与透视、单项拒绝及事件广播、批量拒绝与报告、受信任对端自动接收生效及收件箱 SHA-256 完整性校验、错误边界（400/404）与优雅停机，确立 ADR-046，SSOT 升至 v1.53。
  8. 网络层取消安全与多路复用健壮性强化 (`crates/tetherly-net/src/codec.rs`, `session.rs`):
     - 设计并实现状态化 `TaggedReader` 环形缓冲状态机，彻底消除 Tokio `select!` 竞态丢弃分支时丢失 header/body 字节导致的帧解包失步；`ActiveSession` 全量集成 `TaggedReader`，并在 `tests/phase1.rs` 增设对端拒绝状态观察同步，GitHub Actions CI 5 平台矩阵 100% 全绿通过。
  9. Windows 平台异步端到端测试与生命周期健壮性加固 (`crates/tetherly-android/tests/client_e2e.rs`):
     - 针对 Windows CI Runner 虚拟化 CPU 在多测试并行压力下的 Argon2id 密钥派生耗时与线程调度延迟，将 `android_client_pairs_and_delivers_notify` 连线等待截止时间从 10s 调整为 30s；
     - 针对 `android_client_receives_a_quick_reply_once`，移除了对端 mock 服务端脆弱的固定 500ms 休眠设计，引入 `tokio::sync::oneshot::channel()` 双向生命周期同步，确保服务端在客户端完全取走快速回复帧并完成校验前维持会话存活。

## 2026-10-18 (49) - Clipboard lifecycle governance, metadata telemetry auditing, and history observability closed-loop (v1.52 / ADR-045)
- Task: 实现跨端剪贴板生命周期治理、元数据遥测审计与历史可观测性闭环，消除剪贴板事件流黑盒与明文泄漏风险，建立纯元数据受控环形缓冲区、主动安全清空复位与双端 CLI/Web UI 全同构控制面。
- **背景与缺陷根因**:
  在 Tetherly v1.51 实现了通知全生命周期治理与双向同步后，跨端数据协同网络愈发成熟。但在应用层跨端安全剪贴板系统（Clipboard Lifecycle & Telemetry）中，暴露出事件黑盒、可观测性断层、元数据遥测缺失与历史审计空白：
  1. 跨端剪贴板同步缺乏事件总线驱动与实时感知（Event Blackout）：
     - 当手机端或外部对端向桌面节点推送剪贴板帧（`SessionEvent::Clip(clip)` -> `apply_clip`）并写入本地系统剪贴板时，底层从未向节点事件总线广播任何 `UiEvent`，导致 Web UI、SSE 订阅客户端与 CLI 事件监听器对对端剪贴板摄入完全处于「失明」状态；
     - 本地主动发送剪贴板（`send_clipboard_to` / `send_clipboard_text`）同样未派发任何结构化事件，外部集成工具与诊断控制台无法通过统一事件流审计剪贴板出入站动作；
  2. 缺乏安全的纯元数据透视端点（Metadata-Only Status Inspection）：
     - 现存回环接口仅有 `GET /api/clip`，会直接返回明文文本（`text`）。在系统健康巡检、监控探针或状态展示场景下，调用方仅需要感知剪贴板状态（是否存在文本、字节大小、当前序号 `last_seq`、内容 SHA-256 哈希 `last_hash`、累计出入站次数），无需也不应将未经脱敏的明文文本暴露至日志或监控链路；
  3. 缺乏受控的轻量级历史审计环形缓冲区（Bounded History Ring Buffer）：
     - 节点缺乏剪贴板同步历史的可观测性。用户或客户端无法追溯「最近同步的剪贴板是何时发生、流向为何、来自哪个设备、大小是多少、校验哈希是什么」；
     - 严格遵守安全红线铁律（Verbatim Security Red Line）：「UiEvent 严禁携带短信正文、一次性码、配对码、剪贴板纯文本」与「公开 issue / 日志禁止贴短信正文、一次性码、配对码、剪贴板」。因此，历史审计记录必须是纯元数据结构（Zero-Plaintext），仅记录 `(seq, source, direction, bytes, hash, timestamp_ms)`，绝对不存储任何明文字符；
  4. 缺乏剪贴板主动清空与安全复位原语（Clipboard Reset & Purge）：
     - 用户处理完敏感信息（如一次性密码、凭证碎片）后，缺乏通过回环控制面一键清空本地系统剪贴板、重置内容哈希并销毁审计历史的机制；
  5. 回环端点与 CLI 缺乏规范化 RESTful 治理接口：
     - 缺少 `GET /api/clip/status`（安全纯元数据查询）；
     - 缺少 `GET /api/clip/history`（支持 `?limit=N` 的历史查询）；
     - 缺少 `POST /api/clip/clear` 与 `DELETE /api/clip`（清空剪贴板与复位审计缓冲区）；
     - 双端 CLI（`tetherly` 与 `tetherly-cli`）缺少 `clip status`、`clip history`、`clip clear` 命令，并在 `events` 监听器中缺少 `[clipboard_synced]` 与 `[clipboard_cleared]` 格式化高亮展示；
  6. Web UI 实时交互与历史可视化补全：
     - Web UI（`ui/index.html`）剪贴板卡片增设当前状态徽标、最近 5 条同步记录列表，以及一键「清空剪贴板」确认按钮，联动 SSE 事件动态局部刷新。
- **架构设计与落地 (ADR-045)**:
  1. 核心模型与元数据环形缓冲区 (`crates/tetherly-core/src/clip.rs`, `lib.rs`):
     - 定义 `ClipDirection`（`Inbound`, `Outbound`）与 `ClipMetaEntry` 纯元数据结构；
     - `ClipHub` 集成固定容量环形缓冲区 `MAX_CLIP_HISTORY: usize = 32`（`VecDeque<ClipMetaEntry>`）与 `inbound_count`, `outbound_count` 统计计数器；
     - 提供 `record_entry`（自动 FIFO 逐出，保证 $O(1)$ 内存界限）、`history(limit)`、`clear_history`、`reset_state` 与 `stats` 原语；
     - 增加核心单元测试 `history_and_bounded_ring_buffer` 验证容量约束与状态复位；
  2. 事件总线扩展与安全红线守卫 (`crates/tetherly-node/src/events.rs`, `runtime.rs`, `lib.rs`):
     - 新增 `UiEvent::ClipboardSynced { seq, source, direction, bytes, hash, timestamp_ms, sent_peers }` 与 `UiEvent::ClipboardCleared { timestamp_ms }`；严格遵守零明文原则（Zero-Plaintext），严禁包含文本内容或 OTP；
     - 定义结构化 DTO 报告：`ClipStatusReport`、`ClipHistoryReport`、`ClipMetaEntryDto` 与 `ClipClearReport`；
  3. 运行时可观测性闭环与控制原语 (`crates/tetherly-node/src/runtime.rs`):
     - `apply_clip` 闭环：入站写入系统剪贴板后，记录元数据至 `ClipHub`，递增 `inbound_count`，向事件总线广播 `UiEvent::ClipboardSynced`（inbound）；
     - `send_clipboard_to` / `send_clipboard_text` 闭环：出站投递至对端会话后，记录元数据至 `ClipHub`，递增 `outbound_count`，向事件总线广播 `UiEvent::ClipboardSynced`（outbound）；
     - `Node::clipboard_status`：零泄漏状态查询原语；
     - `Node::clipboard_history`：受控历史记录快照查询原语；
     - `Node::clear_clipboard`：安全清空系统剪贴板（设为 `""`），复位 `ClipHub` 哈希与历史，广播 `UiEvent::ClipboardCleared`；
  4. 轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`):
     - `GET /api/clip/status`：返回 `ClipStatusReport`（零明文）；
     - `GET /api/clip/history`：支持 `?limit=N`，返回 `ClipHistoryReport`；
     - `POST /api/clip/clear` 与 `DELETE /api/clip`：清空剪贴板并返回 `ClipClearReport`；
     - 严格 127.0.0.1 回环绑定与 `Host: 127.0.0.1|localhost` 校验（403 on mismatch）；
  5. CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs` 与 `crates/tetherly-node/src/bin/tetherly.rs`):
     - `clip status [--json] [--ui-port <port>]`：格式化呈现当前剪贴板状态（字节数、序号、SHA-256、流向累计）；
     - `clip history [--limit <n>] [--json] [--ui-port <port>]`：表格化呈现最近同步历史记录；
     - `clip clear [--json] [--ui-port <port>]`：清空系统剪贴板与复位确认；
     - `events` 监听器增设 `[clipboard_synced]` 与 `[clipboard_cleared]` 格式化打印；
  6. Web UI 实时交互与历史可视化 (`ui/index.html`):
     - 剪贴板卡片增设状态徽标（有效文本/空、大小、哈希缩写、流向计数）；
     - 增设「清空剪贴板」确认按钮；
     - 增设最近 5 条同步元数据条目（流向徽标、来源设备、大小、时间、SHA-256 缩写）；
     - SSE 监听器对接 `clipboard_synced` 与 `clipboard_cleared`，动态局部重绘；
  7. 端到端集成测试全覆盖 (`tests/phase1.rs`):
     - 增设 `clipboard_lifecycle_history_and_observability_via_loopback_api`：完备验证初始态、出站同步事件、入站同步事件、32 环形容量上限 FIFO 逐出、清空复位与优雅停机，确立 ADR-045，SSOT 升至 v1.52。

## 2026-10-18 (48) - Notification full-lifecycle governance, bidirectional dismissal synchronization, and notification observability closed-loop (v1.51 / ADR-044)
- Task: 实现通知全生命周期治理、跨端双向消除同步与通知历史可观测性闭环，彻底解决桌面消除通知后移动端常驻通知悬挂断层，完善多维批量治理与回环 RESTful 接口。
- **背景与缺陷根因**:
  在 Tetherly v1.50 建立了完整的对端黑名单与接入鉴权体系后，网络层安全边界已高度健壮。但在应用层跨设备通知流协同（Notification Lifecycle & Ingress）中，暴露出通知单向消除断层、跨端双向同步缺失与批量治理空白：
  1. 跨端通知消除断层与双向同步缺失：
     - 当桌面端用户或 Loopback 客户端消除通知候选时（`Node::dismiss_candidate` / `POST /api/dismiss`），此前仅在本地 `hubs.notify` 内存表中移除了 `Candidate`，从未通过活动 Noise 隧道向源对端（如 Android 手机）发送 `TYPE_NOTIFY_DISMISS` 协议帧（`NotifyDismiss { app_id, uid }`）。导致手机端状态栏中的通知依然挂载，无法达成真正的「桌面已读/清除，手机同步消除」；
     - 反向链路同样存在断层：当手机端主动消除通知并在 Noise 会话接收到 `SessionEvent::Dismiss(d)` 时，桌面节点虽然调用了 `hubs.notify.dismiss_wire(peer, &d)`，但并未向本地事件总线派发 `UiEvent::CandidateDismissed`，导致 Web UI、终端监听器与外部 Loopback 订阅者对消除事件毫无感知，界面依然残留已消除通知；
  2. 缺乏多维批量消除与筛选治理原语：
     - 现存接口仅支持按单一 `id`（`candidate_id`）逐个消除，缺少面向生产环境的批量治理能力：按对端设备过滤并全量消除（`dismiss_by_source` / `peer`）、按应用包名/标识批量消除（`dismiss_by_app` / `app_id`）、一键清空全量待处理通知（`clear_candidates` / `all`）；缺少轻量级结构化 DTO 报告（`CandidateDismissReport` 与 `BatchDismissReport`）；
  3. 回环端点与 CLI 缺乏规范化 RESTful 治理接口：
     - 现有回环路由仅有非标 `POST /api/dismiss`，缺失规范的 RESTful 路由（`POST /api/notify/dismiss` 与 `DELETE /api/notify/candidates`）；`GET /api/notify/candidates` 缺乏按 `peer`、`app_id`、`has_otp` 的维度过滤；CLI 客户端缺乏高级过滤与批量消除命令群（`--all`, `--peer`, `--app-id`）；
  4. 事件总线扩展与 Web UI 实时动态联动：
     - 增设 `UiEvent::CandidateDismissed { id, source, app_id, uid }`；Web UI 通知列表支持消除某条通知时实时从 DOM 剔除，同时监听 SSE `candidate_dismissed` 事件实现多标签页无缝局部重绘，并增设「清空通知」一键消除按钮；
  5. 安全红线铁律（Verbatim Security Red Line）：
     - 「公开 issue / 日志禁止贴短信正文、一次性码、配对码、剪贴板」；
     - 「信任库只在 Noise transport 成功后写」；
     - 「Loopback UI 45716 仅绑定 127.0.0.1，校验 Host: 127.0.0.1|localhost (403 on mismatch); 严禁绑定 0.0.0.0 / ::」；
     - `UiEvent` 严禁携带短信正文、一次性码、配对码、剪贴板纯文本，`CandidateDismissed` 事件严禁携带任何通知正文或 OTP 验证码内容，仅允许携带结构化元数据标识符（`id`, `source`, `app_id`, `uid`）。
- **架构设计与落地 (ADR-044)**:
  1. 核心通知模型扩展 (`crates/tetherly-core/src/notify.rs`):
     - `Candidate` 结构体派生 `Clone`；
     - `take_candidate(&mut self, id: &str) -> Option<Candidate>`：原子提取并返回候选结构体，完整保留 `source` (DeviceId)、`app_id` 与 `uid` 元数据供网络层定向广播；
     - `dismiss_by_source`、`dismiss_by_app`、`clear_candidates`：批量提取并移除候选；
     - `dismiss_wire_candidates`：按对端与 wire 载荷定位移除，并返回逐出的候选列表以驱动事件总线；
  2. 事件总线与结构化 DTO 抽象 (`crates/tetherly-node/src/events.rs`, `runtime.rs`, `lib.rs`):
     - `UiEvent::CandidateDismissed { id, source, app_id, uid }`：闭环消除事件派发，严格不包含正文与 OTP；
     - `CandidateDismissReport { ok, id, source, app_id, uid, notified_remote }`；
     - `BatchDismissReport { ok, count, ids, notified_remote_count }`；
     - `DismissFilter { id, peer, app_id, all }`；
  3. 运行时跨端双向同步与批量消除控制原语 (`crates/tetherly-node/src/runtime.rs`):
     - `Node::dismiss_candidate_sync(&self, id: &str) -> Result<CandidateDismissReport, CoreError>`：原子提取候选并向对应活动会话投递 `NotifyDismiss` 帧，派发本地 `UiEvent::CandidateDismissed`；
     - `Node::dismiss_candidates(&self, filter: &DismissFilter) -> Result<BatchDismissReport, CoreError>`：支持按 `all`、`peer`、`app_id` 或 `id` 批量提取匹配候选，分别向对应的源设备活动会话投递 `NotifyDismiss` 帧并派发本地事件；
     - `SessionEvent::Dismiss(d)` 处理分支闭环：调用 `dismiss_wire_candidates` 并向本地派发 `UiEvent::CandidateDismissed`；
     - `AncsEvent::Removed` 分支闭环：派发 `UiEvent::CandidateDismissed`；
     - `Node::candidates_filtered(&self, peer, app_id, has_otp)`：多维条件过滤查询；
  4. 轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`):
     - `GET /api/notify/candidates`：支持 Query 参数 `peer`, `app_id`, `has_otp`；
     - `POST /api/dismiss`：升级内部调用 `dismiss_candidate_sync` 并返回 `CandidateDismissReport`；
     - `POST /api/notify/dismiss` 与 `DELETE /api/notify/candidates`：统一规范端点，自动兼容单 ID 与批量（`peer`/`app_id`/`all`）消除；
  5. CLI 双端全同构升级 (`tetherly` 与 `tetherly-cli`):
     - `dismiss` 命令群支持 `[id]`、`--peer <peer>`、`--app-id <app>`、`--all`、`--json`、`--ui-port <port>`；
     - `candidates` 命令群支持 `--peer <peer>`、`--app-id <app>`、`--has-otp <bool>`、`--json`、`--ui-port <port>`；
     - `events` 监听器高亮格式化输出 `[candidate_dismissed]`；
  6. Web UI 实时交互与按键闭环 (`ui/index.html`):
     - 监听 `candidate_dismissed` SSE 事件，动态局部重绘；
     - 候选卡片「忽略」按钮乐观置灰并调用 `/api/notify/dismiss`；
     - 增设「清空通知」按键，一键触发全量清空；
  7. 端到端集成测试全覆盖 (`tests/phase1.rs`):
     - `notification_bidirectional_dismissal_and_lifecycle_governance_via_loopback_api` 完备验证。

## 2026-10-17 (47) - Peer blacklist, connection defense, and dynamic admission access control governance (v1.50 / ADR-043)
- Task: 实现对端黑名单访问控制、恶意连接防御与动态接入鉴权治理，建立黑名单持久化拦截、主动会话秒级掐断与双端回环控制闭环。
- **背景与缺陷根因**:
  在 Tetherly v1.49 实现了对端连接会话全域治理与主动按需直连后，网络拓扑具备了高度灵活的受信任会话控制能力。但在跨网段多设备互联与开放局域网/虚网组网中，暴露出仅有受信任白名单、缺乏黑名单防御与动态接入鉴权治理的严重安全防护短板：
  1. 缺少对端黑名单拒绝与拦截机制：此前系统仅有基于 `trusted.json` 的白名单信任判定；一旦某台设备发生异常、行为不受信、存在端口恶意探测或受到中间人骚扰，用户即使通过 `unpair` 解绑设备，对方依然可以不断发起 Noise 握手和 PIN 码配对探测，对节点控制通道造成 socket 资源耗尽与日志泛洪；
  2. 缺乏黑名单持久化存储与原子落盘：节点缺乏独立的 `blocked.json` 黑名单持久化机制，无法记录被封禁的设备 ID（`device_id`）、封禁原因（`reason`）与时间戳（`blocked_at`），进程重启后防御状态完全丢失；
  3. 握手与拨号链路缺乏黑名单鉴权门禁：
     - 在被动接收端（`accept_one`）：完成 Noise transport 握手提取对方真实认证身份后，缺少黑名单前置检查，无法在挂载会话前主动扼杀连接；
     - 在主动发起端（`dial` 与 `connect_peer`）：若对端已被拉黑，依然会尝试发起 TCP 连接并消耗网络带宽；后台自动重连定时器（`spawn_reconnect`）也会死循环尝试向被拉黑设备重连；
  4. 缺乏屏蔽正在通信设备的主动断连与清理机制：拉黑某个当前在线的对端时，必须同步掐断活动 TCP 信道、清理会话映射、驱逐 `live` 列表、重置光标焦点，并向事件总线广播 `UiEvent::PeerDown`；
  5. 缺少轻量级回环控制面端点：外部管理面板与桌面前端缺少 `GET /api/peers/blocked`、`POST /api/peers/block` 与 `POST /api/peers/unblock` 接口；
  6. CLI 双端与 Web UI 缺失黑名单控制能力：双端命令行缺少 `block`、`unblock` 与 `blocked` 子命令，Web UI 无法直观查看黑名单设备列表与一键拉黑/解封。
- **架构设计与落地 (ADR-043)**:
  1. 错误语义与异常边界扩展 (`crates/tetherly-core/src/error.rs`):
     - 增设 `CoreError::PeerBlocked(String)` 变体，将黑名单拦截与鉴权失败明确暴露为结构化错误，避免与一般网络 IO 超时或密钥错误混淆；
  2. 黑名单持久化存储与原子落盘 (`crates/tetherly-node/src/store.rs`):
     - 定义结构体 `BlockedPeer { device_id, reason, blocked_at }` 与 `BlockedTable { peers }`；
     - 常量 `BLOCKED_FILE = "blocked.json"`；
     - 提供 `load_blocked`, `save_blocked`, `add_blocked_peer`, `remove_blocked_peer`, `is_peer_blocked` 原语；采用原子写入（写入临时文件后 `fs::rename`）并设置 POSIX 权限（`0600`）；
  3. 节点运行时动态黑名单同步与接入鉴权门禁 (`crates/tetherly-node/src/runtime.rs`):
     - 在 `Shared` 结构体增设 `blocked: Mutex<BlockedTable>` 内存缓存；
     - 定义 DTO 报告：`PeerBlockReport`, `PeerUnblockReport`, `BlockedPeersReport`；
     - `Node::block_peer(&self, query: &str, reason: Option<&str>) -> Result<PeerBlockReport, CoreError>`：支持设备 ID、友好别名或广播名智能解析（严禁拉黑本机）；写入 `blocked.json`；若对端正在通信，瞬间弹出并丢弃 `Shared.sessions` 发送端切断活动 TCP 信道、驱逐 `Shared.live`、复位输入焦点（`input_seat`），并向事件总线派发 `UiEvent::PeerDown` 与 `UiEvent::PeerBlocked`；
     - `Node::unblock_peer(&self, query: &str) -> Result<PeerUnblockReport, CoreError>`：从黑名单解封并落盘，派发 `UiEvent::PeerUnblocked`；
     - `Node::blocked_peers_report(&self) -> BlockedPeersReport`：列出所有封禁设备；
     - 被动接入门禁：在 `accept_one` 完成 Noise 握手认证提取身份后，立即检查 `Shared.blocked`，若命中直接返回 `CoreError::PeerBlocked`，严禁写入信任库、严禁缓存路由、严禁挂载会话；
     - 主动拨号守卫：`Node::connect_peer` 与后台 `spawn_reconnect` 在检查或拨号前核验黑名单，被拉黑对端立即拒绝或跳过；
  4. 轻量级回环 HTTP 控制面端点 (`crates/tetherly-node/src/uihttp.rs`):
     - `GET /api/peers/blocked`：查询黑名单设备列表；
     - `POST /api/peers/block`：支持 JSON 体 `{"peer": "...", "reason": "..."}` 与 Query 参数 `?peer=...&reason=...`（支持 DELETE 兼容）；
     - `POST /api/peers/unblock`：支持 JSON 体 `{"peer": "..."}` 与 Query 参数 `?peer=...`（支持 `DELETE /api/peers/block` 兼容）；
     - 严格守卫安全红线：仅绑定 `127.0.0.1`，强校验 `Host: 127.0.0.1|localhost`；
  5. CLI 双端全同构升级 (`crates/tetherly-node/src/bin/tetherly.rs` 与 `bins/tetherly-cli/src/main.rs`):
     - `block`: `--peer <PEER> [--reason <REASON>] [--json] [--ui-port <PORT>]`；
     - `unblock`: `--peer <PEER> [--json] [--ui-port <PORT>]`；
     - `blocked`: `[--json] [--ui-port <PORT>]`；
     - 终端 SSE 事件输出同步覆盖 `[peer_blocked]` 与 `[peer_unblocked]`；
  6. Web UI 访问控制面板与按键闭环 (`ui/index.html`):
     - 增设「访问控制与黑名单治理」独立卡片与一键屏蔽弹框交互；
     - 在线设备卡片与受信任设备卡片均增设「屏蔽」按键；
     - 黑名单设备列表呈现设备 ID、别名、屏蔽原因与时间，并提供「解除屏蔽」按键；
     - 接入 SSE `peer_blocked` 与 `peer_unblocked` 驱动界面实时局部重绘；
  7. 端到端集成测试全绿覆盖 (`tests/phase1.rs`):
     - 增设 `peer_blacklist_and_access_control_governance_via_loopback_api`：覆盖初始状态查询、屏蔽对端、活动 TCP 会话秒级掐断、SSE `PeerDown` 与 `PeerBlocked` 广播、`blocked.json` 持久化核验、主动拨号被拒 (400 / PeerBlocked)、被动接入 Noise 握手后即时阻断、解除屏蔽、解封后重新直连建立、以及禁止拉黑本机等边界防御全流程。
- **发布与文档同步**:
  - SSOT `docs/DEVELOPMENT.md` 升至 v1.50，确立 ADR-043。
  - Obsidian 知识库 `tetherly 开发文档.md` 与路由表 `AI-DOC-ROUTER.md` 同步更新。

## 2026-10-17 (46) - Peer connection session governance, on-demand direct connect and graceful disconnect (v1.49 / ADR-042)
- Task: 实现对端连接会话全域治理、主动按需直连与优雅断开控制闭环，彻底解耦会话状态与配对信任关系。
- **背景与缺陷根因**:
  在 Tetherly v1.48 实现了对端能力全景透视与特性协商后，多通道交互具备了动态边界治理能力。但在日常多节点拓扑运行中，暴露出会话生命周期与信任关系强耦合、主动连接能力缺失的关键运维痛点：
  1. 会话断连与信任解绑混淆：此前仅存在 `POST /api/peers/unpair`（以及 CLI `unpair`），一旦调用便会彻底清除 `trusted.json` 中的对称密钥与信任关系，并抹除 `routes.json` 物理寻址缓存；若用户仅希望暂时断开某台设备的活动 TCP 会话（例如切换网络、开会节能或暂停多端同步），此前完全无法在保留配对信任的前提下优雅断开；
  2. 缺少按需主动发起直连原语：已配对受信任设备断线后，系统完全依赖后台 30 秒轮询定时器（`spawn_reconnect`）尝试回拨；若后台重连被用户禁用（`--reconnect false`）或对端刚上线，用户无法主动下达指令要求节点立即与对端建立连接，必须等待长达 30 秒甚至重启守护进程；
  3. 缺少轻量级回环控制端点：外部运维脚本与桌面前端缺少 `POST /api/peers/connect` 与 `POST /api/peers/disconnect` 原语，无法精准控制指定对端的会话在线状态；
  4. 双端命令行缺失连接与断开子命令：`tetherly` 与 `tetherly-cli` 仅有初次配对用的 `dial`，缺少面向已受信任设备的 `connect` 与 `disconnect` 命令群；
  5. Web 前端卡片缺少操作按键：Web UI 对端卡片无法一键手动断开在线会话，离线受信任卡片无法一键触发重新连接。
- **架构设计与落地 (ADR-042)**:
  1. 对端会话生命周期模型解耦与 DTO 抽象 (`PeerConnectReport`, `PeerDisconnectReport`):
     - `PeerConnectReport`: `{ ok, device_id, name, alias, addr, connected, already_connected }`；
     - `PeerDisconnectReport`: `{ ok, device_id, name, alias, disconnected, already_disconnected }`；
     - 严格区分「临时断开会话 (disconnect)」与「永久撤销信任 (unpair)」：disconnect 仅断开活动 TCP 传输信道，绝不修改 `trusted.json` 与 `routes.json`；
  2. 节点运行时主动连接与优雅断开控制原语 (`runtime.rs`):
     - `Node::connect_peer(&self, query: &str, direct_addr: Option<&str>) -> Result<PeerConnectReport, CoreError>`：支持基于设备 ID、别名或广播名的智能解析，若已在线幂等返回 `already_connected: true`；若提供显式 `direct_addr` 校验套接字与非零端口并更新 `routes.json` 缓存，未提供则从 `routes.json` 自动解析物理地址；异步触发 `dial(addr)` 握手建连；
     - `Node::disconnect_peer(&self, query: &str) -> Result<PeerDisconnectReport, CoreError>`：智能解析定位对端，若已离线幂等返回 `already_disconnected: true`，否则从 `Shared.sessions` 移除发送端关闭信道、从 `Shared.live` 驱逐、复位输入焦点（`input_seat`）并向事件总线广播 `UiEvent::PeerDown`；
  3. 轻量级回环控制面端点 (`uihttp.rs`):
     - `POST /api/peers/connect`：支持 JSON 体 `{"peer": "...", "addr": "..."}` 与 Query 参数 `?peer=...&addr=...`；未知对端返回 404，寻址失败或地址非法返回 400；
     - `POST /api/peers/disconnect`：支持 JSON 体 `{"peer": "..."}` 与 Query 参数 `?peer=...`；未知对端返回 404；
     - 严格守卫安全红线：仅绑定 `127.0.0.1`，强校验 `Host: 127.0.0.1|localhost`；
  4. CLI 双端全同构升级 (`tetherly connect/disconnect`, `tetherly-cli connect/disconnect`):
     - `connect`: `--peer <PEER> [--addr <IP:PORT>] [--json] [--ui-port <PORT>]`；
     - `disconnect`: `--peer <PEER> [--json] [--ui-port <PORT>]`；
  5. Web UI 会话治理按键与交互闭环 (`ui/index.html`):
     - 在线设备卡片增设「断开」交互按键，点击调用 `POST /api/peers/disconnect`；
     - 受信任设备卡片依据 `online` 状态动态呈现「断开」或「连接」按键，点击分别调用对应回环接口；
     - 接入 SSE `peer_up` 与 `peer_down` 事件驱动界面实时局部重绘；
  6. 端到端集成测试全绿覆盖 (`tests/phase1.rs`):
     - 增设 `peer_active_connect_and_disconnect_lifecycle_via_loopback_api`：覆盖已在线状态检测、断开会话、SSE `PeerDown` 捕获、信任保持、幂等二次断开、按需直连重连、显式地址连接、以及非法地址与未知对端防御全流程。
- **发布与文档同步**:
  - SSOT `docs/DEVELOPMENT.md` 升至 v1.49，确立 ADR-042。
  - Obsidian 知识库 `tetherly 开发文档.md` 与路由表 `AI-DOC-ROUTER.md` 同步更新。

## 2026-10-17 (45) - Peer capabilities introspection, multi-dimensional feature negotiation, and dynamic capability governance (v1.48 / ADR-041)
- Task: 实现对端能力全景透视、多维特性协商与动态功能边界治理，打通应用层功能黑盒与动态特性启闭链路。
- **背景与缺陷根因**:
  在 Tetherly v1.47 实现了全协议网络流量透视与无锁原子遥测度量后，系统具备了微秒级网络吞吐可观测性。但在应用层多通道（通知同步、剪贴板共享、文件传输、键鼠控制、远程屏幕）交互演进中，暴露出对端功能能力黑盒与特性协商可观测性断层的严重架构缺陷：
  1. 对端能力完全丢弃与黑盒化：协议虽然定义了 `TYPE_CAPS_UPDATE` 帧与 `CapsUpdate { caps: Vec<String> }`，但在 `runtime.rs` 接收到对端 `CapsUpdate` 时，仅提取了端口号（`fileport=`, `inputport=`, `screenport=`），将所有的核心功能特性标签（`notify`, `clip`, `file`, `input`, `screen` 等）完全抛弃；`LivePeer` 结构体缺少 `caps` 存储字段，系统在连接建立后对对端究竟支持哪些特性一无所知；
  2. 缺少能力全景透视端点：`GET /api/peers` 与 `GET /api/state` 均未暴露对端能力列表，外部控制面无法获知对端已就绪的功能矩阵；缺少专用轻量级回环端点 `GET /api/caps`（透视本机支持并对外广播的能力清单）与 `GET /api/peers/caps`（查询所有对端或指定对端的能力矩阵）；
  3. 缺少动态能力热更新与即时广播机制：本机能力此前静态写死，无法在运行时按需启闭某项功能（例如在受限场景动态关闭屏幕推流或剪贴板共享）；缺乏 `Node::broadcast_caps()` / `POST /api/caps` 动态热更新并向所有活动 Noise 会话广播最新能力帧的原语；
  4. 事件总线缺少能力变更事件：对端更新能力或本机广播能力变更时，事件总线没有对应的 `UiEvent::PeerCapsChanged`，导致外部控制面与 UI 视图无法实时响应；
  5. 双端命令行与 Web UI 缺失能力透视与指示徽标：`tetherly` 与 `tetherly-cli` 缺乏 `caps` 命令群，`peers` 列表缺少能力标签；Web UI 对端卡片缺少能力徽标（`[通知]` `[剪贴板]` `[文件]` `[输入]` `[屏幕]`）。
- **架构设计与落地 (ADR-041)**:
  1. 能力模型与纯特性解耦 (`LocalCapsReport`, `PeerCapsDto`, `PeersCapsReport`, `extract_features`): 在 `runtime.rs` 与 `lib.rs` 引入纯特性提取工具函数 `extract_features`，过滤包含 `=` 的端口声明，提取唯一排序的特性列表；扩展 `LivePeer` 增设 `caps: Vec<String>`。
  2. 动态能力状态治理与热广播 (`Shared.local_caps`, `Node::update_local_caps`, `Node::broadcast_caps`): 在 `Shared` 中维护动态能力集（默认 `notify`, `clip`, `file`, `input`, `screen`），支持在运行时按需启闭特性；组装最新 `CapsUpdate` 帧全量异步广播至所有活动的 Noise 会话，并派发本地 `UiEvent::PeerCapsChanged`。
  3. 轻量级回环 HTTP 控制面端点 (`crates/tetherly-node/src/uihttp.rs`): `GET /api/caps` 返回本机能力画像；`POST /api/caps` 与 `PUT /api/caps` 支持 `{"enable": [...], "disable": [...]}` 动态热更新与广播；`GET /api/peers/caps` 支持 `?peer=<id_or_alias>` 智能解析查询；升级 `GET /api/peers` 与 `GET /api/state` 暴露 `caps` 与 `features` 数组；严格红线守卫 127.0.0.1+Host 校验与合法字符白名单。
  4. CLI 双端全同构升级 (`crates/tetherly-node/src/bin/tetherly.rs` 与 `bins/tetherly-cli/src/main.rs`): 新增 `Cmd::Caps`，支持 `--peer <PEER>`, `--enable <CAP>`, `--disable <CAP>`, `--json`, `--ui-port <PORT>`；升级 `peers` 命令输出能力徽标；SSE 事件监听器接入 `peer_caps_changed`。
  5. Web UI 能力看板与交互控制 (`ui/index.html`): 增设「节点能力与特性边界」卡片，提供快捷按键动态启闭特性；对端卡片增设多维特性徽标胶囊（`[通知]`, `[剪贴板]`, `[文件]`, `[输入]`, `[屏幕]`）；接入 SSE `peer_caps_changed` 实现无感局部刷新。
  6. 端到端集成测试全覆盖 (`tests/phase1.rs`): 增设 `peer_capabilities_negotiation_and_introspection_via_loopback_api` 完备验证初态查询、对端能力提取、智能解析检索、动态启闭热广播、SSE 变更事件及非法输入边界防御全流程。
- **发布与文档同步**:
  - SSOT `docs/DEVELOPMENT.md` 升至 v1.48，确立 ADR-041。
  - Obsidian 知识库 `tetherly 开发文档.md` 与路由表 `AI-DOC-ROUTER.md` 同步更新。
  - GitHub Actions 5 平台 CI 矩阵全绿验证。

## 2026-10-17 (44) - Unified network traffic & protocol telemetry observability and runtime resets (v1.47 / ADR-040)
- Task: 实现全协议网络流量透视、多通道遥测度量与运行时可观测性闭环，打通四大 TCP 通道与核心协议帧的可观测性盲区。
- **背景与缺陷根因**:
  在完成 v1.46 物理寻址路由表全景透视与静态注入后，Tetherly 已经具备了跨子网、NAT 穿越与点对点安全网格的稳固网络拓扑。然而在四维多通道（控制 45717、文件 45718、键鼠 45719、屏幕 45720）长效运行过程中，存在全局网络流量与协议消息遥测的重大可观测性盲区：
  1. 全局网络流量黑盒：除了大文件传输模块具备会话级 `TransferStats` 外，对整机的出入站网络吞吐（总 TX/RX 字节数、数据帧包数）缺乏统一度量与聚合；
  2. 四维多通道带宽分布不可知：控制 (45717)、文件 (45718)、输入 (45719)、屏幕 (45720) 各自消耗了多少带宽缺乏细分度量，无法定位网络拥堵或高频传输瓶颈；
  3. 协议消息遥测计数缺失：Ping/Pong 探活、Notification 通知、Clipboard 剪贴板、FileOffer 文件报价等核心协议帧缺乏全局调用计数，会话建连数与握手失败率处于黑盒状态；
  4. 缺少轻量级遥测回环查询与基准重置端点：外部监控探针无法一键获取全维度网络指标快照，也无法在基准性能测试中原子清零历史数据；
  5. 双端命令行与 Web UI 缺乏遥测面板：`tetherly` 与 `tetherly-cli` 缺失 `metrics`/`traffic` 命令群，Web 前端缺少流量透视卡片。
- **架构设计与落地 (ADR-040)**:
  1. `TrafficTelemetry`, `ChannelTrafficDto`, `ProtocolMessagesDto`, `TrafficReport` (`crates/tetherly-node/src/telemetry.rs`, `lib.rs`): 基于 `AtomicU64` 与 `Ordering::Relaxed` 构建高性能无锁度量引擎，以零锁争用与 $O(1)$ 时间复杂度在网络传输热路径上原子累加四通道字节与数据帧数，记录 Ping/Pong、Notification、Clipboard、FileOffer 消息计数与会话生命周期指标（`sessions_established`, `sessions_closed`, `handshakes_failed`）。
  2. 全通道 I/O 链路埋点集成 (`crates/tetherly-node/src/runtime.rs`): 控制通道帧收发与协议帧类型原子累加；文件通道流式/内存传输字节累计；键鼠事件与屏幕压缩帧收发字节与帧数累计；会话建立、断开与握手失败状态准确跟踪。
  3. 轻量级回环 HTTP 控制面端点 (`crates/tetherly-node/src/uihttp.rs`): `GET /api/metrics` 与 `GET /api/traffic` 返回结构化 `TrafficReport`；`POST /api/metrics/reset` 与 `POST /api/traffic/reset` 原子清零所有计数器，向事件总线广播 `UiEvent::MetricsReset`；严格守卫安全红线（仅绑定 127.0.0.1，校验 Host，数据纯数字无敏感载荷）。
  4. CLI 双端全同构升级 (`crates/tetherly-node/src/bin/tetherly.rs` 与 `bins/tetherly-cli/src/main.rs`): 新增 `Cmd::Metrics` 与 `Cmd::Traffic`，支持 `--reset`, `--json`, `--ui-port`；终端表格自适应格式化输出 B/KiB/MiB/GiB，绘制四通道带宽分布柱状占比。
  5. Web UI 实时遥测仪表盘 (`ui/index.html`): 增设「网络流量与全协议遥测」面板，集成总出入站吞吐徽标、四通道带宽分布进度指示、协议核心消息统计芯片、一键重置按键与 SSE `metrics_reset` 实时局部刷新。
  6. 端到端集成测试全覆盖 (`tests/phase1.rs`): 增设 `traffic_metrics_inspection_and_telemetry_reporting_via_loopback_api` 完备验证初态查询、控制帧交互递增、流式文件传输度量、POST 重置及 SSE 事件响应与清零全流程。
- **发布与文档同步**:
  - SSOT `docs/DEVELOPMENT.md` 升至 v1.47，确立 ADR-040。
  - Obsidian 知识库 `tetherly 开发文档.md` 与路由表 `AI-DOC-ROUTER.md` 同步更新。
  - GitHub Actions 5 平台 CI 矩阵全绿验证（Run ID `39085307597`）：
    - `deny` (advisories, bans, licenses, sources): ✓ in 48s
    - `android` (arm64-v8a NDK & debug apk): ✓ in 1m44s
    - `check (ubuntu-latest)`: ✓ in 3m49s
    - `check (windows-latest)`: ✓ in 5m20s
    - `check (macos-latest)`: ✓ in 3m13s
  - 核心安全红线严密守卫：无敏感载荷/凭据泄露，Loopback 仅绑定 127.0.0.1 且校验 Host，零 LGPL 链接。

## 2026-10-17 (43) - Route table governance, physical addressing persistence, and static injection via loopback API (v1.46 / ADR-039)
- Task: 实现物理寻址路由表全景透视、静态路由注入与断线自动重连治理，打通跨子网与无广播多物理网段连接壁垒。
- **背景与缺陷根因**:
  在 v1.18 至 v1.45 的演进中，Tetherly 建立了基于 mDNS 组播广播、Noise 握手及自动重连机制的点对点安全网格。然而在实际多宿主物理网络（跨物理子网、隔离 VLAN、禁止组播的公共 Wi-Fi、或云主机/NAT 穿越场景）中，暴露出以下显著缺陷：
  1. 物理寻址路由黑盒与可观测性断层：`routes.json` 物理地址映射此前仅通过 `GET /api/routes` 暴露裸字符串对，外部无法探知该物理地址对应的对端是否在线、是否已被信任、友好别名为何；
  2. 静态路由注入与动态修改能力缺失：跨子网或路由变更时，无 API/命令行向运行中节点注入对端物理地址，必须杀进程手工改文件；
  3. 缺少智能对端解析联动：指定对端添加路由时无法使用友好别名；
  4. 双端 CLI 缺乏路由管理命令群（缺少 add/del）；
  5. Web UI 缺乏物理路由卡片与实时事件感知。
- **架构设计与落地 (ADR-039)**:
  1. `RouteEntryDto`, `RoutesReport`, `RouteMutateReport` (`crates/tetherly-node/src/runtime.rs`, `lib.rs`): 结构化报告模型，包含 `device_id`, `addr`, `alias`, `name`, `online`, `trusted`，绝不泄露敏感凭据。
  2. 存储层原语升级 (`crates/tetherly-node/src/store.rs`): `remember_route` 与 `forget_route` 返回布尔指示状态（`added` vs `updated`, `removed` vs `not_found`）。
  3. 智能解析与运行时原语 (`crates/tetherly-node/src/runtime.rs`): `Node::routes_report(&self)`, `Node::add_route(&self, peer_spec, addr)`, `Node::remove_route(&self, peer_spec)`，无缝支持友好别名并校验套接字与非零端口，向事件总线派发 `UiEvent::RouteChanged`。
  4. 回环 HTTP API (`crates/tetherly-node/src/uihttp.rs`): `GET /api/routes` 返回全景报告；`POST /api/routes` (及 `PUT`) 静态注入与热更新；`DELETE /api/routes` (及 `POST /api/routes/delete`, `POST /api/routes/remove`) 支持 Query 参数或 JSON 请求体删除。
  5. CLI 双端全同构 (`crates/tetherly-node/src/bin/tetherly.rs` 与 `bins/tetherly-cli/src/main.rs`): 新增 `Cmd::Route { action: RouteCmd }`，支持 `route list`, `route add --peer <peer> --addr <addr>`, `route del/remove --peer <peer>` 及 `--json` 格式化输出。
  6. Web UI 物理路由治理面板 (`ui/index.html`): 增设「物理寻址路由与静态注入」卡片，集成动态列表展示、弹窗交互注入、一键删除确认与 SSE `route_changed` 事件实时刷新。
  7. 端到端集成测试覆盖 (`tests/phase1.rs`): 增设 `route_table_governance_and_static_injection_via_loopback_api` 完备验证治理全链路。
- **发布与文档同步**:
  - SSOT `docs/DEVELOPMENT.md` 升至 v1.46，确立 ADR-039。
  - Obsidian 知识库 `tetherly 开发文档.md` 与路由表 `AI-DOC-ROUTER.md` 同步更新。
  - GitHub Actions 5 平台 CI 矩阵全绿验证（Run ID `39081141144`）：
    - `android` (arm64-v8a NDK): ✓ in 1m41s
    - `deny` (advisories, bans, licenses, sources): ✓ in 1m7s
    - `check (macos-latest)`: ✓ in 2m10s
    - `check (ubuntu-latest)`: ✓ in 3m32s
    - `check (windows-latest)`: ✓ in 5m1s
  - 核心安全红线严密守卫：无敏感凭据泄露，Loopback 仅绑定 127.0.0.1 且校验 Host，零 LGPL 链接。

## 2026-10-17 (23) - O(1) memory chunked streaming file transfer and incremental SHA-256 (v1.26)
- Task: 突破整文件内存驻留限制，实现 $O(1)$ 内存常数界的 64 KiB 分块流式收发管线与增量 SHA-256。
- **背景与缺陷根因**:
  在 v1.24/v1.25 中，虽然将 `FILE_MAX = 512 MiB` 确立为内存保护边界并打通了生产发送入口，但底层架构依旧采用「整文件缓冲」模式：
  1. 发送端：`offer_and_send_bytes` 会在 `pending_out` 中持久化整份 `Vec<u8>`，同时调用方还持有原始 buffer；
  2. 接收端：`accept_file` 在内存中用 `Vec<u8>` 接收全部解密块，直至全部收齐后方才执行一次性原子刷盘 `atomic_write`。
  当网络传输数百兆乃至数吉字节大文件时，双端内存暴涨、GC/分配器压力激增，直接限制了低配设备（如天翼云手机、老旧轻薄本）的稳定运行。必须重构传输管线为真正的 $O(1)$ 内存流式架构。
- **架构设计与落地 (ADR-019)**:
  1. `IncrementalSha256` (`crates/tetherly-core/src/frame.rs`): 纯净、无 OS 依赖的增量 SHA-256 状态机，包装 `sha2::Sha256`，支持对任意流式切片持续 update 与 finalize，单元测试证明与 `hex_sha256` 逐位一致。
  2. `send_stream_from` (`crates/tetherly-net/src/filechan.rs`): 针对任意 `R: AsyncRead + Unpin` 与 `W: AsyncWrite + Unpin`，严格以 `CHUNK = 64 KiB` 边界分块加密并传输，结合 `counter_nonce` 与 `aad_for(transfer_id, chan_offset)`，内存常数界限定为单块 64 KiB 缓冲区。
  3. `recv_chunks_to_sink` (`crates/tetherly-net/src/filechan.rs`): 边收边验 AEAD 标签，解密数据实时送入 `IncrementalSha256` 并异步刷盘至 `W: AsyncWrite + Unpin`（即 `.part` 文件），达成接收端 $O(1)$ 内存驻留。
  4. 节点层直接流水线集成 (`crates/tetherly-node/src/runtime.rs`):
     - `offer_and_send_file`: 接收磁盘文件路径，增量分块计算整体 SHA-256 生成 `transfer_id`，挂载 `OutboundSource::Path(PathBuf)`；
     - `on_peer_accepted_file`: 若对端接受，异步打开 `tokio::fs::File`，快速 seek 到 `resume_from` 块对齐偏移，直接建立 TCP 流并通过 `send_stream_from` 直冲网卡；
     - `accept_file`: 恢复断点或新建 `.part` 文件，通过 `recv_chunks_to_sink` 增量写入，传输完毕比对增量 SHA-256 摘要，校验通过后原子重命名至 `inbox/<filename>`，彻底解除 512 MiB 内存驻留上限。
  5. API 闭环 (`crates/tetherly-node/src/uihttp.rs`): `POST /api/file/send` 传入 `path` 时直接触发底层零拷贝磁盘流式通道。
- **新增回归与集成覆盖**:
  - `tetherly-core`: `incremental_sha256_matches_one_shot_hex_sha256`
  - `tetherly-net`: `streaming_roundtrip_with_sink_and_incremental_hasher`、`streaming_resume_from_chunk_boundary`
  - `tests/phase1.rs`: `multi_chunk_file_streamed_via_live_node_api_reaches_inbox`
- **全套验证**:
  - `cargo test --workspace` 全绿（涵盖 core、net、crypto、android、node、cli 及 loopback、phase1~phase5、backpressure、screen_wire、rekey 等全量测试）。
  - `cargo fmt --check` 全绿。
  - `cargo clippy --workspace --all-targets -- -D warnings` 全绿（0 警告 0 报错）。
  - `cargo deny check`（advisories/bans/licenses/sources 全部通过）。
  - **GitHub Actions CI-First 自动化验证通过**:
    - `ci.yml` (run `38973071906` on commit `b73e88d`): 5 个平台矩阵任务 100% 成功（`android` NDK arm64-v8a + Gradle 1m54s、`check macos-latest` 2m15s、`check ubuntu-latest` 1m48s、`check windows-latest` 3m43s、`cargo deny check` 54s）。
    - `release.yml` (run `38973572292` on commit `b73e88d`): 3 个发布编译矩阵任务 100% 成功（`build-linux` debian:bullseye glibc 2.31 容器构建通过 objdump 符号校验 2m54s、`build-desktop macos-arm64` 2m51s、`build-desktop windows-x86_64` 4m6s），生成三端 release 二进制工件。

## 2026-10-17 (22) - Production file-sending loopback path and GitHub CI-First doctrine (v1.25)
- Task: 全面接手开发、确立 GitHub CI-First 开发准则、闭环生产文件发送入口并同步 Obsidian 知识库。
- **GitHub CI-First 架构准则 (ADR-018)**: 遵循指示，将多平台编译（Windows x86_64、Linux glibc 2.31 容器、macOS arm64、Android NDK arm64-v8a + Gradle）、代码风格（`cargo fmt`）、代码质量（`clippy -D warnings`）、多平台集成测试（`cargo test --workspace --locked`）与安全/合规审计（`cargo deny check`）全面移交 GitHub Actions CI。本地开发机专注代码开发与单测回归，杜绝本地跨平台工具链漂移。
- **生产文件发送闭环**:
  1. 回环 HTTP API (`crates/tetherly-node/src/uihttp.rs`): 新增 `POST /api/file/send`，支持 `{ path: "...", peer?: "...", name?: "..." }` 或 `{ data_base64: "...", peer?: "...", name?: "..." }`。执行目标 peer 校验、文件可读性与 `FILE_MAX` (512 MiB) 强校验、文件名安全净化（`sanitize_file_name`）、计算 SHA-256，并调用 `Node::offer_and_send_bytes`。
  2. CLI 回环客户端 (`crates/tetherly-node/src/bin/tetherly.rs` 与 `bins/tetherly-cli/src/main.rs`): 新增 `tetherly send <path> [--name <name>] [--peer <tdev_...>] [--ui-port <port>]` 子命令，作为 127.0.0.1 活节点回环客户端（ADR-013 标准）。
  3. UI 交互 (`ui/index.html`): 增设文件发送表单（目标设备下拉、文件选择器、发送按钮与动态状态提示），在网页中通过 `FileReader` 读取为 base64 发起发送。
  4. 端到端集成回归 (`tests/phase1.rs`): 新增 `file_sent_via_live_node_api_reaches_the_paired_peer`，覆盖磁盘路径发送、Web base64 发送、收件箱 SHA-256 对账与异常校验（未知设备/不存在文件均返回 400）。
- **验证与发布**:
  - 本地与 CI 验证：全工作区 291 项测试全部通过（290 → 291）。
  - GitHub Actions CI (Run ID `38966810057` on commit `f1b59c3`) 5 项任务全部通过（`deny` 52s、`ubuntu-latest` 2m37s、`macos-latest` 3m0s、`android` 1m46s、`windows-latest` 4m31s）。
  - SSOT `docs/DEVELOPMENT.md` 升至 v1.25，同步写入 Obsidian 知识库及路由表。

## 2026-10-16 (21) - FILE_MAX becomes a real gate and interface classification explains itself (v1.24)
- Task: 你把值得做的做下吧 — implement the two items proposed after reviewing https://github.com/motto1/Lancat: (D) turn the dead `FILE_MAX` guard into a real gate with a regression, and (B-1) add a read-only, explainable interface-classification outlet. Lancat is **GPL-3.0**, so only publicly-known architectural ideas were borrowed — no code linked or copied, and none of its ports (`47820`, `5353`), service name (`_lancat._tcp.local.`) or message names appear in Tetherly.
- **Defect (D): the cap was a dead constant.** `tetherly_core::filexfer::FILE_MAX = 512 MiB` existed but nothing used it to refuse anything: `accept_file` had `if total > size.max(32 * 1024 * 1024)` — a local `max()` that always picked the larger operand, so a file larger than 32 MiB slipped through by construction — and the 45718 receive path was invoked with `max_size = u64::MAX`. The v1 path is **whole-file resident in memory** on both ends (the receiver buffers a `Vec<u8>` then `atomic_write`s; the sender's `pending_out.data = data.to_vec()` copies on top of the caller's buffer), so `FILE_MAX` is really a *memory* bound — which is why one number, enforced everywhere, is the honest design.
- **Fix (D): three gates + a non-fatal refusal.** (1) `FileHub::offer_resumable` now returns `CoreError::FileTooLarge(FILE_MAX)` when any declared `size > FILE_MAX` — a new variant that fits the existing `*TooLarge` family (Hello/Payload/Clipboard/Screen). (2) The channel: `accept_file` returns `NetError::TooLarge` for `total > FILE_MAX` and passes `FILE_MAX` (not `u64::MAX`) as `recv_chunks_after_header`'s `max_size`, so the declared header alone can refuse a transfer before any byte is written. (3) The sender: `offer_and_send_bytes` refuses at entry. Plus the behavioural fix that matters most: `SessionEvent::FileOffer` used to propagate the error with `?`, which **killed the whole session** on a bad name/hash/oversize; it now logs `warn!(%e, transfer = %tid, "refused file offer")` and keeps the session — refusing one transfer must not drop the peer.
- **Fix (B-1): interface classification is now one rule with a readable reason.** `crates/tetherly-core/src/bind.rs` gains `BindClass { Listen, LoopbackAlias, Unspecified, Broadcast, Multicast, Overlay, CgnatOverlay, Ipv6, NotLan }` with `classify`/`classify_v4`/`reason()`, and `should_listen*` is now defined as `classify(..).listens()` — so the predicate and its explanation cannot drift. Two classes are new and both matter: `LoopbackAlias` names a `127/8` alias other than `127.0.0.1` (e.g. systemd-resolved's `127.0.0.53`); `if_addrs` enumerates it, and `lan_ips()` previously skipped loopback interfaces implicitly — a second rule that could disagree with `should_listen`, so that skip was **removed** and folded into the one classifier. `CgnatOverlay` names RFC 6598 `100.64.0.0/10` — the Tailscale/NetBird fabric that is neither RFC1918 nor link-local in Rust's `Ipv4Addr`, and was therefore silently skipped by v1 unless the user passed `--overlay-cidr` (this is the Lancat lesson: classify by known range and say *why*). Node side: `lan::IfaceReport` + `interface_report()` (seeded with `127.0.0.1`, deduped by IP) and `log_skipped_interfaces()` (one info line per skipped address); `uihttp::Snapshot.lan` exposes the report so the UI can explain “nothing found” without the user reading logs.
- **Regressions (285 → 290 green)**: core `bind::classify_names_the_reason_and_stays_consistent` (every class, incl. `127.0.0.53 → LoopbackAlias`, `100.64.0.9`/`100.127.255.1 → CgnatOverlay`, `100.128.0.1`/`8.8.8.8 → NotLan`, `::`/`::1 → Ipv6`, plus the invariants `listens() == (self == Listen)` and `reason().is_none() == (self == Listen)`); core `filexfer::an_oversized_offer_is_refused_before_it_becomes_a_candidate` (`FILE_MAX + 1` → `FileTooLarge(FILE_MAX)` and the hub holds nothing; exactly `FILE_MAX` accepted); net `filechan::a_declared_size_over_the_cap_is_refused_on_the_header_alone` (a `TFL2` header declaring `total = 1025` with `max_size = 1024` → `NetError::TooLarge` and no bytes written); node `lan::interface_report_matches_the_actual_listen_set` (report[0] is `127.0.0.1`/listening, every row's `listen` equals `should_listen(ip)` and `reason.is_none()`, and the sorted report's listening IPs equal the sorted `lan_ips()`); end-to-end `tests/phase1::an_oversized_offer_is_refused_without_dropping_the_session` (two real nodes; a forged `FileOffer` of `FILE_MAX + 1` leaves `offered_files()` empty after 300 ms, then a normal 64 KiB file still transfers with a matching sha256 — proving the refusal did not kill the session).
- Verification: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets --locked -- -D warnings` clean; `cargo test --workspace --locked` **290 green**; `cargo deny check` advisories/bans/licenses/sources ok; `Cargo.lock` unchanged. SSOT v1.23 → **v1.24**.
- Honest boundary / adjacent findings reported, not fixed: (a) there is **no production file-send path** — `offer_and_send_bytes` is exercised only by `tests/phase1.rs`; no `/api/file/send` route, no CLI subcommand, so the file feature is receive-only in production (the receiver is fully wired, the sender is not); (b) the whole-file-in-memory design makes `FILE_MAX` a memory bound by nature, and true streaming (write-as-you-go + incremental sha256, chunked/mmap send) is the follow-up that would let it rise. Lancat's own choices that must **not** be copied stay recorded from the earlier assessment: LAN-trusted/no-pairing/**unencrypted** transport, one-hop auto relay, and a Cloudflare Tunnel/frp public path all contradict Tetherly's SPAKE2 + trust-after-Noise design and §2.3.

## 2026-10-16 (20) - Windows ANCS BLE GATT transport + inbound pump (v1.23)
- Task: 开工 / 继续 / 做完待办 — close the one Phase 4 link that was still missing a real machine path: the computer-side ANCS transport.
- **What was missing (established, not assumed)**: §9.4 already named the platform stacks, and `tetherly-core::ancs` (Notification Source decode, Control Point encoders, byte-wise `DataAssembler`, `AncsIngress` state machine, `AncsTransport` port + `MemoryAncsTransport`) was all green. But there was **no `src/ancs/` in `tetherly-node`**, `NodeConfig.ancs` is `None` on every production path, and core deliberately has no notion of "a value changed". So on a real machine ANCS could not start at all — the port existed, the plug did not.
- **Lockfile-neutrality proved before writing code**: read `windows-0.59.0/Cargo.toml` — the WinRT Bluetooth APIs are **feature flags** on the already-present `windows` crate (only `windows-core` + `windows-targets` are dependencies), so `git status --porcelain Cargo.lock` stayed empty after adding the features.
- **The design hinged on one discovery**: `windows` 0.59's `IAsyncOperation<T>` both implements `IntoFuture` and has a **blocking `get()`** driven by an OS `Waiter` (`src/extensions/Foundation/Async.rs`), so the three synchronous port methods need neither a hand-rolled executor nor a runtime — no new dependency, no busy-wait.
- **Thread model**: `crates/tetherly-node/src/ancs/win_gatt.rs` (self-contained `#[cfg(windows)]`, absolute imports only, per the `screen/macos.rs` precedent) runs a dedicated `tetherly-ancs` thread that owns every WinRT object and creates its own **MTA** apartment (`CoInitializeEx(None, COINIT_MULTITHREADED)`; `windows-core` has no `initialize_mta`). The `AncsTransport` front-end holds only a `Sender<Cmd>`, so the port is `Send + Sync` **by construction** and the node never touches a COM interface. Inbound bytes cross on a `tokio::sync::mpsc::unbounded_channel` of `AncsInbound` (`Ready`/`NotificationSource`/`DataSource`/`Disconnected`), assembled in `crates/tetherly-node/src/ancs.rs`; `open(address)` returns `Ok(None)` off Windows.
- **§9.4 constraints enforced by structure**: `FromBluetoothAddressAsync` → `GetGattServicesForUuidAsync(ANCS)` → `GetCharacteristicsAsync` → find the three characteristics and **validate their properties** (Notification Source / Data Source must be `Notify`, Control Point must be `Write`); CCCD writes `Notify`; `ValueChanged` callbacks **only copy bytes** (never write the Control Point from that thread); `write_control_point` goes through `DataWriter` → `WriteValueWithResultAsync` and checks `GattWriteResult::Status`, **blocking until it is true**, so "serial, at most one in flight" is a fact rather than an assumption — with a **10 s ack bound** on the front-end side (the v1.21 `WriteStalled` posture: a wedged stack reports `AncsSubscribe`/`AncsWrite`, never a success it cannot prove). `ConnectionStatusChanged` raises an atomic flag → session ends → `ancs_backoff_delay(attempt)` (1 s → 30 s); `Session::drop` explicitly `RemoveValueChanged`×2 + `RemoveConnectionStatusChanged`. The `ConnectionStatusChanged` handler args arrive as `&Option<BluetoothLEDevice>` because a WinRT **class** has `Default = Option<T>` (interfaces like `windows_core::IInspectable` do not), and both delegates are passed **by reference** (`TypedEventHandler::new` needs `Param<T, CloneType>`).
- **Node + CLI**: `Node::spawn_ancs_pump(inbound)` runs on its own thread with a current-thread tokio runtime (so the blocking GATT waits never land on the async workers) and feeds `ancs_connect` / `ancs_notification_source` / `ancs_data_source` / `ancs_disconnected`, driving `ancs_tick` on a 500 ms interval; `tetherly run --ancs <11:22:33:44:55:66>` opens the link (pair the iPhone in Windows Bluetooth settings first — the same two-step wizard core/UI already documents).
- **Regressions (280 → 285 green)**: 4 pure-logic `win_gatt` unit tests (Windows address notation `:`/`-`/bare-hex with the first byte as the high byte; 6 malformed inputs refused; the four ANCS UUIDs' LE decode ↔ canonical GUID **field-for-field** — a mapping error would silently query the wrong service, so the assertion has value —; malformed UUID reported) + 1 end-to-end `tests/phase4.rs::m4_5_the_transport_pump_subscribes_in_order_and_feeds_the_ingress` driving the pump with `MemoryAncsTransport` (`Ready` → subscribe order Data Source first; callbacks only enqueue while the Control Point is written on `tick`; a Data Source response becomes a candidate for uid 77; `Disconnected` → `AncsState::Backoff`).
- Verification: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets --locked -- -D warnings` clean; `cargo test --workspace --locked` **285 green**; `cargo deny check` advisories/bans/licenses/sources ok; `Cargo.lock` unchanged.
- SSOT v1.22 → **v1.23** (header + status row + new "1.23 相对 1.22" block + §9.4 landed paragraph + §12 Phase 4 landed-scope paragraph + §13 integration row + §22 item 7 + §8.x file-tree line). **Honest boundary recorded**: the WinRT code compiles on native Windows x64 (stronger than v1.19's cross-target type-check), but pairing and live notifications need a physical iPhone, so M4.1's p95 stays Manual-required; macOS (CoreBluetooth) and Linux (BlueZ) stacks are **not implemented** and `open()` honestly returns `Ok(None)` rather than pretending.

## 2026-10-16 (19) - Inner-frame payload budget corrected: fragmenting screen frames could not be encrypted (v1.22)
- Task: 继续 — keep developing; while adding the v1.21 write-stall regression, chase down the `Crypto(Noise("input error"))` it surfaced.
- **How it surfaced**: the write-stall regression filled frames with a payload of `INNER_PAYLOAD_MAX` (65536) and both tests failed with `Crypto(Noise("input error"))` *before* any stall. First assumed a test-sizing bug; measuring the real ceiling showed it was a **production** bug.
- **Measurement (not reasoning)**: added `crates/tetherly-crypto/tests/limits.rs`, which completes a real local IK handshake (`NoiseHandshake::initiator`/`responder` → `into_transport`) and probes `NoiseTransport::encrypt`. The transport accepts exactly **65519** plaintext bytes (`65535 − 16` AEAD tag) and refuses 65520. `InnerFrame::encode` prepends a **12-byte** header, so the largest encryptable inner payload is `65519 − 12 = 65507`.
- **Defect**: `frame.rs`'s `INNER_PAYLOAD_MAX` was a flat `64 * 1024 = 65536` — **29 bytes too large**, because it counted neither the 12-byte inner header nor the 16-byte AEAD tag. The screen fragmentation `SCREEN_FRAG_CHUNK_MAX = INNER_PAYLOAD_MAX − SCREEN_FRAG_HEADER` (24) then emitted fragment payloads of `24 + 65512 = 65536` → **65548-byte plaintext the transport cannot encrypt**. So **every screen frame large enough to fragment was unsendable on 45720**. zstd compresses real frames far below the fragment threshold, which is why the shipped end-to-end path never hit it — a "success the process cannot prove" sibling of the v1.18 `pin` class. `to_frame()`'s single-frame path shared the root cause; the **file channel is unaffected** (its own ChaCha20-Poly1305 `aead_seal` + length prefix on 45718, not the Noise transport).
- **Fix (derive, don't hardcode)**: `crates/tetherly-core/src/frame.rs` now has `INNER_HEADER = 12` and `NOISE_MESSAGE_MAX = 65_535`, with `INNER_PAYLOAD_MAX = NOISE_MESSAGE_MAX − 16 − INNER_HEADER` (= **65507**); `encode`/`decode` use `INNER_HEADER` instead of a literal 12. `SCREEN_FRAG_CHUNK_MAX` keeps its formula and now yields fragment payloads of exactly 65507 → 65519 plaintext = the ceiling, exactly encryptable. Wire-compatible: no version ever successfully sent a payload in (65507, 65536] (encryption fails before the write), so tightening the decode cap loses nothing. Corrected the misleading "64 KiB per fragment" comments in `screen.rs`.
- **Regressions (275 → 280 green)**: `tetherly-crypto/tests/limits.rs` (2 — measured ceiling 65519 pass / 65520 refuse; a max inner frame is encryptable while one byte more is refused); `tetherly-net/tests/screen_wire.rs` (1 — a 4 MiB 1024x1024 BGRA frame fragments via `to_frames()`, crosses a **real session** fragment-by-fragment with `send_bounded`, and reassembles byte-identical through `ScreenAssembler`); `tetherly-net/tests/backpressure.rs` (2 — the v1.21 write-stall session regression, filler resized).
- Verification: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace --locked` **280 green**; `cargo deny check` advisories/bans/licenses/sources ok. SSOT bumped to **v1.22** (new "1.22 相对 1.21" block, §9.7 fragment-budget note, §14 inner-payload line, §13 regression row).
- Honest boundary: the budget correction is proven by the new end-to-end regression; a real-machine full-screen (uncompressed / large-resolution) 45720 run stays inside Phase 5's existing Manual-required scope.

## 2026-10-16 (18) - Session liveness + backpressure, and moving the Linux release anchor to glibc 2.31 (v1.21)
- Task: 你全面负责本项目继续开发吧 — take the two defects the cross-VPS test left open (the notify "silent black hole" and the glibc floor mismatch) and close them.
- **What the test proved first**: v1.20 notify is genuinely end-to-end across VPSes — two pushes from `pxed` landed on `hk`; `/api/state` on `hk` showed `{"title":"来自桌面","has_otp":true,...}` and `{"title":"cross-vps-test","has_otp":false,...}` with `peers:[{device_id:"tdev_egro3fwkdjv5oeku",...}]`. So the feature worked once.
- **Defect 1 — the silent black hole (root cause)**: after the first push, later `notify` calls returned success while the peer logged *nothing*. Root cause was not "TCP is broken": the control session `await`ed a write **inside** a `select!` branch body, `write_tagged`'s `flush()` parks forever once the kernel send buffer is full, the outbound queue was `mpsc::unbounded_channel` (so the producer could never observe backpressure — `send_to` "succeeded" by handing the frame to an ever-growing queue nobody drained), and the node sent no pings, so a half-dead session stayed `ESTABLISHED` indefinitely. This is the same defect class as v1.18's `pin`: the process believed a delivery it could not prove.
- **Fix — three layers, each in its own crate**:
  1. `tetherly-core::liveness` (new, OS-free, `#![forbid(unsafe_code)]` preserved): `SESSION_HEARTBEAT=15s`, `SESSION_LIVENESS_DEADLINE=45s`, a `Liveness { heartbeat_ms, deadline_ms, last_inbound_ms }` with an injected clock (`Liveness::new(now_ms)` / `with(...)`, `on_inbound`, `is_expired` using `saturating_sub` so a backwards clock never reports dead — "silence, not time travel, is the failure signal"). 4 deterministic unit tests.
  2. `tetherly-net`: `SESSION_WRITE_TIMEOUT=15s`, `NetError::WriteStalled`, `ActiveSession::send_bounded()` wrapping `send_inner` in `tokio::time::timeout` — the `flush()` can no longer park forever.
  3. `tetherly-node`: every session queue is now **bounded** (`SESSION_QUEUE_DEPTH=256`, `SCREEN_QUEUE_DEPTH=32`) and producers use `try_send`, so a slow peer yields `CoreError` ("peer busy; send queue full") instead of a lie; the control session runs a 15s keepalive `ping` task (skips on Full, breaks on Closed); the main loop checks `live.is_expired(clock.unix_ms())` before each write and tears the session down with `warn!("session liveness lost")` → `PeerDown`; **all 8 write sites** call `send_bounded` (including the `pong` reply and every accept/leave/frame path); the screen path drops a whole frame (`tx.capacity() < frags.len()`) rather than half-enqueueing it.
- **Defect 2 — the glibc floor mismatch**: v1.20's `ubuntu-22.04` artifact required `GLIBC_2.34`, but §14 promises coverage down to glibc 2.31 hosts (Debian 11). A dynamic binary's required `GLIBC_x.y` is a **hard lower bound** — no flag lowers it; the only fix is to build against an older libc. `.github/workflows/release.yml` now runs `build-linux` inside a **`debian:bullseye` (glibc 2.31)** container (apt-get install runs *before* checkout), builds, then a guard prints `objdump -T | grep -Eo 'GLIBC_[0-9]+\.[0-9]+' | sort -u` and **fails the build** on any `GLIBC_2.32+`. `build-desktop` is a `windows-latest`/`macos-latest` matrix; `publish` `needs: [build-linux, build-desktop]` and only runs on a tag.
- Verification: `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace --locked` **275 green** (271 → 275, +4 liveness tests); `cargo deny check` advisories/bans/licenses/sources ok. Workspace bumped **0.1.3** (path deps + `Cargo.lock` only for the 7 tetherly packages; `cbc` left at its own version).
- SSOT v1.20 → **v1.21** (header + status row + new 1.21 block with 动机/根因/修复三层/效果/Linux 锚点/验证与诚实边界 + §14 Linux row now the bullseye container + `GLIBC_2.32+` guard). **Honest boundary recorded**: the container build path is CI-only (no Linux linker on this host), so the first release must be re-verified on a real glibc 2.31 host before the coverage claim is final.
- **Shipped**: pushed `main` (commit `f9b6d16`), `ci.yml` green on all five jobs (android, deny, check windows/macos/ubuntu). Tagged `v0.1.3`; the first release run then needed two CI fixes, both committed and re-tagged (final tag on `1883377`):
  1. `debian:bullseye`'s `/bin/sh` is **dash**, which rejects `set -o pipefail` — the install-deps step died with `Illegal option -o pipefail`. Fix: pin the job's `run` shell to `bash`.
  2. bullseye is **past LTS**, so `deb.debian.org` no longer serves its pool and every `apt-get update` 404'd. Fix: point `sources.list` at `archive.debian.org` (`debian` + `debian-security`) with `Acquire::Check-Valid-Until=false`.
  Final `release.yml` run green: `build-linux` (container), `build-desktop` (windows + macos), `publish`. The guard printed the real glibc floor: **`GLIBC_2.30`** (highest version referenced), i.e. below the 2.31 target. Release `v0.1.3` published with all six assets (`tetherly`/`tetherly-cli` × linux-x86_64, windows-x86_64, macos-arm64).
- **Local verification method note**: this host's default `x86_64-pc-windows-gnu` linker is the 32-bit MinGW.org 6.3.0 at `c:/mingw/bin` (its `ld` rejects rustc's `--high-entropy-va`). The full local run (clippy + 275 tests + deny) used the 64-bit Scoop MinGW (gcc 16.2.0) as linker and C compiler via **shell env vars only** (`CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER`, `CC_x86_64_pc_windows_gnu`); nothing machine-specific was committed (`.cargo/config.toml` untouched, CI uses an MSVC host).

## 2026-10-16 (17) - CLI control plane as live-node loopback clients + Linux glibc 2.35 release anchor (v1.20)
- Task: 深度改进下吧 / 继续 — act on the defects the cross-VPS test exposed.
- **Defects found by the cross-VPS test**: `tetherly dial` still did `Node::start(cfg)?` — with the daemon already running it re-bound the same LAN ports and died with `no LAN address bound`; and there was **no** CLI path to deliver a notification across paired nodes (`tetherly inject-notify` built a `NotifyPush` in its own process and only called `ingest_local_push`, so it never crossed the Noise session). Third defect: the release Linux binary was built on `ubuntu-latest` (24.04 / glibc 2.39) and **refused to start** on glibc 2.31 VPS hosts.
- **Fix 1 — `Cmd::Dial` → loopback `POST /api/dial`**: same shape as v1.18's `pin` fix. `tetherly dial --addr … [--pin …] [--ui-port 45716]` posts `{addr,pin}` to the running node's loopback API via `tetherly_node::loopback_post`; the pin arming, dial, `persist_trust`, and `remember_route` all happen inside the live node. Honest non-zero exit when nothing listens.
- **Fix 2 — `Cmd::InjectNotify` deleted, `Cmd::Notify` + `POST /api/notify` added**: only the process holding the Noise session can deliver on it. The new route resolves the target (`peers.sort_by_key(device_id)`; one live peer implied; multiple → 400 requiring an explicit `--peer`; `--peer` is `DeviceId::parse`d and checked against the live set, unknown/offline → 400 `peer is not connected`; zero → 400 `no connected peer`), builds a `.truncate()`d push (`app_name="tetherly"`, `actions=["copy"]`, uid/timestamp from the wall clock), and calls `Node::send_notify()`. Body delivered, never executed, never logged. `NotifyBody` defaults `app_id` to `com.example.sms` via `#[serde(default = …)]`.
- **Fix 3 — release Linux glibc floor**: `.github/workflows/release.yml` build row `ubuntu-latest` → `ubuntu-22.04` (glibc 2.35, covers Debian 12 / Ubuntu 22.04 / current RHEL-family), plus a guard step `objdump -T target/release/tetherly | grep -E 'GLIBC_2\.(3[6-9]|[4-9][0-9])'` that **fails the build** instead of shipping an unrunnable package. Windows/macOS rows unchanged; CI test matrix unchanged.
- **Local toolchain note (not committed)**: this host's default `x86_64-pc-windows-gnu` MinGW is MinGW.org 6.3.0, a **32-bit** toolchain whose `ld` rejects rustc 1.99's `--high-entropy-va`, and the rustup rust-mingw `dlltool` ships without an `as` (so GNU import-lib generation died with "CreateProcess"). To get real local test evidence I temporarily used rustup's bundled 64-bit self-contained MinGW (gcc 14.2) as the linker, an `as.exe` shim next to `dlltool` delegating to NDK clang 20's assembler, and NDK clang 20 with MinGW headers as the `zstd-sys` C compiler. `.cargo/config.toml` stays machine-agnostic; CI uses an MSVC host and is unaffected. No repository file encodes any of this.
- Verification: `cargo fmt --all -- --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace --locked` **271 green** (loopback 11, phase1 10, phase2 5, phase3 4, phase4 10, phase5 5, core 129+27+2+18, crypto 11, net 7+7+3, node 16, android 3, client_e2e 3); `cargo deny check` advisories/bans/licenses/sources ok. New regression `tests/phase1.rs::notify_posted_to_the_live_node_api_reaches_the_paired_peer` asserts the posted text reaches the peer's candidate list and an unconnected `--peer` is refused (so a 200 cannot be a blanket accept).
- SSOT v1.19 → **v1.20** (header + date + status row + new 1.20 block + ADR-013 extension covering dial/notify + §13 integration row + §14 port/glibc notes). Honest boundary: real LAN-direct dual-desktop and 8h soak remain Manual-required.

## 2026-10-15 (16) - Land the macOS CoreGraphics + Linux X11 screen backends (v1.19)
- Task: 继续做完所有 todo. The user's standing "finish all the todos" direction; the remaining screen item was the long-declared §9.7 "下一迭代" macOS `core-graphics` / Linux X11 SHM capture.
- **`crates/tetherly-node/src/screen/macos.rs` (new, ~240 lines)**: `MacSource` — `CGMainDisplayID` → `CGDisplayCreateImage` → draw into an explicit BGRA8 `CGBitmapContext` (`PremultipliedFirst | Order32Little`) → de-pad the framework's actual stride → `ScreenFrame::raw`. Rewrote the pre-existing draft because it requested `w*4` stride and hard-refused when CoreGraphics padded, which would make any width whose row is not 16-byte aligned (e.g. 1365 px = 5460 B) fail outright; the rewrite requests a 16-byte-aligned stride and unpads. Screen-Recording denied → honest `ScreenRefused` (never a blank frame). `present` needs an AppKit `NSWindow` and returns an honest `ScreenRefused` naming it (§9.7 defers macOS present). `CGDisplayCreateImage`'s soft-deprecation warning is allowed at the one import line, not module-wide.
- **`crates/tetherly-node/src/screen/x11.rs` (new, ~420 lines)**: `X11Source` — MIT-SHM fast path (`shm_create_segment` + `shm_get_image`; the server writes into our `mmap`, no socket copy) with a plain `GetImage` fallback for SHM-less servers; MIT-SHM ≥ 1.2 gate (extension presence ≠ fd passing allowed); reads exactly `reply.size` bytes so the previous frame's tail can't leak in; refuses MSB-first byte order and root depth outside {24,32}; depth-24 high byte filled 0xFF. **Present implemented too**: override-redirect window + `PutImage`, with the drawable depth read from the root (`screen.root_depth`) instead of a hardcoded 24 — otherwise a 32-bit root answers `BadMatch`.
- **Wiring**: `screen.rs` picks `inner` per target by `#[cfg]` (`DxgiSource`/`MacSource`/`X11Source`), keeping `SystemSource` monomorphic; `SystemSink::present` grew the macOS/X11/other arms; `lib.rs`'s `DxgiSource` re-export became cfg-gated (it no longer exists on non-Windows). `Cargo.toml` gained target-deps: macOS `objc2-core-foundation` + `objc2-core-graphics` (`default-features = false`, to avoid the objc2-metal/dispatch2 surface) and Linux `x11rb` (`shm`) + `libc` (x11rb's own `libc` sits behind its disabled `allow-unsafe-code` feature). **No new package entered the lockfile** — all already arrive via `arboard 3.6.1`.
- **Verification, and its honest limit**: both backends were type-checked for their real targets (`x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`) with throwaway probe crates that `#[path]`-include the module and mirror the real wiring (private module, `present` called by an adapter), so dead-code and deprecated lints fire exactly as in CI; a stub `zstd` let the whole `screen.rs` be checked without a target C compiler. Three real defects surfaced and were fixed: two `ConnectionError` vs `ReplyError` mismatches from chaining `.and_then(|c| c.reply())`, an `Ok(ScreenFrame)` vs `Ok(Option<ScreenFrame>)` mismatch, and a `chunks_exact_mut`/`as_chunks` lint (the latter needs Rust 1.88; the workspace MSRV is 1.82, so the probe had to declare `rust-version = "1.82"` to reproduce CI's lint set). The probes were deleted afterwards. There is no Linux/macOS linker on this host (WSL has only the stopped docker-desktop VM, Docker is not running), so the 7+7 backend unit tests could not be executed here — they are pure logic (stride alignment, de-padding, truncation refusal, alpha fill, non-BGRA refusal) and touch no display, so CI runs them natively on `ubuntu-latest`/`macos-latest`. Every assertion was verified by hand.
- Verification: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace --locked` **green**; `cargo deny check` advisories/bans/licenses/sources all ok.
- SSOT v1.18 → **v1.19** (header + status row + new 1.19 block + §2.4 matrix row + §9.7 capture/present/landing + §12 M5.1 row + §12 v1.19 exemption note + §13 integration row + §23 item 5 + two stale "next iteration" clauses). Honest boundary: real capture (needs a display/permission), macOS present, the Wayland portal, and the open encoder (OpenH264/rav1e) remain Manual-required; this iteration claims **backend implemented and per-target compile-verified**, not "macOS/Linux remote desktop works".

## 2026-10-15 (15) - Cloud-phone real-device testing: APK E2E, M1.1 p95, and the `tetherly pin` defect (v1.18)
- Task: 继续 — with the user's grant (「我给你权限,你可以用三星的云手机进行真机开发测试」), drive the shipped APK end-to-end on the Samsung cloud phone (SM-F900F Galaxy Fold, arm64-v8a, Android 13, `10.144.144.52:5555`, over `adb reverse`) and close the remaining Manual-required Android items.
- **APK E2E closed**: built the `.so` (`cargo ndk` arm64-v8a) + `gradle assembleDebug`, installed `app.tetherly`, granted notification-listener access; the front-end `NotificationListenerService` → JNI → `libtetherly_android.so` live Noise session → desktop candidate `{app_id:"com.android.shell", title:"Your verification code", has_otp:true, source:tdev_...}`. Body/OTP never entered the snapshot or the logs.
- **M1.1 p95 measured on device**: `rtt_min≈17.4ms / p50≈17.9ms / mean≈37.5ms / p95≈56.9ms / p99≈361.8ms / max≈361.8ms` — inside the <1.0s budget. Honest boundary recorded: cloud phone over the `adb reverse` tunnel (same model on LAN ≈15–16ms), p99 tail is tunnel jitter; 8h soak still Manual-required.
- **The headline defect — `tetherly pin` (root cause of the "outer frame too large" false trail)**: `Cmd::Pin` did `Node::start(cfg)?; println!(node.generate_pin()?)` — it armed a PIN in a **throwaway process** whose listener nobody can reach. `pin_created_ms` is pure in-process state (never persisted), so a node with no PIN reports `pin_created_ms = 0` and every dial is judged `PinExpired`; on the phone this read as "the PIN shows but pairing always says expired", costing hours. The live route was always the daemon's loopback `POST 127.0.0.1:45716/api/pin` (the UI button's route). Fix: `Cmd::Pin { --ui-port }` now calls `tetherly_node::loopback_post(port, "/api/pin", b"{}")` to arm the **live** node, and fails honestly off-port. Support: `uihttp::bind_and_spawn` (sync bind, real port, taken port = hard error), `Node::spawn_ui()`/`ui_port()` (`Shared.bound_ui`), `run()` binds UI before returning. Regression: `phase1::pin_shown_by_the_live_node_api_is_the_one_that_pairs` (also asserts a hardcoded-PIN node cannot be dialed, so it cannot pass by accident).
- Also fixed the stale README "Phase 0(当前)" section — the real current CLI is `tetherly run` / `tetherly pin` (the `tetherly-cli` Phase 0 fixture is retained but off the product path).
- **Terrain work**: a two-process pairing attempt initially failed with `os error 10053` + host log `pairing failure recorded fails=1..5`; diagnosed as the cloud phone's app still dialing `adb reverse tcp:45717` with a stale PIN and burning the 127.0.0.1 lockout (5 fails / 15 min, keyed by both peer and IP). Cleared with `adb reverse --remove-all` + `am force-stop app.tetherly` + killing test daemons, then re-ran on non-default ports and pairing succeeded (`{"ok":true}`, `path lan`).
- Verification: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo test --workspace --locked` **270 green**; `cargo deny check` advisories/bans/licenses/sources all ok. Cleaned up: `adb reverse --remove-all`, `am force-stop app.tetherly`, `taskkill /IM tetherly.exe /F`, scratch dirs removed.
- SSOT v1.17 → **v1.18** (header + status + new 1.18 block + §9.6 + §12 M1.1(v1.18) + §13 + §23 + new ADR-013).

## 2026-10-15 (14) - Land SSOT §13's stateful state-machine property layer (v1.17)
- Task: 继续. v1.16 landed proptest/fuzz for **stateless** decoders and messages; §13's genuinely dangerous inbound surface is the **stateful machines** — `ReplayGuard`'s window, the rate limiters, `NotifyHub` dedup/expiry, `FileHub`'s accept gate, `ResumeTable` offsets, screen send/receive/reassembly, `AncsIngress`'s subscribe/backoff/hold. Their risk is not "can one input decode" but "can an arbitrary **sequence** push the state into a contract-violating state". Chose "extend the property layer to sequence-driven state machines" as the milestone.
- **`crates/tetherly-core/tests/state_machines.rs` (18 tests = 11 proptest + 7 deterministic witnesses)**: a `program(max_ops)` strategy yields an arbitrary byte op-program (`Vec<u8>` as an instruction stream — one byte picks the action, further bytes supply parameters); a `Cur` cursor **saturates** on out-of-range reads so the program itself can never panic, attributing every panic to the machine under test. The contract is asserted **after every step**.
- Machines covered: `ReplayGuard` (accepted id never re-accepted; ancient id always refused; no overflow panic — pins v1.15's `saturating_add`), `TokenBucket` (frozen clock releases exactly `floor(burst)`, refuses the next), `NotifyLimiter` (releases never exceed the leaky bound `10 + 2t/1000 + 1`), `ClipLimiter` (backwards clock grants no extra budget), `FileHub` (every name sanitized to a safe basename; never self-accepted before a click; incoming table bounded at `INCOMING_MAX = 256` under a flood), `ResumeTable` (`claim` offset `0 < off < total`, `FILE_CHUNK`-aligned, source-bound), `NotifyHub` (dedup / reply-correlation / expiry hold under any push-reply-tick sequence), `ScreenSender` (`is_streaming()` implies local consent — never streams without it), `ScreenReceiver` (cursor monotone; `stats().received` equals actual receipts), `ScreenAssembler` (any-order reassembly, incomplete stream refused, never panics — pins v1.15's `checked_mul`), `AncsIngress` (no panic on any event sequence; callback inert unless `Ready`, so a `PRE_EXISTING` uid never leaks into `drain_events()` — M4.3). Deterministic witnesses (incl. `screen_sender_gate_opens_once_the_local_machine_consents`) prove the gated assertions are not vacuously true.
- **Result: no new bug found** — v1.15's two fixes hold at the state level and the new properties did not break through. `cargo test --workspace --locked` 251 → **269 green**.
- Verification: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; Linux cross-check clean (with `CC_x86_64_unknown_linux_gnu=gcc AR_...=ar`); `cargo deny check` all ok.
- SSOT v1.16 → **v1.17** (header + status + new 1.17 block + §13 proptest/fuzz rows extended). Honest boundary unchanged: libfuzzer/`cargo-fuzz` (nightly), real capture/injection, and dual-desktop physical round-trips remain Manual-required; this layer does not impersonate coverage-guided fuzz.

## 2026-10-11 (13) - Land SSOT §13's proptest + fuzz layers (v1.16)
- Task: 继续开发. With every feature milestone either landed or hardware-blocked, I audited SSOT §13 and found a concrete, self-verifiable gap: §13's table declares a `proptest` layer ("控制 JSON 往返") and a `fuzz` layer ("内层帧、ANCS tuple") but **only 单测/向量/集成 actually existed**. v1.15 had just hand-fixed two remote-triggerable decoder bugs, which is exactly the class these layers are meant to catch. Chose "land the declared proptest + fuzz layers" as the milestone.
- **`proptest` core (`crates/tetherly-core/tests/decoders.rs`, 27 props)**: strategies `text`/`bytes`/`push_strategy`/`file_meta`/`input_event`/`raw_screen_frame`/`control_msg`/`multi_fragment_frame`. Never-panic on arbitrary input for every inbound decoder (`InnerFrame::decode`, `ScreenFrame::decode_payload`, `ControlMsg::decode_payload`, `ScreenAssembler::push`, `InputEvent::from_frame`, `NotificationEvent::parse`, `Hello::from_bytes`, `DeviceId::parse`, `parse_pin`, `sanitize_file_name`, JSON `from_payload`s, `DataAssembler` with an 8 KiB bound assert). Round-trips for every encodable message (notify push/dismiss/reply, clip set, file offer/decision/done, caps update, input event, screen frame, control msg, hello). Fragmentation: any-order reassembly, incomplete-stream refusal, payload cap.
- **`proptest` net (`crates/tetherly-net/tests/decoders.rs`, 7 props)**: `read_len_prefixed`/`read_tagged`/`read_header` never panic; oversized length prefix refused (`MAX_OUTER`+1..=u32::MAX); valid headers round-trip; `TFL1` reads as `start=0`; wrong magic refused. `&[u8]` is used directly as `AsyncRead` (tokio impl); `prop_assert!` cannot live inside an `async` block, so those use panic-based `assert!`.
- **`fuzz` core (`crates/tetherly-core/tests/fuzz_decoders.rs`, 2 tests)**: only stable toolchain (no nightly → no libfuzzer/`cargo-fuzz`), so a **deterministic xorshift mutator** drives a valid corpus (inner frames, all 7 control JSON types, all 7 input kinds, whole+fragmented screen frames, control msgs, ANCS source events + data-source tuples, canonical Hello) through 6 mutation kinds (bit flip, extreme byte, truncate, extend, multi-byte overwrite, splice) into **every decoder**, all under `catch_unwind`. Fixed seed `0x9E3779B97F4A7C15`, 512 mutations/seed, every 4th stacked. ~28k mutations in ~4.5s.
- Deps: `proptest = "1"` (MIT) added to `[workspace.dependencies]`; `[dev-dependencies] proptest.workspace = true` in core + net. Passes `cargo deny check`.
- **Result: no decoder bug found** — the v1.15 fixes hold and the new properties did not break through. `cargo test --workspace --locked` 205 → **251 green**.
- Verification: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo deny check` all ok. Cross-check `cargo clippy --target x86_64-unknown-linux-gnu ...` clean **once `CC_x86_64_unknown_linux_gnu=gcc AR_...=ar` is set** — Phase 5's `zstd-sys` now needs a target C compiler the host lacks, so the pre-Phase-5 cross-check trick alone no longer works (check-only; nothing links).
- SSOT v1.15 → **v1.16** (header + status + new 1.16 block + §13 proptest/fuzz rows rewritten). Honest boundary: libfuzzer/`cargo-fuzz` (nightly) remains Manual-required; the deterministic mutation driver is the stable-toolchain substitute, stated in the §13 row.

## 2026-10-11 (12) - Peer-input hardening: two remote-triggerable robustness bugs + regression tests (v1.15)
- Task: 继续 — with every feature milestone either landed or hardware-blocked, the remaining productive work is correctness hardening of the inbound (peer-controlled) surfaces. Scanned core/crypto/net for panics and unguarded arithmetic on wire values; found and fixed two.
- **`tetherly-core/src/replay.rs` (`ReplayGuard::check_and_record`)**: `msg_id + 1024 < self.next` adds 1024 to a **peer-supplied `u64`**. `msg_id = u64::MAX` (frame.rs decodes msg_id straight from the wire) panics in debug (`attempt to add with overflow`) and, in release (`overflow-checks` off), wraps so an ancient id is accepted as fresh — weakening the replay window. Fix: `msg_id.saturating_add(1024) < self.next`. Semantics unchanged (the 1024-entry `recent` deque still backstops).
- **`tetherly-core/src/screen.rs` (`ScreenAssembler::push`)**: `off = idx * SCREEN_FRAG_CHUNK_MAX` then `self.total - off` with no `off < total` check. A peer can send a valid small `total` (16) with `idx = 1`, so `off = 65536 > total` → `total - off` underflows (debug panic; release yields an enormous `expected` that fails the length check but only after the arithmetic wrapped). Fix: `checked_mul` for `off` + explicit `off >= self.total → Err(ScreenFragment)`; no allocation, no panic.
- Tests: +3 (`rejects_id_far_behind_the_high_water_mark`, `extreme_peer_ids_do_not_overflow`, `assembler_rejects_an_index_past_the_stream_end`). `cargo test --workspace --locked` 202 → **205 green**.
- Verification: `cargo fmt --all --check` clean; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo deny check` advisories/bans/licenses/sources all ok. No wire/port/state-machine change.
- SSOT v1.14 → **v1.15** (header + status + 1.15 block).

## 2026-10-11 (11) - macOS Keychain + Linux libsecret credential backends (v1.14), compile-verified
- Task: 继续 — port the credential vault beyond Windows, but only if I could actually verify it. Probed first: `rustup target add x86_64-apple-darwin x86_64-unknown-linux-gnu` worked and `security-framework` (macOS) + `secret-service` v4 `rt-tokio-crypto-rust` (Linux) both `cargo check` clean for their targets. The node crate can't be checked off-host (pre-existing `zstd-sys` needs a cross C compiler), so I split the backends into self-contained files and verified them via a path-include probe.
- refactor: `cred.rs` → dispatcher with `#[path]` backends `cred/{windows,macos,linux,other}.rs`. Public API unchanged. Windows backend = moved v1.13 code verbatim.
- macos.rs: `security-framework` generic password, service `tetherly.identity`, account `<device_id>`; `errSecItemNotFound` → "absent" (fallback); any keychain error → fallback, never fatal.
- linux.rs: `secret-service` blocking, default collection, attribute `device_id=<id>`, label `tetherly.identity`, `replace=true` upsert; `available()` false when no `org.freedesktop.secrets`; `default-features=false` + `rt-tokio-crypto-rust`.
- Cargo.toml: macos → `security-framework`/`security-framework-sys`; unix-not-macos → `secret-service`. All MIT OR Apache-2.0 (deny clean).
- Verification: probe `cargo check --target x86_64-apple-darwin` (macos backend) and `--target x86_64-unknown-linux-gnu` (linux backend) — **the Linux probe caught a real lifetime error** (`create_item`'s `Item<'_>` borrowing the temporary `Result` past `collection`), fixed by binding the result. Host: fmt/clippy clean, `cargo test --workspace --locked` 202 passed, `cargo deny check` ok. Probes deleted.
- SSOT v1.13 → **v1.14** (header + status + 1.14 block + §6.1 rewritten).
- Honest boundary: **runtime** keychain/Secret Service read-write is Manual-required (verified only by compilation here + the platform CI jobs that build macos/ubuntu). `TETHERLY_VAULT_TEST=1 cargo test` runs a real vault round-trip on a real machine.
## 2026-10-11 (10) - Hardening the v1.12/v1.13 code: direct tests for the restart scan + filechan edges
- Task: 继续 after the credential work. The two features just landed had thin direct coverage on their trickiest branches, so I added targeted tests (no behavior change).
- net (`crates/tetherly-net/src/filechan.rs`): `truncated_stream_is_an_error_not_a_silent_short_file` (a body that ends before `total` must be `Err(NetError::Io)`, never a short-but-Ok buffer that could later be sha256'd as if complete); `resume_start_must_match_the_preloaded_length` (a TFL2 stream claiming start=0 but a non-empty receiver buffer is `NetError::Handshake`, not an append at the wrong channel offset); `tfl1_legacy_header_is_read_as_start_zero` (backward-compat: the pre-resume `TFL1` header still decodes). net filechan now 7 tests.
- node (`crates/tetherly-node/src/runtime.rs`): new `#[cfg(test)] mod resume_scan_tests` — `a_healthy_sidecar_is_loaded_and_a_torn_one_is_dropped` drives `load_resume_table` over a real temp `inbox/` with a healthy sidecar+part, a torn sidecar (part missing → dropped and its file deleted), a bad device id, a zero offset, and an out-of-range offset, plus a non-sidecar file that must be ignored; `missing_inbox_dir_yields_an_empty_table`. node now 16 tests.
- Verification: targeted `cargo test -p tetherly-net filechan` (7 pass) and `-p tetherly-node resume_scan` (2 pass), then full workspace regression.
## 2026-10-11 (9) - Windows Credential Manager identity storage (v1.13)
- Task: continue 都做完吧 — the last sizeable *locally-verifiable-on-Windows* gap was OS credential-vault identity storage (SSOT §6.1 said "系统凭据库优先" but only the file existed). Implemented it behind a portable fallback and verified the real vault round-trip on this Windows machine.
- new `crates/tetherly-node/src/cred.rs`: `load_or_create_identity(dir)` prefers the vault; `vault_store`/`vault_delete`; `write_ref` (`identity.ref`, public device-id pointer); `mod ffi` with safe wrappers `write`/`read`/`delete` over `CredWriteW`/`CredReadW`/`CredDeleteW`. `vault_available()` = Windows only; `vault_enabled_for(dir)` = installed data dir only (env `TETHERLY_VAULT=1/0` override). `lib.rs` adds `pub mod cred;`; `runtime.rs::start_with_clock` now calls `crate::cred::load_or_create_identity`; `Cargo.toml` windows-sys gains `Win32_Security_Credentials`.
- Migration: first run with an existing `identity.bin` reads it, writes the vault entry, writes `identity.ref`, and removes the plaintext file. After that the secret exists only in the vault; the dir holds `identity.ref` (device id, not secret).
- Hygiene: temp-dir nodes (all integration tests) never touch the vault — they use their own `identity.bin`, so `cargo test` leaves the operator's Credential Manager clean. `CRED_PERSIST_LOCAL_MACHINE` (per-user, survives reboot, machine-local).
- Tests: real Windows vault round-trip (`windows_vault_roundtrips_via_the_ref_pointer`) — `vault_store`→`write_ref`→`load_from_vault` device id match→`identity.bin` absent→`vault_delete`→not found; `installed_data_dir_opts_into_the_vault` (gate rule); `file_fallback_roundtrips_on_every_platform` (non-Windows / fallback). 3 pass on Windows.
- SSOT: v1.12 → **v1.13** (header + status + 1.13 changelog block + §6.1 rewritten + §12 Phase 6 note).
- Honest boundary: macOS Keychain / Linux libsecret still NOT wired (`vault_available()==false`); file fallback is the behavior there.
## 2026-10-11 (8) - File cross-process resume (v1.12), §7.4 "v1 无跨进程续传" gap closed
- Task: user said 都做完吧 (finish everything). Next largest *locally verifiable* gap: SSOT §7.4 still said "v1 无跨进程续传" — large files that die mid-transfer restart from zero. Implemented chunk-aligned, content-addressed cross-process resume across core/net/node with CI tests.
- core (`filexfer.rs`): `ResumeState{transfer_id,source,total,offset}` + `ResumeTable` (`new`/`put`/`remove`/`get`/`all`); `claim(source,transfer_id,total)` returns the offset only if source+size match, offset>0, offset<total, and offset % FILE_CHUNK == 0; `put` aligns offset down to the chunk boundary. `IncomingTransfer.resume_from: Option<u64>`; `FileHub::offer` → `offer_resumable(...,resume_hint)` (hint validated >0,<total,aligned); `accept` → `accept_at(id,now,resume_from)` (invalid → `CoreError::ResumeOffset`); `reject` returns `resume_from:None`. `frame.rs`: `FileDecision.resume_from` (serde `skip_serializing_if`), `FileDecision::{accept_resuming,accept}`. `FILE_CHUNK=64*1024`. Tests `resume_claim_requires_source_and_size_and_alignment`, `accept_at_validates_resume_offset`.
- net (`filechan.rs`): `FILE_MAGIC_V2=*b"TFL2"`; `aad_for(id,chan_offset)=id||offset.to_le_bytes()`; `send_chunks_from(w,token,id,total,start,data)` writes `TFL2|id|total|start` and seals from `start` with `counter=chan_offset/CHUNK+1`; `read_header` accepts TFL2 or TFL1(start=0); `recv_chunks_after_header(...,start,...)` checks alignment + `out.len()==start`, opens each chunk with `aad_for(id,chan_offset)`, appends, invokes `on_chunk(out,len)`; `resumable_offset(path,chunk)` truncates a partial tail. `send_bytes`/`recv_bytes` are thin wrappers (start=0). Tests `file_bytes_roundtrip`, `resume_sends_only_the_tail_from_a_chunk_boundary`, `a_chunk_cannot_be_relocated_across_resume_positions`, `resumable_offset_truncates_partial_tail`.
- node (`runtime.rs`): `Hubs.resume: ResumeTable`; `ResumeSidecar{device_id,total,offset}`; `load_resume_table(data_dir)` scans `inbox/.*.resume.json`, drops torn sidecars (missing/short `.part`), starts clean. `offer_and_send_bytes` → content-addressed `transfer_id = "tr_"+hex_sha256(data)[..16]`. FileOffer arm computes `hint = hubs.resume.claim(peer,id,total)` → `offer_resumable`. `on_peer_accepted_file` reads `decision.resume_from` → `send_file_data(...,start,...)` → `send_chunks_from`. `accept_file` rewritten: `read_file_header`, verify `total==size && start==resume_hint`, preload existing `.part` (truncate to `start`, refuse `aligned<start`), `set_len(start)` then append each chunk via `on_chunk` + `sync_all`, `note_resume` on error, sha256 verify, atomic rename into `inbox/`, remove `.part` + sidecar, send FileDone.
- Verification: `cargo test --test phase1 m1_8` → `m1_8_file_resumes_from_persisted_offset ... ok` (1 passed). core filexfer 5 tests pass; net filechan 4 tests pass. SSOT v1.11 → **v1.12** (header + 1.12 changelog block, §7.4 rewrite, Phase 1 M1.8 row, exit-range, §13/§19 refs).
- Manual-required: none for this feature — it is fully covered by CI (unit + restart integration). Real interrupted large-file transfer over a flaky LAN is the same code path.
## 2026-10-11 (7) - notify.reply quick reply (v1.11), §7.2 stub closed
- Task: user said 都做完吧 (finish everything). Picked the largest *locally verifiable* remaining gap: `notify.reply` — SSOT §7.2 still listed `0x0103` as a stub, and Phase 6 backlog carried it. Implemented desktop→phone quick reply end to end across core/net/node/UI/Android with CI tests.
- core: `frame.rs` — `NotifyReply{uid,app_id,text}`, `to_frame()`/`from_payload()`, `NOTIFY_TEXT_MAX = 4*1024` (multibyte-safe truncate; `NotifyPush` now shares the same const), test `notify_reply_roundtrip_and_cap` (roundtrip + CJK boundary + MUST_UNDERSTAND never fires on 0x0103). `notify.rs` — `NotifyHub.replies: HashMap<reply_id, QueuedReply>`; `reply_id = sha256(source\0app\0uid)`; `ingest_reply` (trim, reject blank, last-write-wins), `take_reply_id`/`take_reply` (single-use), `reply_views()`; `ReplyView{id,source,app_id,uid}` (no text); `QueuedReply` private with a redacting hand-written `Debug`. `error.rs` — `EmptyReply`.
- net: `dispatch.rs` — `SessionEvent::Reply(NotifyReply)` replaces the `ReplyStub`; `TYPE_NOTIFY_REPLY => NotifyReply::from_payload(...)`.
- node: `events.rs` — `UiEvent::ReplyReceived{reply: ReplyViewDto}` + `ReplyViewDto` (no text). `runtime.rs` — `handle_event` `Reply` arm (ingest + emit, logs only `text_len`); `send_reply`, `reply_to_candidate(id,text)` (blank → `Err(EmptyReply)`, else truncate + send to the candidate's source), `take_reply(id) -> Option<(ReplyView, String)>`, `reply_views()`. `uihttp.rs` — `POST /api/reply {id,text}` and `POST /api/reply/take {id}`; `Snapshot.replies`.
- ui: `index.html` — per-candidate reply input (`maxLength=4096`, Enter sends) + "收到的回复" section.
- Android: `client.rs` — `SessionHandle.inbox` + `take_replies()`, `serve` loop handles `TYPE_NOTIFY_REPLY` (drop same uid+app, last-write-wins); `jni.rs` — `Java_app_tetherly_NativeBridge_nativeTakeReplies`; `NativeBridge.kt` `takeReplies()`/`PhoneReply`; `NotifyListener.kt` `drainReplies()` + `fireRemoteReply()` (matches live notification by `key==uid && packageName==appId`, finds a free-form `RemoteInput`, `RemoteInput.addResultsToIntents` + `actionIntent.send`; text never logged); `NodeSession.kt` `takeReplies()`.
- Tests: `tests/phase2.rs::m2_6_notify_reply_reaches_the_phone_and_is_consumed_once` (pair, desktop reply, phone polls views, take once, second take `None`, blank `is_err`); `crates/tetherly-android/tests/client_e2e.rs::android_client_receives_a_quick_reply_once` (real `accept_session` server sends `NotifyReply::to_frame()`).
- Verification: `cargo fmt --all --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test --workspace --locked` **188 passed** (up from 182). SSOT v1.10 → **v1.11** (header + 1.11 changelog block, §7.2 row, §9.6 快速回复 paragraph, Phase 6 backlog note, status row).
- Manual-required: real on-device `RemoteInput` firing needs a foreground notification with a reply action + notification-listener grant (protocol + inbox covered by CI).
- Next: `cargo deny check`, then the remaining locally-completable items (file persistent resume, Windows Credential Manager), then the honest hard-blocked-remainder report. Commit/push only on explicit user instruction.

## 2026-10-11 (6) - Phase 1 Android native library (v1.10), §9.6 honesty gap closed
- Task: user said 继续开发. Chose the largest honest SSOT gap that is verifiable end-to-end: the Android `.so` (`libtetherly_android.so`) — real Ed25519 identity + Noise session — which §9.6 marked as a Phase 1 honesty gap and the Kotlin app already referenced but had no library for. This unblocks physical Android M1.1 (was Manual-required *because* the `.so` was missing) and is verifiable on the two adb-connected cloud phones.
- New crate `crates/tetherly-android` (lib `tetherly_android`, crate-type `["cdylib","rlib"]`): (1) `store.rs` — real Ed25519/X25519 `Identity::from_secrets`, `identity.bin` 64B temp+rename 0600, `trusted.json` SAME schema as desktop node (files interchangeable); (2) `client.rs` — `AndroidClient::load_or_create` + `start(addr,pin,once)` spawns a "tetherly-android" thread with a current_thread tokio runtime, dials 45717 via the desktop's own `dial_session` (SPAKE2 8-digit PIN first run, Noise IK resume after), `SessionHandle` notify/state_code/connected_peer/last_error/stop, PHONE_CAPS=["notify","clip","file"], backoff 250ms→5s, TYPE_PING→pong; (3) `jni.rs` — 7 `Java_app_tetherly_NativeBridge_*` exports with `<'local>` JNIEnv lifetimes, `#![allow(unsafe_code)]` (the one justified exemption; lib root is `#![deny(unsafe_code)]`), `jni` dep gated to `cfg(target_os="android")`; (4) `src/bin/android_probe.rs` — on-device pair+notify proof.
- `tetherly-net`: added `SessionConfig.caps: Option<Vec<String>>` (explicit Hello cap override; default None = existing inference) so the phone advertises an honest cap set. Updated all 8 SessionConfig literal sites (net session, node runtime, cli, tests loopback×4/phase3/phase5). Also fixed an unused-import warning in `mdns.rs` (split `should_listen` behind `#[cfg(feature="lan")]`).
- Kotlin: rewrote `NativeBridge.kt` (object, attach/sendNotify/deviceId/idPk/state/lastError/stop + splitAddr), `NodeSession.kt` (identity = real device_id or "unavailable", start/stop/state/lastError/enqueueNotify), `MainActivity.kt` (status line shows real device_id + live connection state via a 1s Runnable).
- Build: `cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build -p tetherly-android --release` → `libtetherly_android.so` (aarch64 ELF, NDK r29, ~1.6MB, not stripped); verified all 7 JNI symbols exported.
- **On-device E2E (real hardware)**: phones have NO route to the host overlay (10.144.144.0/24) — only adb access host→phone — so used `adb reverse tcp:45717 tcp:45717` to tunnel phone-localhost to the host desktop node (started with `tetherly run`, PIN via `POST /api/pin`). Two Galaxy Fold (SM-F900F, arm64-v8a, Android 13): each generated a real identity, paired, and the desktop received the `notify.push` (candidate `has_otp:true`, body NOT in snapshot/logs); both `trusted.json` written (mutual). **Re-ran phone 1 with a deliberately WRONG PIN — still connected → proved Noise IK resume, not re-pairing.** Second phone paired on a fresh PIN (the first PIN had expired past its 3-min TTL, which is why an early attempt got "early eof").
- CI: rewrote the `android` job to install the rust `aarch64-linux-android` target + `cargo-ndk`, build the `.so` into jniLibs, then run Gradle. `android/app/src/main/jniLibs/` added to `.gitignore` (build artifact, never committed). android/README documents the native build + probe workflow.
- Verification: `cargo fmt --all --check` clean, `cargo clippy --workspace --all-targets -- -D warnings` clean, `cargo test --workspace --locked` green (incl. new android crate: 3 unit + 1 host e2e). SSOT bumped v1.9 → **v1.10** (header + new 1.10 changelog block, §9.6 rewrite, §8 workspace, §12 M1.1/M1.5 rows, §14 CI, §19 item 5, waiver block).
- Manual-required (narrowed): physical Android M1.1 **p95 timing** + 8h soak (the `.so`-missing blocker is gone); APK-side Kotlin→JNI→session UI path on a real phone.
- Next: run `cargo deny check`, then report (commit/push only on explicit user instruction).

## 2026-10-11 (5) - Phase 3 real-machine key/mouse chain (v1.9), DeskFlow-niche gap closed
- Task: user asked how to close the DeskFlow-niche gaps. Answer: the last mile is M3.1 real injection + capture; implemented both, plus M5.1 pixel capture closed via the injection itself.
- Landed: (1) SystemInputSink (platform.rs, cfg(windows)): SendInput - TIN1 1000-units to 65535 absolute coords, BTN_LEFT/RIGHT/MIDDLE masks, wheel (WHEEL+HWHEEL), VK keyboard; NodeConfig.input_sink: Option<Arc<Mutex<dyn InputSink>>>; (2) controlled-side mirroring: attach_input_client forwards newly applied MemorySink-trace actions to the real sink (protocol filtering stays authoritative); (3) Node::spawn_input_capture (controller): WH_MOUSE_LL always armed (cursor-only, edge detection), WH_KEYBOARD_LL armed ONLY while the seat is remote (local-keyboard privacy), injected events filtered via LLMHF/LLKHF_INJECTED (no single-machine feedback loop), events feed the same drive_input* path as CI; added drive_input_button/wheel; (4) TIN1 semantics pinned in SSOT 9.5 (VK codes, 1000 units, mask bits); (5) bin wires both sides; (6) screen_size/cursor_pos helpers exported.
- Probe evidence (temp test, deleted after use): pair two in-process nodes, controller crosses right edge, controlled machine's OS cursor really moved to (0,539); mid-screen move to (959,539) precise; Win/Esc keyboard injection through the same seat; the injected desktop change made DXGI deliver a real 1920x1080 frame (8,294,400 bytes = w*h*4 exact) - M5.1 pixel-capture gap CLOSED.
- Debug en route: probe's open_input direction was inverted (server=controller dials; fixed per phase3 tests) and inputport caps needed waiting - both are probe bugs, not library bugs; the four viewer input break arms now log warns (added).
- Verification: fmt/clippy/cargo test --workspace --locked 178 passed (+1 units-abs conversion test; +1 present smoke; +1 grab-recovery earlier) / deny clean. SSOT bumped to v1.9 (9.5, M3.1, M5.1, checklist item 10, platform matrix, waiver block).
- Manual-required remaining: dual-desktop 100 physical round-trips (needs two interactive desktops), macOS/Linux capture+injection, cross-desktop p95.
- Next: push to main.

## 2026-10-11 (4) - root-cause fixes from the production code review
- Scope: fix all findings from the final review (pump break, read timeout, GDI leak, viewer error swallowing, parsing testability, unbounded read_line, window clipping, zstd bound).
- Fixes: (1) `ScreenSource::interrupt()` default-no-op added to the core port (OS-free); runtime `screen_send_frame` now grabs via `spawn_blocking` + 10s `timeout`, calling `interrupt` on overrun — a wedged adb stream EOFs and unwinds instead of stalling the pump forever; (2) bin pump: warn + exponential backoff (250ms→5s) + continue on any error (was: debug-log + permanent break, killing the stream for every future visitor); `MissedTickBehavior::Delay`; (3) `AdbSource`: child handle in its own mutex so `interrupt` can kill without taking the reader lock (no deadlock); `impl Drop` kills the child; stream reset clears both; (4) viewer `attach_screen_client`: all four silent `Err(_) => break` arms now `warn!` with the cause (assembler/decompress/receiver/recv) — future stalls are diagnosable; (5) `SystemSink` reinit destroys the old DIB+DC (`DeleteDC`+`DeleteObject`) — leaks an ~8MB DIB per resolution change before (pre-existing); (6) present window moved to a dedicated thread WITH a message pump — the pumpless window silently failed cross-thread `SetWindowPos` (and would hang `PrintWindow`); resize is now `PostMessageW(WM_APP_RESIZE)` handled on the window's own thread, so the window is sized to the frame (verified live: 736x1100 for a 720x1280 phone, width exact, height clamped by the 1080p monitor); present waits up to 2s for window init instead of dropping the first frame; (7) AdbSource parsing extracted into tested pure functions (`parse_len_line`, `parse_raw_header`, `swap_rb_in_place`) + 5 unit tests; `read_line` bounded via `.take(128)`; device temp file `rm -f` at stream start; (8) `ZstdCodec` capacity now the exact `ZSTD_COMPRESSBOUND` formula.
- E2E (release, cloud phone → Windows pair): A sent=467 / B received=467 / 0 dropped (467 × 720×1280×4 exact), zero WARN, zero grab-timeouts, window resize live-verified. Earlier "received=1" reproductions were stale-process environment contamination, not code.
- Verification: fmt/clippy/`cargo test --workspace --locked` **177 passed** (+5 AdbSource parsing unit tests, +1 present smoke (windows-gated), +1 grab-failure recovery contract via `MemoryScreenSource::fail_on_grab`) / `cargo deny check` clean.
- Next: push to main; open encoders; macOS/Linux.

## 2026-10-11 (3) - screen hot-path performance pass (user: 性能完全优化)
- Scope: evidence-first. Baseline bench (new `examples/bench_screen.rs`, kept) at 1080p/8.3MB: zstd stream L3 enc 18.4ms/dec 8.4ms (fresh ctx per frame), to_frames 2.07ms, assembler 3.0ms; workspace had NO [profile.release].
- Optimizations landed: (1) `ZstdCodec` persistent bulk contexts (Mutex, `compress_to_buffer`, bound-sized buffer, exact-len decompress into preallocated Vec) — enc 12.6ms/dec 4.1ms in release; (2) `ScreenAssembler` offset-based reassembly (one preallocated body + mask, chunk-length validation) — 1.28ms (-57%), core stays OS-free/no-unsafe; (3) `to_frames` single-write fragment header; (4) `SystemSink::present` single memcpy straight into the `CreateDIBSection` bits pointer (no staging Vec, no per-frame GetObjectW; `dib` field is lifetime-load-bearing); (5) `[profile.release] lto=thin, codegen-units=1`; (6) `AdbSource` redesigned to ONE persistent stream (kills per-frame adb spawn + CREATE_NO_WINDOW + stdin null).
- AdbSource bandwidth measurements drove the design: overlay adb leg ~187KB/s → raw 3.7MB/frame = 17.5s/frame (unusable) vs PNG ~31KB = 0.17s. So default = persistent length-prefixed PNG loop (`screencap -p > tmp; wc -c < tmp; cat tmp` with pid-suffixed tmp), raw RGBA stream kept behind `TETHERLY_ADB_RAW=1` for fast links (USB/plain LAN), auto-fallback raw→PNG on unsupported header; one-shot PNG kept as last resort. Dedup unchanged (PNG/raw byte equality → `Ok(None)`).
- Real-device probe (Fold 720x1280): persistent PNG stream first frame 843ms, steady 436ms/grab vs one-shot ~543ms (spawn elimination ~20%); device screencap+PNG-encode cycle is the floor.
- E2E (release build, cloud phone → Windows pair): 31 sent / 24 received / 0 dropped in 30s, 17KB/frame zstd wire unchanged. p95 2.39s is NOT a regression: with fast dedup `Ok(None)` ticks, p95 now measures the screen's content-change period (loading animation has 2-3s still phases), not transport cost.
- Verification: fmt/clippy/`cargo test --workspace --locked` 170 passed/deny all green. Bugs fixed en route: missing `.stdout(Stdio::piped())` on the stream spawn; `GetObjectW`/`BITMAP` unused imports; AdbError Display/CoreError conversions.
- Next: push to main; open encoders; macOS/Linux.

## 2026-10-11 (2) — AdbSource cross-device E2E on cloud phones
- Task: user offered cloud phones (有图像) for real-pixel testing. 2× Galaxy Fold SM-F900F (Android 13, arm64) on adb: 10.144.144.51:5555 (720x1280 override) + :52 (1080x2376), via EasyTier overlay + 192.168.1.4 LAN. Chose AdbSource route (user-confirmed) over Android MediaProjection (new-立项, 1-2d).
- Actions: added `AdbSource` (screen.rs, portable): `adb exec-out screencap -p` → png 0.18 decode → RGBA→BGRA convert → packed `ScreenFrame::raw(Bgra8)`; byte-identical PNG → `Ok(None)` (DXGI-style idle); `TETHERLY_ADB` env override. Bin: `run --screen-adb <serial>` wires `EncodedSource<AdbSource>`; also `--file-port/--input-port/--screen-port/--ui-port` for two nodes on one host. `screen_send_frame` now skips grab unless engine state is Streaming (idle adb spawn would otherwise run continuously). E2E found + fixed 2 real gaps: (1) visitor UI `/api/screen/start` never called `open_screen` — now auto-dials the single live peer (multi-peer → honest refusal); (2) visitor receive counters were task-local — added `Shared.screen_rx_stats` + `Node::screen_rx_stats()`, merged into Snapshot (received/dropped/bytes now live on B).
- Results (real E2E, two `tetherly run` processes, paired via UI API): A(host, `--screen-adb 10.144.144.51:5555`, ports 45817-45820/45816) → B(viewer, SystemSink GDI). Flow: PIN(A) → dial(B) → start#1(B, auto-open, Start refused=1 M5.2) → allow(A) → start#2(B) → streaming. 25s window: A sent=46/781,977B zstd wire, p95=901,097µs; B received=18. Long-run check (~20min): A sent=1063, B received=1060 / 0 dropped / 3,907,584,000B = 1060 × 720×1280×4 exactly. GDI window screenshot (FindWindow TetherlyScreen + SetForegroundWindow + CopyFromScreen; PrintWindow hangs — no message pump on present thread) shows the live phone screen (clock 13:13 matches capture time); pixel match vs fresh adb screencap: 97.6% within ±8 (residual = loading-animation phase + DWM color management ~5.2/channel), exact-match impossible across DWM. Note: `cargo build` at root does NOT build the bin (tetherly-node bin is dev-dep of root test pkg) — use `cargo build -p tetherly-node`.
- Verification: fmt/clippy/`cargo test --workspace --locked` (170 passed) / `cargo deny check` all green. Probe (deleted after use) confirmed AdbSource grab1 918ms 720x1280 packed BGRA + grab2 idle-path.
- Next: push to `main`, confirm GitHub Actions green. DXGI-side pixel capture still needs an interactive desktop; OpenH264/rav1e; macOS/Linux.
- Blockers: unchanged — open encoders, DXGI interactive-desktop capture, macOS/Linux.

## 2026-10-11
- Task: Close Phase 5 real-machine loop (真机闭环); spec v1.8. Windows DXGI Desktop Duplication capture + zstd lossless + GDI BitBlt present + p95 frame-interval budget, all as production screen chain on the host.
- Actions: `crates/tetherly-node/src/screen.rs` gained `DxgiSource` (D3D11 BGRA → IDXGIOutput1::DuplicateOutput; `AcquireNextFrame` WAIT_TIMEOUT → `Ok(None)` so an unchanged desktop yields no frame; staging texture + `Map(READ)` + row-tail trim into packed BGRA8) and `ZstdCodec` (lossless level 3, `ScreenCodec` port) + `EncodedSource<S>` (per-frame compress, emit raw when not smaller) + `SystemSink` (GDI `CreateDIBSection` 32bpp top-down + synchronous `BitBlt`, no message pump). Wired `NodeConfig.screen_codec`/`screen_sink`, `Shared.screen_codec`/`screen_sink`/`screen_interval`, `ScreenIntervalTracker` (wall-clock µs, bounded 1024) with `Node::screen_latency_p95_us()`, `ScreenFrame::to_frames()` + `ScreenAssembler::push` fragmentation/reassembly, and client decompress via `screen_codec.decompress(expected_len)` before the seq window. `tetherly run` injects `SystemSink` + a 16ms frame pump. UI field `Snapshot.screen.p95_frame_interval_us`.
- Results: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all green (core 116, phase1 7, phase2 4, phase3 4, phase4 10, phase5 4, loopback 10, crypto 10, net 1, node 4); `cargo deny check` clean (zstd/windows permissive). Real-machine run on this Windows host: DXGI device init succeeds (D3D11→adapter→output→Duplication, no error), WAIT_TIMEOUT→Ok(None) idle path works correctly, and GDI present succeeds (60/60 `SystemSink.present` calls). Actual pixel capture was not exercised because this agent shell has no interactive desktop session for DXGI to flip (static desktop → AcquireNextFrame always WAIT_TIMEOUT). Real frame data + cross-desktop p95 + p95 metric remain Manual-required. Docs bumped to v1.8 (`DEVELOPMENT.md` §9.7/§2.3/§12/§23, `.codex-memory/`, Obsidian note + MOC/router rows).
- Next: push to `main`, confirm GitHub Actions green.
- Blockers: no interactive desktop in this agent shell (DXGI pixel capture not exercisable here); no physical dual-desktop or open encoder (OpenH264/rav1e) on this host; macOS/Linux screen loop remains next iteration.

## 2026-10-10
- Task: Land Phase 5 remote screen (screen streaming) CI slice; spec v1.7; RustDesk architecture borrow only, AGPL not linkable.
- Actions: new `tetherly-core::screen` (OS-free `TMV1` frame + `ControlMsg{Start,Stop,Ack}` codec, `ScreenSender`/`ScreenReceiver`/`ScreenState`, `MemoryScreenSource`/`MemoryScreenSink`, `ScreenSource`/`ScreenSink` ports); new TCP port 45720 (`SCREEN_PORT`) with `NetError::ScreenRequiresTrust` for unpaired dials, `screen_only` sessions reject `HelloDisposition::Pair`; mandatory local consent `ScreenState{Idle,Requested,Allowed,Streaming}` — peer connecting only sets `Requested`, `Node::screen_allow()` must be called explicitly before `Start` is honored; `tetherly-node` 14-anchor screen wiring (`bound_screen`/`screen_out`/`screen_engine`/`screen_sink`/`screen_ctl`/`screen_source`/`spawn_listeners`/`spawn_overlay`/caps/`LivePeer.screen_port`/`open_screen`/`accept_screen`/`attach_screen_server`/`attach_screen_client`/`screen_send_frame`); `crates/tetherly-node/src/screen.rs` with `SystemSource`/`SystemSink` stubs (return `ScreenRefused`, real capture Manual-required); `deny.toml` ban `hbb_common`/`rustdesk`/`rustdesk-server`/`scrap`; UI routes `/api/screen/allow|start|stop` + `ui/index.html`「远程屏幕（实验）」section; `tests/phase5.rs` M5.1–M5.4 (4 tests green: consent frame-seq, Start-before-allow refused, peer-gone → Idle, 45720 unpaired rejected).
- Results: local fmt/clippy `-D warnings`/`cargo test --workspace --locked`/`cargo deny check` all green (loopback 10, phase1 7, phase2 4, phase3 4, phase4 10, phase5 4, core 112, crypto 10, net 1, node 4). Real screen capture/encoder/presentation stays Manual-required. M5.5 not started. RustDesk AGPL code zero-link confirmed.
- Next: push to `main`, confirm GitHub Actions green.
- Blockers: No real GPU/display on CI runners; Wayland unsupported for screen capture.

## 2026-10-10
- Task: Open Phase 4 (waiver granted) and land the computer-side iPhone ANCS CI slice; spec v1.6.
- Actions: new `tetherly-core::ancs` (UUID/event/CP codec, `DataAssembler` byte-wise reassembly, `AncsIngress`, `AncsTransport`, `MemoryAncsTransport`, `ancs_actions`, `ancs_source_id`); §9.4 constraints made structural (subscribe order, enqueue-only callback + `tick` write, serial `in_flight`, one retry of silence, 500ms→30s backoff, uid dedup, `PreExisting` drop); `Opener` port (`SystemOpener` no-shell / `MemoryOpener`); node `ancs_*` surface + `open_candidate`; `Candidate.actions` carried to DTO; UI two-step wizard and `/api/ancs/*`, `/api/open`; `tests/phase4.rs` (10 tests, M4.1/M4.2/M4.3 + serial/reassembly/Removed/inert/backoff).
- Results: local fmt/clippy `-D warnings`/`cargo test --workspace --locked`/`cargo deny check` all green (loopback 10, phase1 7, phase2 4, phase3 4, phase4 10, core 93, crypto 10, net 1, node 4). Real BLE GATT + physical iPhone stay Manual-required.
- Next: push to `main`, confirm GitHub Actions green.
- Blockers: No real BLE stack or iPhone on this host.

## 2026-10-09
- Task: Harden the landed Phase 3 input slice and remove a CI flake.
- Actions: `InputServer::local_move` now forwards normalized `Move` when the seat is already `Remote` (motion was silently dropped after the edge); `MemorySink.applied` bounded by `INPUT_SINK_TRACE_MAX` (was unbounded + cloned per frame); integration `wait_live` budget raised 8s → 25s to exceed `HANDSHAKE_TIMEOUT` (20s), which was flaking `m2_5` on loaded Windows runners.
- Results: local workspace tests green (loopback 10, phase1 7, phase2 4, phase3 4, core 71, crypto 10, net 1, node 4); CI runs `38006315548` and `38006923616` all five jobs **success**.
- Next: Stop. Phase 4 needs a new waiver.
- Blockers: None.

## 2026-10-09
- Task: Land Phase 3 native keyboard/mouse CI slice; spec v1.5; push so GitHub CI compiles and tests.
- Actions: `tetherly-core::input` (binary `TIN1`, seq window, `InputServer`/`InputClient`/`MemorySink`, edge state machine); node listens on separate TCP 45719 with `SessionConfig.resume_only` (pairing still 45717); `caps.update inputport=`; input frames only on the 45719 channel; `tests/phase3.rs`.
- Results: local fmt/clippy `-D warnings`/`cargo test --workspace --locked`/deny green. First CI run `37989970083` failed: Windows `m3_2` (shared `identity.bin.tmp` collision) and Android unit tests (`org.json` stub). Fixed with per-write temp names and a real `org.json` test dependency. CI run `37997269866` all five jobs **success**.
- Next: Stop. Do not open Phase 4. Physical SendInput / dual-desktop remain Manual-required.
- Blockers: No real dual-desktop or OS injection in CI.

## 2026-10-09
- Task: Land Phase 2 EasyTier sidecar CI slice; spec v1.4; do not open Phase 3.
- Actions: overlay CIDR/PathKind + sidecar RPC/CLI/iface discovery; overlay bind separate from LAN/mDNS; LAN-wins attach; UI「仅局域网」; `tests/phase2.rs` M2.2–M2.5; clippy nits; isolate host TUN with `10.199.199.0/24`.
- Results: fmt/clippy `-D warnings`/workspace tests (loopback 10, phase1 7, phase2 4, core 63)/cargo deny green. No `easytier*` in Cargo.toml.
- Next: Stop. Do not open Phase 3. M2.1 physical cellular p95 remains Manual-required.
- Blockers: No physical phone + cellular EasyTier soak in this session.

## 2026-10-09
- Task: Land Phase 1 CI slice (LAN desktop + Android skeleton); update spec to v1.3; push.
- Actions: `tetherly-node` persist/LAN/notify/clip/file; loopback HTTP UI; Android NLService; `tests/phase1.rs`; `copy_otp` CandidateExpired; deny BSL-1.0; DEVELOPMENT.md v1.3; Obsidian 速读.
- Results: workspace tests green (loopback 10, phase1 7, core 58); clippy `-D warnings`; cargo deny licenses/bans/sources ok.
- Next: Stop. Do not open Phase 2. Physical Android / JNI `.so` remain Manual-required.
- Blockers: No physical phone in this session; no cargo-ndk `.so`.

## 2026-10-09
- Task: Root-cause-fix Phase 0 review findings; push GitHub; mount Obsidian.
- Actions: Split PairBind AEAD nonces (`tetherlybndI`/`tetherlybndR`); commit trust only after Noise transport; Clock + handshake 20s timeout + TCP 30/min; UTF-8 truncate; RNG fail-closed; inner `msg_id` + ReplayGuard + `needs_rekey`; on-wire Hello-name MITM test; delete tmp helper.
- Results: clippy `-D warnings` and `cargo test --workspace --locked` green. Spec bumped to v1.2. Public repo created.
- Next: Phase 1 only on explicit request.
- Blockers: None.

## 2026-10-09
- Task: Fix Windows CI `cargo fmt --check` newline failure.
- Actions: Add `.gitattributes` (`eol=lf`) so Windows runners do not rewrite rustfmt Unix newlines; disable `core.autocrlf` on the Windows job before toolchain setup.
- Results: Pending CI after push.
- Next: Confirm Windows check is green.
- Blockers: None.

## 2026-10-19
- Task: Tetherly v1.53 Production Code Review Root-Cause Fix, Verification, and Dual-Archival (Obsidian & GitHub).
- Actions:
  1. Root-cause fixed P1 multi-file cumulative `total_bytes` sum in `file_offers` (`crates/tetherly-node/src/runtime.rs`);
  2. Root-cause fixed P2 `user_accept_file_reported` and `user_reject_file_reported` state machine race condition and lock granularity (consolidated to single atomic `hubs` lock scope);
  3. Deduplicated `user_accept_file` and `user_reject_file` by directly wrapping their `_reported` variants;
  4. Added multi-file offer cumulative size (4000 bytes) and atomic decision report assertion in E2E integration test `tests/phase1.rs`;
  5. Multi-platform CI verified: GitHub Actions CI run `39162786508` passed 100% green across all 5 jobs (`deny`, `android`, `check (ubuntu-latest)`, `check (macos-latest)`, `check (windows-latest)`);
  6. Documentation synchronized: `docs/DEVELOPMENT.md` ADR-046 Section 10, Obsidian canonical note `Note/Project/tetherly/tetherly 开发文档.md`, and root router `00.MOC/AI-DOC-ROUTER.md`;
  7. Ran `scan-stale-docs` in Obsidian vault (1462 files scanned, 0 stale hits); pushed Obsidian vault commits to `origin master`;
  8. Updated `.codex-memory` project state and session log.
- Results: 100% green CI on all platforms, zero-plaintext security invariants strictly preserved, documentation and code archives fully synchronized to Obsidian and GitHub.
- Next: Ready for next architectural milestone.
- Blockers: None.

## 2026-10-19
- Task: Land Tetherly v1.57 (ADR-050) macOS ANCS CoreBluetooth 传输栈架构、跨平台外设地址抽象与守护进程链路修复.
- Actions:
  1. Abstracted cross-platform Bluetooth Low Energy peripheral addressing (`BleAddress` enum with `Mac([u8; 6])` and `Uuid([u8; 16])`) in `crates/tetherly-core/src/ancs.rs` and `lib.rs`;
  2. Implemented universal parser `parse_ble_address` and canonical formatting `to_canonical_string`;
  3. Fixed critical daemon startup bug in `crates/tetherly-node/src/bin/tetherly.rs` where CLI `--ancs <addr>` discarded transport and left `cfg.ancs` empty; properly wired `cfg.ancs = Some(transport)` and `cfg.ancs_peripheral = addr` into `NodeConfig` before `Node::start`;
  4. Implemented native macOS CoreBluetooth GATT transport stack in `crates/tetherly-node/src/ancs/macos.rs` and wired it into `crates/tetherly-node/src/ancs.rs` using `objc2-core-bluetooth` and GCD serial dispatch queue;
  5. Extended `AncsReport` with `peripheral: Option<String>` in `crates/tetherly-node/src/runtime.rs` and displayed peripheral address in CLI daemon status;
  6. Added comprehensive unit and regression tests in `tetherly-core` and `tests/phase4.rs` (`test_ble_address_and_daemon_ancs_wiring_regression`);
  7. Ran full workspace verification: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, full test suite pass (325+ tests green), `cargo deny check` passed;
  8. Synchronized documentation: updated SSOT `docs/DEVELOPMENT.md` to v1.57 (ADR-050), updated `.codex-memory/project-state.md` and `session-log.md`.
- Results: All checks and tests passed 100% green. Daemon startup ANCS bug fixed, macOS CoreBluetooth stack operational, cross-platform address resolution complete.
- Next: Update `.codex-memory/todo.md` and Obsidian knowledge base docs.
- Blockers: None.
