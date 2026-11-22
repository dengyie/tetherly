# Session log
## 2026-11-20 · 云手机人工恢复 + v0.2.68 真机验收全绿（4 节点 online）
- 用户开屏恢复两台云机：Termux keepalive 自动拉起 easytier（验证手册机内恢复路径有效）。mesh 实测：cloudphone-1 p2p 19.6ms / cloudphone-2 p2p 28ms，`no_tun=true` UDP 模式；adb 5555 双机 device。注意 .52 ping 不通 mesh 对端（ICMP 被 easytier UDP 隧道丢弃属正常），TCP 才是真实链路。
- tetherly 验收补全：两台 reverse tcp:45717 重下后，`.51` 升 0.2.68 → force-stop→relaunch `tdev_4kcq2woqljktkk6y · 已连接`；`.52` 升 0.2.68 → `tdev_breflgheng3owukz · 已连接`。Mac `/api/v1/devices` **4 节点全 online**（Mac/Win/两 Android）。v0.2.67/68 自动重连双机验收通过。
- `.51` 升级前出现的「device_id: unavailable · 本机库未加载」为 v0.2.66 遗留（lib/arm64 空目录 + extractNativeLibs=false 的加载路径问题），升级 0.2.68 后自愈——若复现按「重装最新 APK」处置。
- 教训固化：云机 VM 恢复后 adb reverse 表清空，45717 reverse 必须重下（与 5173/8000 同类）；本机库未加载不一定是 lib 缺失，先看 UI 是否 PIN 未配对。

## 2026-11-20 · v0.2.68：会话所有权治理（暂态回收不再拆线）
- 深审 v0.2.67 自动重连链路发现矛盾：`NotifyListener.onDestroy/onListenerDisconnected` 与 `ForegroundKeep.onDestroy` 无条件 `session.stop()`——ROM 暂态重绑通知监听（.51/.52 现场实证的模式）即拆线，且无任何路径重启（TetherlyApp 自动启动只在进程冷启动跑一次），配对安装从此静默失联直至下次冷启动。前台服务同理被一并停掉。
- 修复（commit **eed4612**）：所有权决策收敛到新 `SessionOwnershipPolicy`（5 事件决策表，JVM 单测锁定）：仅 `EXPLICIT_STOP`（ForegroundKeep.stop，现显式补 session.stop）与 `FGS_TIMEOUT`（已含 RestartCoordinator 立即重拨）拆线；`LISTENER_DISCONNECTED/LISTENER_DESTROYED/SERVICE_DESTROYED` 不动会话，重绑落入幂等 `onListenerConnected`。
- 本地验证：Rust android crate 3/3、fmt/clippy/diff-check 过；本地 JVM 跑不了——本机仅 JDK 25，AGP 8.7.3 拒绝（要求 ≤21），JVM 单测（SessionOwnershipPolicyTest 3 例等）由 CI Android job 验证。
- CI 44629841213 六作业全绿（android unit + assemble 均 BUILD SUCCESSFUL）；release 44631506377 全绿 → **v0.2.68 发布 12 资产**（APK SHA256 `45a2873d…`，SHA256SUMS 一致）。
- 用户指令：项目迭代优先，云手机验证暂停；`.52` reboot 后 frpc 未恢复两台仍失联（需云控制台人工恢复）。
- 红线入册（用户死命令）：[[Note/Infra/云手机 ADB 中继运维手册]] 速读区 + 变更记录双写「严禁重启两台三星云手机」，冷启动类测试一律降级 force-stop + relaunch，整机重启须用户批准。

## 2026-11-20 · v0.2.67：Android 配对安装自动重连（现场缺陷闭环）
- 现场缺陷（.51/.52 真机发现）：配对节点 force-stop/升级后冷启动停在连接表单，需手动点「保存并连接」。修复 commit **2c00c08** `feat(android): §172 — auto-reconnect a paired install on process start`。
- 实现：`AutoConnectPolicy.kt` 冷启动门控（`desktopAddr.isNotBlank() && identity.bin 已持久化` 才自动拨号，全新安装永不自拨默认地址）；`NodeSession.ensureStarted()` 幂等门（替代裸 attach，避免 listener/前台路径与 Application.onCreate 竞争时拆线重拨）；`TetherlyApp.onCreate` 自动启动；`ForegroundKeep` start/onStartCommand 收敛到 ensureStarted。3 个 JVM 单测（paired 自启 / fresh 永不 / 空地址阻断）。
- CI run **44606444671 全绿**；release run **44609260621 全绿**，v0.2.67 发布 13 资产（SHA256SUMS、release 签名 APK、dmg/msi、三平台 bin+CLI+viewer）；APK SHA256 `67b183c…fc35e0f` 与 SHA256SUMS 一致。
- .52 真机验收（release v0.2.66→0.2.67 升级安装）：force-stop → relaunch，UI 直接显示 `device_id: tdev_breflgheng3owukz · 已连接`（未点任何按钮），ForegroundKeep FGS dataSync isForeground=true startForegroundCount=1，NotifyListener 在 enabled_notification_listeners —— **自动重连功能验收通过**。
- 开机自动重连验证中断：`.52 adb reboot` 后 Termux frpc 隧道未自恢复，5555 端口超时（与 .51 重启后同模式）；两台云机均需云控制台侧重启 Termux/frpc 后补验。后台 5 分钟间隔探针挂起等待恢复。
- 结论：v0.2.67 产品缺陷闭环完成；「paired 节点冷启动不自动重连」观察项销项。残留：boot-time 路径验证（.52 开机 + .51 恢复后）。

## 2026-11-20 · v0.2.66 Android 云手机真机验收（用户授权云手机测试）
- 两台三星云手机（SM-F900F / Android 13，10.144.144.51/52:5555）从 debug 签名旧版升级 release 签名 v0.2.66（versionCode 2066，签名密钥 vault Note/Accounts/ai/tetherly-release-signing/）。
- .52 全链路 GREEN：adb reverse 45717 → UI PIN 配对（Noise）→ tdev_breflgheng3owukz 已连接；Mac devices online+trusted rtt 20–30ms；NotifyListener 被系统绑定；`cmd notification post` 测试通知 → 零明文 candidate（TetherlySmoke/Shell）到达 Mac 活动流；捕获即触发 ForegroundKeep FGS（dataSync，startForegroundCount=1，PROC_STATE_TOP）——§172 FGS 真机生命周期通过（Android 13 无 6h onTimeout 语义，重启路径仍列 Android 15 Manual-required）。
- .51 配对成功（tdev_4kcq2woqljktkk6y 已连接）后 force-stop→relaunch 未自动重连（UI 需手动连接，记录为产品观察项：paired 节点冷启动不自动重连）；其 settings put 监听写入未触发 NMS 重绑（binding died 后不再自动恢复，toggle 无效），reboot 恢复中其 frpc 隧道未回（云机控制台侧需重拉），开机绑定验证待 .51 恢复后补。
- UI 自动化取证：Compose 坐标随分辨率变化须从 uiautomator dump 提取；`input text` 空格截断，PIN 需去空格。
- 配套修复/沉淀：deploy.sh all-target digest `--skip-existing`、SHA 锚点 CRLF 清理、冒烟脚本 SYSTEM owner 探针 + UTF-8 控制台 + null size；vault 速记两条流水；doc-check PASS。


## 2026-11-20 · §172 收口：v0.2.66 发布 + 双端部署 + 现场冒烟 19/19
- CI 五轮收敛（985485b→3e62b10）：frontend 提交 dist 契约、三平台 clippy（shell `sse_response` cfg(test)、clipboard allows、store needless ?、windows-sys 0.59 ACE 常量在 Security 根）、loopback reject-mock 补请求消费（Windows RST 掩盖响应头）、backpressure recv 测试 socket2 钉缓冲确定性化。最终 run **44573908834 全绿**。
- Release 四轮收敛（b0aeb63→2bb5a50 重打 tag×3）：frontend job pnpm 先于 setup-node、apkanalyzer/apksigner 经 ANDROID_HOME 定位、publish 剔除 frontend 辅助文件；Android release 签名密钥生成入 vault `Note/Accounts/ai/tetherly-release-signing/` 并写 4 个 repo secrets。最终 run **44585068559 全绿**，v0.2.66 发布 12 资产（SHA256SUMS、release 签名 APK、dmg/msi、三平台 bin+CLI+viewer）。
- Deploy：macOS health 0.2.66 ✓（package-macos-app stdin-config curl 关停）；Windows 修两处现场缺陷（all-target digest 重复下载 `--skip-existing`、PowerShell CRLF 破坏 SHA 锚点），热重启 19764→6104、readiness PID/version/start-time ✓、SYSTEM ✓、受保护进程 3/3 ✓。
- Smoke 三轮迭代后 **19/19 PASS**：session 断言改 SYSTEM owner 探针（session 非稳定契约）、remote_v1 强制 UTF-8 控制台（隔离实验证明产品完整传输 emoji，`??` 为脚本控制台编码）、auto-accept 接受 null size、CIM 引号层级对齐 deploy.sh。
- 治理：doc-check PASS 4/4；Obsidian canonical 基线更新 v0.2.66 双端部署；发布冻结随本次发布解除；Manual-required 残余单列（Android 15 真机长稳、sidecar 长期观察、smooth-wheel 承诺确认）。

## 2026-11-19 · 深度 review 修复路线图（v0.2.65）
- 汇总当前 HEAD `3609ca1` 的安全、资源、生命周期、客户端、CI/release/deploy 审计，形成 [`docs/REVIEW_REMEDIATION_PLAN.md`](../docs/REVIEW_REMEDIATION_PLAN.md)。
- 登记 P1/P2 修复批次：权限边界与本地副作用、HTTP/文件/session 资源上界、状态机 teardown、客户端可靠性、CI/release/deploy gate。
- 明确发布冻结：P1/P2 定向 RED/GREEN、GitHub CI、release artifact 和现场 readiness/rollback 证据完成前，不新增 tag、release 或生产部署。
- 本轮随后完成本地 remediation：node lib **306/306**、loopback **12/12**、API bindings+contract **43**、net backpressure **3/3**、Tauri SSE **8/8**；node check、CLI check、bash/python 静态门禁和目标旧 API grep 通过。
- 已落地：私有 send staging/路径 containment、Admin-only POST export + 原子 `create_new`、legacy/v1 SSE 鉴权与 parser、file listener admission/header timeout、recv flush timeout、camera ownership、browser/Tauri SSE、Android reply/service 代码、CI/release/smoke route 迁移。
- 后续 remediation 代理已补齐：raw inbox 句柄+64 KiB 分块响应；file connect/header/chunk/total deadline；单目标 offer/accept rollback；同源同元数据 duplicate admission；hard-link atomic inbox reservation；screen generation/privacy/ticket failure cleanup；clipboard preview/commit + bounded per-peer retry；browser/Tauri SSE handshake/idle/sequence；Android quick-reply queue 与 foreground timeout/restart 状态机；CI/release/deploy/smoke gate 代码和静态契约。
- 当前未完成：multi-peer queue-full 高层集成证据、高层 stalled-transfer 集成证据、timeout/shutdown 后 partial/sidecar 现场策略确认；未运行 Android Gradle/unit/NDK、GitHub CI、workspace 全量、Windows/macOS release/deploy、真实 Android/浏览器/WebView/跨平台现场验收；发布冻结继续有效。
- 新增 GREEN：`runtime::lifecycle::tests::terminal_transfer_transition_clears_outbound_state` 1/1；`runtime::lifecycle::tests::shutdown_transitions_outbound_transfer_and_outbox_to_terminal` 1/1；node `cargo check --lib --tests --locked` 通过。multi-peer failure branches and shutdown now reuse terminal transition to clear pending/cancel state and update outbox/stats.
- 深度优化迭代：前端 13 个 check 错误根因修复（tetherly-api decl 列表补 `TransferStats`/`TrayOwner`/`DisplayModeDto` + 重新 bless + 删除临时 shim；fixture 补新字段），`pnpm check` 0 errors / `pnpm test` 36/36；P3 storage/open 绝对路径按 tier 脱敏（Admin 保留，client token 相对占位，node lib **315/315**）；P3 macOS 打包 token 改 curl stdin config 不进 argv；生成器 decl() 出口逐行 trim，blessed api.ts 无尾随空格。
- 传输集成证据：`inbound_stall_tests::stalled_inbound_transfer_fails_and_keeps_resume_state`（cfg(test) 500ms 注入窗口，RED 4.05s→GREEN 0.55s，.part/sidecar 保留）；`multi_peer_file_dispatch_isolates_queue_full_failure_to_its_peer`（突变测试 RED→GREEN 0.03s）。node lib **317/317**、net lib 13/13。剩余 Residual 仅 sidecar 现场确认与外部平台/CI 门禁。

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

## 2026-10-19
- Task: Land Tetherly v1.62 (ADR-055) 跨端收件箱文件内容提取、流式下载、系统级直开与多媒体预览全闭环.
- Actions:
  1. Extended `UiEvent` in `crates/tetherly-node/src/events.rs` with `InboxFileOpened` variant adhering to Zero-Plaintext invariant (only sanitized metadata, no file contents or private keys);
  2. Implemented `InboxOpenReport`, `mime_type_from_name`, `open_inbox_file`, and `read_inbox_file` in `crates/tetherly-node/src/runtime.rs` with strict multi-layer path traversal guards (`CoreError::UnsafeFileName`);
  3. Added thread-safe injectable `Opener` on `Node` (`set_opener` / `opener`) defaulting to `SystemOpener` (macOS `open`, Windows `rundll32`, Linux `xdg-open`) while allowing `MemoryOpener` for tests;
  4. Upgraded `uihttp.rs` from static 3-tuple responses to full `UiResponse` structure with dynamic MIME types and `Content-Disposition` headers;
  5. Fixed binary distortion bug in `parse_http_response` across both `uihttp.rs` and `bins/tetherly-cli/src/main.rs` by switching from `String::from_utf8_lossy` text splitting to raw byte window scanning `buf.windows(4).position(|w| w == b"\r\n\r\n")`;
  6. Added loopback endpoints: `GET /api/file/inbox/download`, `GET /api/file/inbox/raw`, `POST /api/file/inbox/open` (and `/api/inbox/*` aliases) with loopback binding and Host 403 Forbidden check;
  7. Fully upgraded dual-CLI targets (`bins/tetherly-cli` and `crates/tetherly-node/src/bin/tetherly.rs`) with `inbox get <name> [--dest <path>]` and `inbox open [<name>]`, plus SSE `events` formatting;
  8. Enhanced Web UI in `ui/index.html` with "打开目录" button, file card actions ("打开", "下载", "预览", "删除"), and SSE `inbox_file_opened` live update;
  9. Added comprehensive E2E integration test in `tests/phase1.rs`: `inbox_file_download_raw_preview_and_native_open_via_loopback_api`;
  10. Verified all workspace baselines: `cargo fmt --all --check` (clean), `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo check --target aarch64-unknown-linux-gnu` (clean), `cargo test --workspace` (all 260+ tests pass), `cargo deny check` (advisories ok, bans ok, licenses ok, sources ok);
  11. Updated SSOT `docs/DEVELOPMENT.md` to v1.62 (ADR-055), updated `.codex-memory/decisions.md`, `.codex-memory/project-state.md`, and synchronized to Obsidian vault (`Note/Project/tetherly/tetherly 开发文档.md`).
## 2026-10-19
- Task: Land Tetherly v1.63 (ADR-056) 跨端桌面原生系统级通知推送、横幅弹窗治理与免打扰闭环.
- Actions:
  1. Eliminated the historical dangling stub trait `Notifier` in `crates/tetherly-core/src/ports.rs` (one of the 6 fundamental platform ports in SSOT §2494: `Clock`, `Rng`, `TrustStore`, `Insertor`, `Clip`, `Notifier`) and upgraded it to `fn notify(&self, payload: &NotificationPayload) -> Result<(), CoreError>`;
  2. Defined strongly-typed `NotificationPayload` and `NotificationUrgency` (`Low`, `Normal`, `Critical`) in `crates/tetherly-core/src/ports.rs`;
  3. Implemented decoupled native OS desktop notification dispatch (`SystemNotifier` via macOS `osascript`, Windows PowerShell balloon/toast, Linux `notify-send`) and mockable test dispatch (`MemoryNotifier`) in `crates/tetherly-node/src/platform.rs`;
  4. Extended `UiEvent` in `crates/tetherly-node/src/events.rs` with `NotificationBannerSent` and `NotifyConfigChanged` variants, rigorously enforcing the Zero-Plaintext invariant (only sanitized title, app source, OTP flag, and timestamp; strictly omitting message body and OTP digits);
  5. Implemented centralized DND governance (`NotifyBannerConfig` with `banners_enabled: bool`, `dnd_mode`: `Off` / `Silent` / `All`, `sound_enabled: bool`, `mask_otp: bool`, `allow_otp_in_dnd: bool`), dispatch primitive `Node::dispatch_notification_banner`, and automated integration with phone push notifications (`Node::ingest_local_push`) and file transfer completion (`Node::receive_file_loop`) in `crates/tetherly-node/src/runtime.rs`;
  6. Added loopback control endpoints: `GET/POST /api/notify/config` (with `/api/notify/banner` alias) and `POST /api/notify/test` in `crates/tetherly-node/src/uihttp.rs`, strictly enforcing 127.0.0.1 physical binding and Host 403 Forbidden check;
  7. Upgraded dual-CLI targets (`bins/tetherly-cli` and `crates/tetherly-node/src/bin/tetherly.rs`) with `banner config` and `banner test` subcommands and rich SSE `events` formatting;
  8. Built "桌面原生系统级通知与免打扰 (Desktop Banners & DND)" Web UI control card in `ui/index.html` with real-time toggles, DND select, metrics display, one-click test banner, and SSE listener;
  9. Implemented comprehensive E2E integration test in `tests/phase2.rs`: `desktop_notification_banners_dnd_governance_and_otp_masking_via_loopback_api`, validating banner dispatch, memory capture, Zero-Plaintext SSE, OTP masking, critical urgency promotion, silent DND, all DND suppression and bypass, banner master switch disablement, and Host 403 rejection;
  10. Ran full workspace verification: `cargo fmt --all --check` (clean), `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo check --target aarch64-unknown-linux-gnu` (clean), `cargo test --workspace` (all 282+ tests pass), `cargo deny check` (advisories ok, bans ok, licenses ok, sources ok);
  11. Updated SSOT `docs/DEVELOPMENT.md` to v1.63 (ADR-056), updated `.codex-memory/decisions.md`, `.codex-memory/project-state.md`, and synchronized to Obsidian vault (`Note/Project/tetherly/tetherly 开发文档.md`).
- Results: All 9 tasks for v1.63 completed and verified 100% green.
- Next: Git commit keeping working tree clean.
- Blockers: None.

## 2026-10-19
- Task: Land Tetherly v1.67 (ADR-060) 跨端点对点安全文本/便签即时交互通道、离线消息缓冲、消息历史治理与双向全闭环.
- Actions:
  1. Defined inner frame type `TYPE_TEXT_MESSAGE: u16 = 0x0202` and `TextMessage` model in `crates/tetherly-core/src/frame.rs` and `lib.rs` with `TEXT_MESSAGE_MAX = 32 * 1024` (32 KiB) single-frame transport ceiling; added roundtrip and bounds tests in `crates/tetherly-core/tests/decoders.rs`;
  2. Implemented `PeerMessage`, `MessageDirection` (`Inbound`, `Outbound`), bounded ring buffer `MessageEngine` (200 capacity), `send_message_to_peer` (with single-live-peer auto-targeting), `message_history` (multi-dimensional filter and atomic read marking), and `clear_messages` in `crates/tetherly-node/src/runtime.rs` and `lib.rs`; added inbound dispatch handler in `crates/tetherly-net/src/dispatch.rs`;
  3. Extended `UiEvent` in `crates/tetherly-node/src/events.rs` with `MessageReceived` and `MessageSent` zero-plaintext event variants (only metadata, strictly omitting plaintext body);
  4. Implemented loopback endpoints in `crates/tetherly-node/src/uihttp.rs`: `POST /api/message/send`, `GET /api/message/history`, `POST /api/message/query`, and `POST /api/message/clear` (with `/api/msg/*` aliases) with 127.0.0.1 physical binding and Host 403 Forbidden check;
  5. Implemented isomorphic `message`/`msg` CLI subcommands (`send`, `history`/`list`, `clear`, `watch`) and real-time SSE event formatting in both `bins/tetherly-cli/src/main.rs` and `crates/tetherly-node/src/bin/tetherly.rs`;
  6. Added "跨端即时消息与安全便签通道" control card in `ui/index.html` with target selector, unread filter, XSS-safe chat bubble streaming, quick multi-line send form, and SSE real-time sync;
  7. Added comprehensive E2E integration test in `tests/phase1.rs`: `node_direct_secure_messaging_history_governance_and_zero_plaintext_via_loopback_api`, asserting two-way message exchanges, unread marking, circular buffer clearing, zero-plaintext SSE verification, and loopback 403 isolation;
  8. Ran full workspace verification: `cargo fmt --all --check` (clean), `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo check --target aarch64-unknown-linux-gnu` (clean), `cargo test --workspace` (all 349 tests pass), `cargo deny check` (advisories ok, bans ok, licenses ok, sources ok);
  9. Updated SSOT `docs/DEVELOPMENT.md` to v1.67 (ADR-060), updated `.codex-memory/decisions.md` and `.codex-memory/project-state.md`.
- Results: All 8 tasks for v1.67 completed and verified 100% green.
- Next: Git commit and sync to Obsidian vault.
- Blockers: None.

## 2026-10-19
- Task: Land Tetherly v1.68 (ADR-061) 跨端发件箱生命周期治理、发送历史可观测性、传输状态追踪与安全审计清除全闭环.
- Actions:
  1. Defined strongly-typed send state enum `OutboxStatus` (`InFlight`, `Completed`, `Rejected`, `Failed`, `Cancelled`) and record model `OutboxEntry` in `crates/tetherly-node/src/runtime.rs` and `lib.rs`;
  2. Implemented bounded ring buffer engine `OutboxEngine` (200 capacity) in `runtime.rs` with $O(1)$ amortized push, reverse transfer ID status update `update_status`, multi-dimensional querying `query` (peer, status, limit), conditional clear `clear`, and entry resolution `get_by_id`;
  3. Wired outbox tracking into `offer_and_send_bytes`, `offer_and_send_file`, `set_transfer_phase`, `cancel_transfer`, and `SessionEvent::FileReject`; implemented one-key resend pipeline `Node::resend_outbox_file` verifying source disk files and recording `AuditCategory::File` audit traces;
  4. Extended `UiEvent` in `crates/tetherly-node/src/events.rs` with `OutboxUpdated` and `OutboxCleared` zero-plaintext variants, strictly enforcing privacy by broadcasting only transfer metadata without file payloads or private keys;
  5. Implemented loopback endpoints in `crates/tetherly-node/src/uihttp.rs`: `GET /api/file/outbox`, `POST /api/file/outbox/query`, `POST /api/file/outbox/clear`, and `POST /api/file/outbox/resend` (with `/api/outbox/*` aliases) guarded by 127.0.0.1 binding and Host 403 Forbidden checks;
  6. Implemented isomorphic `outbox`/`out` CLI subcommands (`list`, `clear`, `resend`, `watch`) and real-time SSE event formatting in both `bins/tetherly-cli/src/main.rs` and `crates/tetherly-node/src/bin/tetherly.rs`;
  7. Added "已发送文件与发件箱治理 (Outbox & Sent Files)" control card in `ui/index.html` with target/status filter dropdowns, file card streaming, one-key resend action, clear action, and SSE `outbox_updated`/`outbox_cleared` real-time sync;
  8. Added comprehensive E2E integration test in `tests/phase1.rs`: `node_outbox_file_lifecycle_query_clearing_resend_and_zero_plaintext_via_loopback_api`, validating outbox initialization, InFlight staging, Completed on accept, Rejected on reject, multi-dimensional query, resend pipeline, partial clear, full clear, SSE zero-plaintext, and Host 403 isolation;
  9. Ran full workspace verification: `cargo fmt --all --check` (clean), `cargo clippy --workspace --all-targets -- -D warnings` (clean), `cargo check --target aarch64-unknown-linux-gnu` (clean), `cargo test --workspace` (all 350 tests pass), `cargo deny check` (advisories ok, bans ok, licenses ok, sources ok);
  10. Updated SSOT `docs/DEVELOPMENT.md` to v1.68 (ADR-061), updated `.codex-memory/decisions.md`, `.codex-memory/project-state.md`, and synchronized to Obsidian vault (`Note/Project/tetherly/tetherly 开发文档.md`).
- Results: All 8 tasks for v1.68 completed and verified 100% green.
- Next: Git commit keeping working tree clean, followed by v1.69 feature planning and execution.
- Blockers: None.



## 2026-11-13
- Task: 接手 tetherly；确认 Windows→Mac 鼠标方向已在 v0.2.29 解决；按"你优化迭代本项目"做 v0.2.30 优化迭代并发版部署。
- Actions:
  1. 复核 v0.2.29（HEAD a6990ef）双端日志：`7b71531` zone-entered gate 修复 §101 的 7 秒弹回，用户真机确认 Windows 鼠标可接入 Mac；记录两处非阻塞噪音（回归后 ~1 ms attach_kick 预拨号；macOS `WARP_SETTLE_EVENTS=4` 产生 4 帧重复坐标的 `lone edge-clamp move`）。
  2. TDD 落地 A：core `local_move_delta` Remote 分支 `rem_changed` 门控（red：零位移事件推出 `Move { x: 500, y: 0 }` → green）；新增回归 `remote_seat_suppresses_move_frames_when_remote_position_unchanged`。
  3. 落地 C：`Shared.input_trace` 64 条环 + `trace_input` 埋点 10 类决策点 + `GET /api/input/trace` + 双 CLI `tetherly input trace`；新增 `input_trace_ring_is_bounded_and_records_rescue`、`test_input_trace_endpoint`。
  4. 明确拒绝 B（attach-kick 冷却），理由写入 §102.4 与 decisions。
  5. 版本 bump 0.2.30（Cargo.toml + `cargo update --workspace --offline` 同步 Cargo.lock 8 行）。
  6. 验证：fmt、clippy -D warnings 0 错误；core 313、node 205、loopback 11 / phase1 68 / phase2 8 / phase3 13 / phase5 65。
  7. 文档：DEVELOPMENT.md 顶部基线刷新（v0.2.7 → v0.2.30，标记 Windows→Mac 已解决）+ §102；`.codex-memory` 四文件同步；Obsidian 速读 / projects 子路由 / 生产变更速记同步并跑治理三检。
- Results: commit `f7337e2` + tag v0.2.30；release CI 9 资产全绿；`scripts/deploy.sh --target all --tag v0.2.30` EXIT=0——Mac 0.2.30（pid 97956，accessibility_trusted=true 无需 TCC reset）、home-win 0.2.30（302340→298224，SYSTEM，SessionId=2），受保护进程 100% 不动；双端 `/api/input/trace` 可用。ci 工作流 ubuntu/windows 红在既有 `m3_12`：ubuntu 为测试构造性竞态（已改等待环，测试改动）；windows 为 pump 开通即死（根因未定，见 DEVELOPMENT §102.7）。
- Next: 真机验收 v0.2.30（`lone edge-clamp move` 四连消失、tx_frames 增速下降、`tetherly input trace` 回放）；v0.2.31 候选：pump 开通/退出原因进决策环 → windows CI 取证 `m3_12`；后续候选优化见 todo "后续方向"。
- Blockers: None.

## 2026-11-13（第二轮）
- Task: 用户反馈"Windows 鼠标首次进入 Mac 很久进不来；在 Mac 里左键好像点不了"，深度看日志诊断并落地 v0.2.31。
- Actions:
  1. 复核 22:28 双端日志 + `/api/input/trace`：首次进入 8 s 是预臂 accept 关闸（屏幕上却是本机用户光标）→ 顶边全被 `dwell gated`；进入后 0.9–3.3 s 弹回是顶边停车 + 20 单位滞回 + 残余推力累满 12 px。左键问题日志零 Button 记录（埋点盲区）；代码复核发现 macOS `move_abs` 恒 `MouseMoved`，按住期间不发 Dragged。
  2. TDD 落地 A（闸门锚定 Enter：`note_controller_entered/left`、预臂 accept 开闸）、B（core `entry_return_grace` + `EdgeCrossingConfig.entry_return_grace_ms` 默认 1000 + `topology_update_config` 第 11 参 + API body/query）、C（`INJECTED_BUTTONS_DOWN` + `injected_move_kind` + `move_abs` Dragged 类型）、D（`/api/input` 六计数字段 + trace 六新 kind + 双 pump `pump_exit`/`channel_open`）。
  3. 选项 iii（Mac 再预臂延迟）按 §102.4 不做。
  4. 版本 bump 0.2.31（Cargo.toml + `cargo update --workspace --offline`）。
  5. 验证：fmt、clippy -D warnings 0 错误；core 315、node 211；loopback 11 / phase2 8 / phase3 13 / phase5 65；phase1 68 并行跑 3 次：2 次各有 1 个无关用例负载抖动（input_tunnel release 即时断言、quick_actions 落盘 5 s），单跑各 3 次全过，1 次全绿。
  6. 文档：DEVELOPMENT.md 顶部基线 → v0.2.31 + §103；`.codex-memory` 四文件同步。
- Results: commit `c41af3d` + tag v0.2.31，release run 43327674466 五作业全绿；`scripts/deploy.sh --target all --tag v0.2.31` EXIT=0（Mac pid 97956→51476 accessibility_trusted=true；home-win 298224→310648 SYSTEM/SessionId=2；受保护进程 100% 不动）；双端 `/api/input` 新字段全 0 待漫游。新埋点首批取证：attach 双拨号（gen=N 被 gen=N+1 顶掉 → 第一条 pump `outbound slot dropped`），记为 `m3_12` 候选实证（§103.8）。原计划：待 commit + tag v0.2.31 → release CI → `scripts/deploy.sh --target all --tag v0.2.31` → 双端 health 0.2.31 与 `/api/input` 新字段 → 用户真机点击对账（`tx_buttons`/`rx_buttons`/`dropped`、`push_button`/`apply_button`）。
- Next: 部署后真机验收 §103.8 四项；若 Windows 侧 `tx_buttons` 不增则查 LL hook Button 分发；`m3_12` windows 红用 `pump_exit` 取证。
- Blockers: None.

## 2026-11-13（第三轮）
- Task: 用户 v0.2.31 真机 3 分钟测试反馈三症状（Windows 屏中央留光标；Mac 里不灵敏/延迟/丢数据；底边第一次进入有点卡），用户授权"全面负责开发"，落地 v0.2.32。
- Actions:
  1. 诊断：Windows relay 模式从未隐藏系统光标；core Remote 位移按本机尺寸归一化（1920×1080 → 1470×956 各向异性 0.77/0.89 + 1000 单位量化）；1.0 s 硬宽限到期即回归；relay 分支缺 bogus 过滤；§103.8 attach 双拨号根因是 `input_out` 清空晚于 `send_to`。
  2. TDD 落地 A（`platform.rs::system_cursor` + `HookControl::set_keyboard` 接线 + `start_input_hooks` force restore）、B（core `set_remote_screen`/`set_remote_speed_percent` + accept/release 接线 + `EdgeCrossingConfig.remote_speed_percent` + `topology_update_config` 第 12 参 + API）、C（`required_return_push_px` 斜坡 + 默认宽限 3000）、D（relay bogus 过滤 + attach drop 提前）。
  3. 版本 bump 0.2.32 + `cargo update --workspace --offline`。
  4. 验证：fmt；clippy -D warnings 0 错误；core 321、node 214；loopback 11 / phase1 68 / phase2 8 / phase3 13 / phase5 65 一次全绿。Windows 交叉编译本机失败于 openh264-sys2 build script（缺 Windows C 头），Win32 签名对照 windows-sys 0.59.0 源码核对，编译门禁交 ci.yml windows 作业。
  5. 文档：DEVELOPMENT.md 顶部基线 → v0.2.32 + §104；`.codex-memory` 四文件同步。
- Results: `b133f6b` 功能提交 + `afe2279` ci dead_code 补修（v0.2.31 遗留，ubuntu/windows 连红根因）；ci.yml 三平台全绿（m3_12 windows 连红随双拨号修复消失；ubuntu m3_2 偶发一次超时，rerun 绿）；tag v0.2.32 → release 43426215702 成功 → deploy.sh exit 0；Mac/Windows health 0.2.32，Windows SYSTEM sid=2 + `[cursor] restored (force=true)`，受保护进程不动，rtt 61–64 ms。
- Next: 用户真机 3 分钟实测 §104.7 (a)–(d)；手感不对调 `POST /api/topology/config {"remote_speed_percent":N}`；m3_2 若再超时查 `open_input` 重试循环的无超时 await。
- Blockers: None.

## 2026-11-13（第四轮）
- Task: 用户 v0.2.32 真机反馈：(a) Windows 中央光标闪一下后消失；(b) Mac 跟手好多了；(c) 底边第一次进入仍很卡"你看下日志"；(d) channel_open / pump_exit 口径"你 review 下"。
- Actions:
  1. 读 Windows `tetherly_system.log` + trace：(a) `park warp` 13:05:10.998178 早于 `system_cursor: hidden` 13:05:11.004023；回归 disarm 恢复光标早于 warp 回家。(c) 13:04:57.95 controller left → 13:05:07.05 `distance gate not armed; requiring long dwell accum_px=2` → 13:05:10.70 pinned heartbeat 才完成；13:05:42 Mac watcher 自动重连抢 attach 致第二次同型卡顿。(d) 每次 attach 一条 `channel_open`，`slot dropped` 为回归时正常拆除。
  2. TDD 落地 `reverse_local_owner` 快路径 + `enter/leave_remote_seat_capture` 平台顺序 + `HookControl::hide_cursor`；4 个新单测；phase3 m3_11 距离门段落按新语义删去。
  3. 版本 bump 0.2.33 + `cargo update --workspace --offline`；提交 `31e42ae`。
  4. 文档：DEVELOPMENT.md 基线 → v0.2.33、§104.7 (d) 更正、新增 §105；`.codex-memory` 四文件同步。
- Results: 本机 fmt / clippy -D warnings 0 错误；core 321、node 218、集成全绿。
- Deploy: ci 43446075823 绿 → tag v0.2.33 → release 43448044509 绿 → deploy EXIT=0；Mac pid 10880 / Windows pid 274908 SYSTEM sid=2；受保护进程不动。
- Next: 用户真机验收 §105.7。
- Blockers: None.

## 2026-11-14 - v0.2.34（§106）
- Context: 用户 v0.2.33 真机反馈：首次进 Mac 仍卡；Mac 遗留光标。
- Work: 日志定位 16:01:51.142 钉边心跳误抢控制器 → Windows 降级 → 16:03:07–16:03:14 慢反向；新增 `park_target_cursor_after_leave` + macOS idle hide/reveal + `reverse_heartbeat` trace；TDD 两个新测试先红后绿。
- Results: fmt / clippy 0 错误；core 321、node 220、集成全绿。commit 983c37c。
- Deploy: ci 43461276452 绿 → tag v0.2.34 → release 43463401787 绿 → deploy EXIT=0；Mac pid 61015 / Windows pid 309292 SYSTEM sid=2；受保护进程不动。
- Next: 用户按 §106.7 真机验收。

## 2026-11-14 - v0.2.35（§107）
- Context: 用户：第一次 Windows 鼠标进入 Mac 仍卡很久。
- Work: 日志对比定位 2.9 s 同步停顿为进程级饥饿；只读探针确认 BelowNormal + WS 6.5 MB；新增 `raise_daemon_responsiveness` 及测试。
- Results: fmt / clippy 0 错误；cargo test --workspace EXIT=0（838 passed, 0 failed）。
- Deploy: ci 43469993884 绿 → tag v0.2.35 → release 43471801241 绿 → deploy EXIT=0；Windows pid 307384 AboveNormal / MinWS 64 MB / SYSTEM sid=2；`daemon responsiveness` 四项全 true。
- Next: 用户按 §107.7 真机验收。

## 2026-11-14 - v0.2.36（§108）
- Context: 用户：Mac 鼠标在 Windows 时 Windows 鼠标进不来 Mac。
- Work: 日志定位控制端在屏路径无声失败；复现测试 RED；contiguous 分支补记锚点 GREEN。
- Results: fmt / clippy 0 错误；cargo test --workspace 全绿（node 224、core 321）。
- Deploy: ci 43479518747 绿 → tag v0.2.36 → release 43481580990 绿 → deploy EXIT=0；Mac pid 23482；Windows pid 267304 SYSTEM；受保护进程不动。
- Next: 用户按 §108.7 真机验收；弹回观察见 §108.8。

## 2026-11-14 - v0.2.37（§109）
- Context: 用户：Mac 在控制 Windows 时被 Windows 接管，之后 Windows 鼠标回 Mac 会卡很久。
- Work: Mac 日志定位降级后 8 轮进入→弹回；根因为护罩空白 NSCursor 未恢复；新增 `set_arrow_cursor_now` / `restore_arrow_cursor` 并在三处调用；两条测试 RED→GREEN；phase3 m3_9 第 3 步改轮询（macOS CI 43486556104 竞态红）。
- Results: fmt / clippy 0 错误；cargo test --workspace 全绿（node 226）；ci 43488480812（3a2ac2f）三平台 + deny + android 全绿。
- Deploy: tag v0.2.37 → release 43490201522 绿 → deploy EXIT=0；Mac pid 57798；Windows pid 308400 SYSTEM；受保护进程不动。
- Next: 用户按 §109.7 真机验收。

## 2026-11-14 - v0.2.38（§110）
- Context: 用户：v0.2.37 后“老问题”仍在，Mac trace 显示 4 轮进入后 0.3–0.6 s 弹回。
- Work: Windows 日志 19:27 定位整段位移计入回程推力 + 48 px 入口门槛；只算越边部分、入口门槛 10×、ReturnDiag 日志；3 条测试 RED→GREEN。
- Results: fmt / clippy 0 错误；cargo test --workspace 全绿（core 324）。
- Deploy: ci 43496484032 绿 → tag v0.2.38 → release 43498423490 绿 → deploy EXIT=0；Mac pid 16286；Windows pid 288460 SYSTEM；受保护进程不动。
- Next: 用户按 §110.7 真机验收。

## 2026-11-14 v0.2.39（诊断版）
- Context: 用户：Mac 光标先进 Windows 后，Windows 鼠标划向 Mac 要 10–20 s 才能停住。
- Work: v0.2.38 日志显示 10 次进入均在 0.4–1.2 s 内被 push≈required 送回；原因未定，补 ReturnDiag 笔划统计、relay 状态和 Mac 光标可见性证据（§111）。
- Deploy: ci 43505914478 → tag v0.2.39 → release 43507848391 → deploy EXIT=0；Mac pid 89356；Windows pid 308452 SYSTEM；受保护进程不动。
- Next: 用户复现后按 §111.5 判读。

## 2026-11-14 v0.2.40（§112）
- Context: v0.2.39 复现。Mac 每次 Enter 都是 `cursor_reveal visible=false->false idle_hidden=true`，用户看不到指针，来回划导致弹回。
- Work:
  - 新增 `scripts/verify_crossing.py` 真机验收脚本。
  - 按用户要求“深度借鉴 deskflow”，重写 macOS 光标显隐：
    - 单一隐藏状态 `DeskflowCursorOwners`，两个 owner：Capture 和 Idle；
    - `deskflow_show_cursor/hide_cursor` 对齐 OSXScreen：主屏只调一次，然后 `associate(true)`；
    - 撤掉依赖 `CGCursorIsVisible` 的补 show。
- Tests: owner 状态机 2 项 RED→GREEN；fmt / clippy / workspace 全绿。
- Deploy:
  - ci 43518069653（ubuntu m3_2 计时 flake，重跑后绿）→ tag v0.2.40 → release 43520857621 → Mac pid 74782。
  - Windows 旧 v0.2.39 在 22:44:48Z 一次 10053 断开加 auto-reconnect 之后挂死（HTTP 无响应、日志停止），`/api/daemon/restart` 因此卡住。处置：dump（`E:\code\tetherly\data\dumps\`）→ stop 308452 → 覆盖 CI 产物 → TetherlySystemLauncher → pid 310880，SYSTEM，0.2.40 healthy。
- Next:
  - 用户运行 verify_crossing.py；
  - 排查 Windows 挂死：10053 关闭与 reconnect 并发，疑似锁顺序问题；
  - deploy.sh 在 restart 前先探 health。

## 2026-11-14 v0.2.41
- 分析剩余切换延迟：网络单向约 5 ms，可排除。23:08:15 → 23:08:22 约 7 s 的等待来自 Target reverse 栅栏（600 ms 未 armed dwell / pinned heartbeat，加 3 s 冷却）。
- 修复：`REVERSE_HAND_OWNS_PX=64` 的本地手晋升。新增测试 local_hand_sweep_takes_ownership_and_fast_reverses、local_hand_bump_does_not_take_ownership。
- 下一步：
  - Windows 守护进程挂死根因（10053 断开与 reconnect 同毫秒）；
  - deploy.sh 重启前加健康探测；
  - 修复 m3_2 flake；
  - 接管后 Windows 引擎越界还需约 185 ms，等待下一个 hook 事件。

## 2026-11-14 v0.2.42
- 「偶尔卡顿」：00:18:49 Mac push enter → Windows apply enter 晚了约 7.5 s。这是重启后第一次 enter（空闲约 19 min）；enter 与同批 entry move 相隔 4.7 s，日志时间戳乱序。
- 只读采集主机状态：提交 31 GB / 物理 16 GB，C: idle 7.7%，exe 在 E: HDD，WS 7.8 MB，flags=0x9 → 硬下限无效。
- 修复：`lock_daemon_image`（VirtualQuery 遍历映像 + 纯函数 `image_lock_ranges` / `image_lock_fits_floor` + VirtualLock）。启动日志新增 `image_locked_bytes`。
- 测试：3 个跨平台单测加 1 个 Windows 实测。
- 部署前内存清理：新增 `scripts/win_memory_cleanup.ps1` 并接入 deploy.sh（预检之后、staging 之前）。只清理卡死的 sshd 会话加 EmptyWorkingSet，受保护进程永不触碰，exit 2 中止部署。首次执行可用内存 1.1 GB → 3.5 GB，结束 9 个僵尸会话进程和 1 个卡死的 pytest。
- Windows CI clippy 修复：`then_some`，函数指针先转 `as *const ()`（0115933）。

## 2026-11-14 v0.2.43
- v0.2.42 部署验证：`image_locked_bytes=26304512`，WS 37.4 MB，部署内清理生效，受保护进程完好。
- 新问题：Windows 热重启后 Mac 3m49s 才重连。三个根因：
  1. `accept_one` 把对端临时源端口写进 routes（Windows 路由 = `192.168.1.7:56857`）；
  2. 拨号无 connect 超时，overlay 黑洞 SYN，macOS 挂 75 s；
  3. 退避计数在 peer 在线时不复位，会话断开也不唤醒循环。
- 修复：`inbound_route_update` / `reconnect_candidates`（回退 `ip:CONTROL_PORT`，坏路由自愈）/ `connect_with_timeout(3s)` / `ReconnectBackoff` + `reconnect_wake` Notify。共 8 个单测。
- 附带发现：旧 daemon 同毫秒做了两次 LAN 升级，产生重复会话（未修）。
- v0.2.43 已部署并验证：Windows 热重启 02:44:04，Mac 在 1 s 内经 overlay 重连，随后升级到 LAN；input channel 自动重连成功。Windows routes.json 已自愈为 `10.144.144.100:45717`（原为临时端口 56857）。部署内清理：可用内存 3.6 GB → 4.3 GB，裁剪 43 个进程；受保护进程 3/3 完好；image_locked_bytes=26316800。

## 2026-11-14 v0.2.44 性能/内存优化
- 只读基线：两端 daemon 都已很省（Mac footprint 25.5 MB，Windows 私有 11.5 MB，空闲 CPU <1%）；瓶颈在 I/O：日志无界、写了两三份、带 ANSI；剪贴板每 800 ms 全量读一次再算 SHA-256。
- 修复：新模块 `logsink`（16 MiB 滚动为 `.1`，日志跟随 `--data-dir`，仅终端才镜像 stdout/ANSI）；两条逐事件 INFO 降为 debug；`Clip::change_token`（NSPasteboard changeCount / GetClipboardSequenceNumber）+ `clip_read_needed` 跳过未变化的轮询。新增 10 个单测，全量 870 通过。
- 已部署并验证：Windows 日志改为写用户目录的 `tetherly.log`；Mac 旧日志已滚动；Mac footprint 16 MB，Windows 私有 6.6 MB；受保护进程 3/3 完好。

## 2026-11-14 v0.2.45–v0.2.46 LAN 提升 single-flight / input start 竞态 / 提升握手超时
- 删除孤儿日志 `C:\Windows\System32\config\systemprofile\AppData\Local\tetherly\tetherly.log`（3.7 MB）。
- v0.2.45：
  - `InflightSet` 让每个 peer 同时只做一次 LAN 提升；id 较小的一端先提升，较大的一端退避 1.5 s；探测循环每轮检查 `peer_on_overlay`。
  - `open_input_settled` 修复 phase5 偶发失败：CAS 失败时等待在途拨号，并上报真实的 `channel_open`。
- 上线时观察到：Mac 新、Windows 旧的混合版本窗口内，提升握手半开，卡满 20 s，约 19 s 没有可用链路。
- v0.2.46：`dial_lan_promotion` 握手超时改为 3 s。
- 已部署并验证：两次重连都是单次提升、单条 LAN；path overlay 从 42 条降到 3 条；受保护进程 3/3 完好。DEVELOPMENT §117。

## 2026-11-14 v0.2.47 Windows→Mac 光标横向卡顿（坐标量化）
- 只读排除：网络（LAN 2–4 ms）、overlay、relay echo 门控、时间批处理、Mac tap/apply 开销。
- 根因：1000 单位网格在 1470 pt 宽 Mac 上每单位 1.47 pt；800 Hz × 1.7 px 的事件约 1/3 被量化为 no-op，随后跳变（RED 400→272）。
- 修复：SCREEN_UNITS=10000、FINE 帧 0x0408、双解码、`legacy_compat_frame`、cap `inputunits=10000`，pump 按 `peer_supports_fine_input` 降级；迟滞带 = SCREEN_UNITS/50。
- 已部署：两端 0.2.47、互见 cap、path lan、受保护进程 3/3。DEVELOPMENT §118。

## 2026-11-14 v0.2.48 Windows 回边卡顿
- 证据：Windows 回边时 release → restore（SPI_SETCURSORS 8–14 ms），完成后才 return_warp，箭头在中心停靠点闪现。
- 修复：先传送后释放；`rescue_seat_to_local` 缩小 engine 锁作用域，消除 reconcile 路径的同线程重入死锁。
- 新增 2 个 RED→GREEN 测试；CI 与 release 全绿；两端 0.2.48、lan、受保护进程 3/3。DEVELOPMENT §119。

## 2026-11-15 v0.2.49 全面 review + 日志系统
- 三路 review（输入/网络/HTTP）结果逐条核实；5 个高危且改动面小的问题已 TDD 修复（每项都先 RED）。
- 日志：运行时可切换级别并定时自动回滚、tail/grep/level 过滤、panic 落盘、启动横幅；Windows 排障不再需要 PowerShell 读文件。
- 路线图写入 DEVELOPMENT §120：P0 API token 鉴权 + data_dir ACL；P0 会话 generation；P1 inbox 可执行文件、LAN 提升身份校验、rekey 取消安全。

## 2026-11-15 v0.2.51
- P0-2 zombie sessions: RED tests session_channel_closes_when_registration_is_dropped_despite_keepalive / replaced_session_exit_does_not_release_its_successor → GREEN. register_session/release_session/spawn_session_keepalive extracted in lifecycle.rs. CI 43624055681, release 43625021011, deployed both; zombie overlay conn gone.

## 2026-11-15 v0.2.52 API token + ACL
- P0-1：特权 UI 路由必须带每次启动生成的 token（`apitoken.rs`）。RED→GREEN 测试：`privileged_routes_require_the_api_token`、`refused_request_with_late_body_gets_clean_401`（拒绝前不读掉请求体就会 RST）、`port_registered_token_wins_over_the_default`（多 node 测试按端口取 token）。
- CI 43628983820 与 release 43630108316 均为绿；两端 0.2.52；无 token 返回 401、带 token 返回 200；Windows token 文件 DACL 正确。
- home-win ACL：release 目录和 bat 去掉 AU:M（备份在 `E:\code\acl-backup\`），收紧后带 token 的重部署成功，受保护进程 3/3。DEVELOPMENT §122。

## 2026-11-15 v0.2.53 P1 收尾
- P1-a inbox 可执行文件降级：`inbox_open_demotes_executables_to_the_folder`。P1-b LAN 提升身份与黑名单校验：`lan_promotion_rejects_identity_mismatch_and_blocked_peer`。P1-c 写入取消安全（outbox）：`send_cancelled_mid_write_keeps_the_stream_and_rekey_intact`。三项都是先看到 RED 再修到 GREEN。
- CI 43633483432 与 release 43634582339 均为绿；两端 0.2.53，path lan；Mac 降级实测 demoted:true；受保护进程 3/3。DEVELOPMENT §123。

## 2026-11-15 v0.2.54 深度 review + P2 清零
- R1 inbox 白名单（`inbox_open_launches_only_passive_types`）、R2 来源标记（`received_file_gets_the_quarantine_attribute`）、R3 Windows 以控制台用户身份打开（CreateProcessAsUserW）。
- 侧通道黑名单（`input_and_screen_channels_refuse_blocked_or_unexpected_peers`）；`dial_lan_promotion` 握手前先查黑名单。
- P2 十项，其中新测试 `input_batch_is_atomic_under_queue_pressure`、`stalled_request_is_answered_408_and_closed`、`dial_peer_by_id_survives_a_slow_path`、`screen_codec_follows_the_viewer_not_the_last_caps_update` 都是先看到 RED 再修到 GREEN。
- phase1 偶发 Timeout：负载 50+ 时出现；stash 基线与改动后各跑 3 次均 68/68，判定为环境抖动。
- CI 43641398837 的 android 红：cfg 属性被新插入的函数占用。修复后 CI 43642452118 绿；release 43643501170 绿。
- 部署结果：两端 0.2.54，path lan；Windows pid 293340 SYSTEM；受保护进程 3/3；无新增 WARN。DEVELOPMENT §124。

## 2026-11-15 v0.2.55 Windows 手回切卡顿（§125）

- 用户反馈：Mac 鼠标进入 Windows 后，用 Windows 鼠标回切仍然卡。
- 日志：两次慢回合都没有 fast push，最后靠 pinned 心跳触发（约 4 s）；回合 B 在 accept 之后，engine 穿越又多等了 1.3 s。
- 根因：Windows LL 钩子非 relay 分支对相邻两次 `pt` 提议取差。光标在 1079 被 clamp 时得到 +3/−1/+2 的锯齿，负值清空 dwell，单个 tick 达不到 6 px。
- 修复：`hook_move_state::advance(x, y, cur)`，增量 = `pt - GetCursorPos()`。RED 测试 `clamped_edge_push_deltas_measure_from_actual_cursor`。
- CI 43693145564 绿；release 43694027234 绿。
- 部署结果：两端 0.2.55，path lan；Windows pid 269800 SYSTEM；受保护进程 3/3；无新增 WARN。

## 2026-11-15 M2a 收尾 + m3_11 flake（§129）
- 接手会话。v1 路由对齐 §4.2（pin/rotate、input/release）+ m3_11 push 流改为循环内持续驱动（负载下超 TOUCH_GAP_MS 导致 dwell 重置的 flake 根治）。commit 53c441b。
- phase1 8 例 dial Timeout 判定与 §124 一致（默认并发下环境抖动，-j2 全绿 973 passed）。
- 文档：DEVELOPMENT.md 基线块更新至 v0.2.58 + §129；vault projects 路由同步。
- 下一步：推 main → CI → tag v0.2.58 → deploy；然后 M2b Web SPA。

## 2026-11-15 v0.2.58 发布（§129）
- CI 43718956135 全绿 → tag v0.2.58 → release 43719992315 绿 → deploy EXIT=0：Mac pid 7276；Windows hot-restart 308536→310860 SYSTEM；受保护进程 3/3。
- v1 只读探测：health 开放 / diagnostics legacy_routes 计数生效 / devices 返回对端 / pin/rotate 新路由 201。
- 下一步 M2b：client/ Svelte 5 SPA。

## 2026-11-15 M2b：Web SPA 脚手架 + daemon /next/ 托管
- client/（Svelte 5 + Vite 6 + TS）：Transport seam（HttpTransport，手写 SSE 解析、last_event_id 重放、退避、#token= → sessionStorage）、纯 reducer devices/connection store（fold(snapshot, events) property 测试）、App shell + 概览/设备两页。vitest 4/4、svelte-check 0 错、build 46.5 kB。
- Rust 侧 `uihttp/next.rs`：GET /next(/) 入口、/next/assets/* 固定名资产、未知 /next/* 回退入口（SPA 深链）；GET-only，旧控制台留 /，/api/* 冻结不动。4 单测 + fmt + clippy（原生 & windows-gnu）绿；lib 337 全绿。
- client/dist 进 git（daemon 编译期嵌入，CI 无 node 步骤）；node_modules 进 .gitignore；vite 产物固定名（assets/app.js、assets/index.css）。
- 临时实例（45816）冒烟 + 真实浏览器验证：页面渲染、hash 剥离、token 入 sessionStorage、SSE linkState=live、页面切换正常、旧控制台完好。生产 daemon 45716 全程未动。
- 提交 89b6498（Rust 托管）+ 36311ea（client 脚手架）→ push main，CI 43725065383。
- 下一步：system/pairing/input 等 store + api client 层 + 剩余 5 页；dashboard 链接指向 /next/（含 token fragment）留到 M4 切换前。

## 2026-11-15 M2b 续：section stores / Api 层 / 诊断与 PIN（CI 4/4 绿）
- e4e4821：system/pairing/input 纯 reducer store（6 新测试，10/10）；Overview 渲染真数据（本机块 + 输入归属）。
- f3ffef0：client.ts 类型化 Api 层覆盖当前全部 v1 端点；设备页动作（连接/屏蔽/忘记/解除屏蔽）。
- fec98f3：诊断页（seq、legacy 路由命中表、日志尾、log-level 热更）+ 概览配对 PIN 卡片（显示/轮换/撤销）。
- CI 43725065383 / 43725860017 / 43726288460 / 43726693094 全绿。
- 实测 invalidated 链路：config 写 → settings topic invalidated 帧到达 SSE 客户端（hub.project → coalescer → publish 验证通过）。
- 契约现状：snapshot 仅 4 section；layout/transfers/inbox/activity/settings 只发 invalidated、无 GET 端点 —— 对应页面等 Rust 侧 M2c 扩展后接数据。

## 2026-11-15 M2b 续 2：MockTransport 流程测试 + dashboard --next
- f287a2d：MockTransport（脚本回放 + 请求应答器）；App 可注入 transport；4 条流程测试（14/14）。vitest 需 svelte plugin + resolve.conditions:["browser"] + globals:true（组件挂载 + testing-library 自动清理）。
- 7d56b4e：apitoken::next_url + tetherly dashboard --next（fragment 契约同 dashboard_url；默认/tray 不动，M4 切换）。9 apitoken 测试 + clippy 双 target 绿。CLI 实测两 URL 输出正确。
- 139e00d：§130 补记。生产 daemon /next/ 404 属预期（未发版），随下版部署生效。
- 下一步：M2c v1 端点扩展（transfers/layout/settings GET，补四张页面）或直接排 M3 Tauri；等 CI。

## 2026-11-16 M2c：v1 端点扩展（transfers/inbox/layout/settings 读链路）
- 契约层：16 新类型 + 4 新事件（transfers.changed/inbox.changed/layout.changed/settings.changed），形状照真实数据源（inbound|outbound、transferring/verifying/completed/failed、DisplayRect+scale_factor、CrossingConfig 九字段）；Event/EventEnvelope 降 PartialEq（f32 失 Eq）。TS 绑定重生成（u64 计数补 ts(type="number") 避免 bigint）。
- model.rs 四组装函数；hub resolve() 四主题发布全量状态（Activity/Clipboard 仍 invalidated）；v1.rs 四 GET 端点 + allowed() 表。
- client：四个 reducer store + Transfers/Inbox/Layout/Settings 四页，App 导航加 传输/布局；sections.test.ts 16/16、svelte-check 0 错、dist 重建。
- 测试契约修正：SSE replay 测试改期望 transfers.changed（新契约直证）；hub 新投影测试 state_topics_project_full_state_and_others_stay_invalidated。
- 验证：fmt 绿、clippy 双 target 0 错、cargo test --workspace 全绿、vitest 16/16。§131 已记。
- 留下轮：PATCH /settings、传输/inbox 动作端点、activity 页数据、Playwright e2e。

## 2026-11-16 M2c 收尾：CI flake 根因修复（67239ed）
- M2c 提交 0553826 首轮 CI：Windows phase1 `clip_sync_and_send_via_live_node_api_reaches_peers` 失败（sent_peers 0 != 1）；重跑 failed job 全绿 → flake 坐实。
- 根因链条：host.clip().set_text() 只写 OS 剪贴板不动 hubs.clip.last_hash；800ms 自动同步 poller（spawn_clipboard_auto_sync → poll_clipboard_auto_sync）发现 change_token 变化后抢先广播（prepare_local 更新 last_hash）；显式 POST /api/clip/sync（send_clipboard_to → prepare_local）因 hash 未变返回 Ok(0)（echo 抑制）→ ==1 断言 flake。
- 修复：测试开头把 host+peer 剪贴板策略切 Manual（can_send()=false，poller 直接退出），显式 sync 独占广播；peer 收到内容断言不变（权威判据）。本地 1.26s 绿，fmt/clippy 绿。
- CI 43740506835（67239ed）盯守中。

## 2026-11-16 M2d：v1 写端点 + SPA 动作（f9fcf53）
- 契约：SettingsPatch（optional + deny_unknown_fields + is_empty()）、InboxOpenRequest{name}；契约测试（空 patch ok、拼错字段拒、open 载荷）；TS 绑定重生成。
- v1 写路由：POST /transfers/{id}/cancel、DELETE /transfers、DELETE|POST /inbox/{name}(/open)、PATCH /settings（update_config + set_auto_accept_policy）；分派链 device→transfer→inbox_open→inbox_item；allowed() 补全。
- core_error 补 404 映射：UnknownTransfer、File("not found")（照 legacy api_files.rs 判定）——测试先红（500≠404）后绿。
- client：Api 9 个写方法；Transfers 取消/清历史、Inbox 打开（is_safe）/删除/清空、Settings 名称+auto_accept 可写（事件回灌，本地草稿）；App 导航加 收件箱/设置，三页传 {api}；svelte-check 0 错、vitest 16/16、build 绿。
- 验证：fmt、clippy 双 target 0 错、workspace 全绿（phase1 两例 401 竞态 flake 复跑绿，§124 同族）；§132 已记。CI 盯守中。
- 留下轮：activity 页数据、Playwright e2e（M2e）；M3 Tauri。

## 2026-11-16 M2e：activity 喂给 + Playwright e2e（92a4ba4）
- 契约：Activity 四列表（candidates/replies/messages/handoffs）+ 子类型 + MessageDirection/HandoffStatus；Event::ActivityChanged；测试表 10 例；绑定重生成。
- model::activity 四数据源组装；hub Topic::Activity 升级全量状态；v1 GET /activity；门面测试补 200。
- 注意：一个事件（ResourceHandoffReceived）同时 mark Inbox+Activity，测试按 frame name 过滤取帧，勿取首帧。
- client：activity store + Activity.svelte 四卡片 + App 导航 动态；vitest 18/18。
- **HttpTransport 解包 bug（Playwright 揪出）**：wire SSE data 是 EventEnvelope{seq,type,data}，原样上抛 → applySnapshot 收到 {data:…} → devices not iterable → 断流重连循环。jsdom MockTransport 喂裸 payload 暴露不了；parseFrame 统一解包 parsed.data；void err 改 console.error 留痕（吞错误曾致静默 offline）。
- Playwright e2e：config + e2e/server.mjs（dist 静态服 + SSE mock）+ 4 条流程；系统 Chrome（channel:"chrome"）绕 CDN 下载失败；4/4 绿。
- 验证：fmt/clippy 双 target 0 错、api 13 + node 338 绿、workspace 37 段 ok（phase1 本机 flake 跨测试，§124 同族，复跑绿）。§133 已记。CI 盯守中。
- 留下轮：activity 动作（dismiss/reply/mark-read）、M3 Tauri、M4 默认切 /next/。

## 2026-11-16 M3 首轮：Tauri 2 壳切片 + EventEnvelope wire 修正（552dfce + d302351）
- **wire 修正（552dfce）**：EventEnvelope 弃 flatten，改手写 Serialize/Deserialize——wire 唯一形状 {seq,type,data:<裸负载>}。教训：M2e 三方（parseFrame/e2e mock/stores）一致地错，mock 是消费者拷贝照不出生产者契约漂移；SSOT 必须是 contract 测试直证的 wire。
- **壳（d302351）**：client/src-tauri workspace member `tetherly-shell`（tauri 2.9/build 2.6，无 HTTP 客户端 crate，代理手写 HTTP/1.1 照仓库惯例）；invoke('api') 路径白名单 /api/v1、token 只在 Rust State；单 SSE 连接退避续传 emit tetherly://event|link；token 发现镜像 daemon（~/.local/share/tetherly/api.token，**§211 的 ~/Library/Application Support 是错的**）；TauriTransport TS 走 window.__TAURI__ 零 npm 依赖，App isTauri() 自动切换。
- 测试：Rust 11（含 mock daemon 逐字往返断言）、vitest 6（stub 桥）；env 竞态用共享 ENV_LOCK 串行（§124 壳侧预防）。CI Linux 加 webkit2gtk 系统库。
- generate_context! 需 frontendDist 存在——client/dist 本就进 git（daemon include_str! 先例），无 CI 顺序问题。图标：Node zlib 手写 RGBA PNG + sips + 自拼 ICO/ICNS（tauri-build 强制 RGBA）。
- 验证：fmt/clippy workspace 0 错、40 段 ok（phase1 两例 m3_* flake 复跑绿，§124 同族）；壳对活 daemon 冒烟 4 秒干净。§134 已记。CI 盯守中。
- 留下轮：托盘/托盘面板、自启、通知、深链接、.dmg/.msi、daemon 重启恢复与 <120MB 实测、Windows client.token 签发。

## 2026-11-16 M3 二轮：托盘 + 通知 + daemon 重启恢复
- **语义修正（最重要）**：`channel_open==false` 不是暂停——它是空闲态（无控制通道附着）；真共享开关是 `settings.general.advertise`（停 mdns 广播）。托盘 paused 折叠改吃 settings.changed。remote > paused 优先级。
- 托盘：TrayFold 纯折叠（snapshot/input.changed/settings.changed）→ local/remote/paused/offline 四态；32×32 RGBA 字形 const fn 内嵌（const 里没有 sqrt，用平方距离）；tint 重着色条带。**tauri 依赖必须显式 features = ["tray-icon","image-png"]**（空 features 没有托盘能力，tauri-build 不自动开）；`Image::new_owned` 是 const fn 返回值非 Result；Error 无 AssetNotEmbedded。
- 菜单桥接全走 app event（SSE→tetherly://event→托盘折叠；菜单→tetherly://toggle-share→Rust PATCH settings）。on_event 返回折叠前 was_remote——先 apply 再取旧值会吞接管通知。
- 通知：notice_for 纯函数（接管/回还/未读活动），plugin 2.4 桌面 API 是 `.notification().builder().title().body().show()`。
- daemon 重启恢复：connect_with(frame_sink) 抽出 AppHandle 依赖；集成测试两代 daemon 同端口，断言 v2 收到 `GET /api/v1/events?last_event_id=7`、壳折叠 v2 snapshot。
- cargo deny：CI 口径是不带 --workspace（只查 daemon 图）；带 --workspace 会拉进 shell 的 GTK 链（cssparser MPL-2.0、proc-macro-error unmaintained）7 crate 拒——HEAD 同样，非本轮引入。
- proxy mock daemon 测试改循环读满（\r\n\r\n + Content-Length），单次 read 在 head+body 分段时是竞态。
- 验证：fmt / host+gnu clippy 0 错 / workspace **1006** 全绿 / deny ok（CI 口径）。并行 cargo 抢 target 锁假失败再次复现——门必须串行。
- 留下轮：托盘面板 popover、自启、深链接、.dmg/.msi、<120MB 实测、通知点击聚焦、Windows client.token。

## 2026-11-16 M3 三轮：托盘面板 + 自启 + 深链接 + 安装包 + 内存实测
- **托盘面板**：左键 toggle_panel 懒创建 360×480 无边框窗，URL 用 `WebviewUrl::App("#tray")`（实测 url.join 产 tauri://localhost#tray，SPA 读 hash 判定）。**`tauri::tray::Rect` 私有且字段是 dpi 枚举**——锚点改用 Click.position（PhysicalPosition 光标点）；on_window_event 返回 () 别 let _（clippy）。失焦即隐（Focused(false)）。
- **自启**：autostart 插件 ManagerExt 要显式 use；菜单 label 永远从 is_enabled() 读真实 OS 态再重写。
- **深链接**：conf 声明 schemes=["tetherly"]，on_open_url + get_current 双路径都 show_main_window。
- **安装包**：release.yml 加 bundle-desktop（tauri-cli + cargo tauri build --no-dev-server）。本地实产 .app 8.18MiB / .dmg 2.82MiB。hdiutil 残留挂载报"资源忙碌"→ detach -force 重跑。
- **内存实测（M3 验收收口）**：release 壳 + WebContent XPC 双进程 ps 采样：峰值 116MB、稳态 91-96MB < 120MB 达标（daemon 23MB 不计壳口径）。注意壳必须确认 SSE ESTABLISHED 再采样，刚起的瞬时样本连不上 daemon。
- 验证：双 clippy 0 错 / workspace 1006 绿 / deny ok（瞬时 advisory 警告复跑绿）/ vitest 28 / svelte-check 0 / dist 入 git。
- 留下轮：Windows client.token、通知点击聚焦、GPU/Networking XPC 全链口径、tray_owner（需 daemon SettingsPatch 先扩）、bundle job CI 首跑。

## 2026-11-16 M3 四轮：tray_owner 托盘接管 + 10s 兜底回收
- 契约：`TrayOwner` 枚举（daemon 默认/client）+ `TRAY_TAKEOVER_RECLAIM_MS=10_000`；SettingsGeneral/Patch 各加 tray_owner；TS 绑定 bless。
- daemon：Shared 加 tray_owner/keepalive（存 Clock.unix_ms 而非 Instant——ManualClock 测试要推时钟）/sse_seen；set_tray_owner 幂等 + emit_config_changed 走既有 settings.changed 推送链。SSE events() 订阅即喂狗、每帧再喂（ping 15s < 窗口 10s，活连接永不过期）。watchdog 在 cmd_core run() 1s 轮询 expired → set_tray_owner(Daemon) + tray_set_visible(true)。
- 内置托盘隐藏：模块级 VISIBLE AtomicBool，两平台各自 500ms 轮询（不跨线程 UI 调用）。macOS 复用 TetherlyTrayTarget 加 trayVisibilityTick: selector + NSRunLoop NSTimer（popover.rs 先例），alphaValue 0↔1（菜单/拖放区保留，恢复免重建）；Windows tray_wnd_proc WM_TIMER 分支 NIM_ADD/NIM_DELETE。
- 壳：Claimed(AtomicBool) 一次性守卫，LINK 首次 Live PATCH {"tray_owner":"client"}（重连不重发）；ExitRequested 时 2s 超时同步 PATCH daemon 兜底。**修 bug**：壳暂停共享 PATCH 体是嵌套 {"general":{…}}，SettingsPatch 扁平 + deny_unknown_fields → 真机必 400（§135 埋雷），改扁平。
- 集成验收（隔离端口真 daemon）：GET daemon→PATCH client→bogus 400→SSE 3s 喂狗→静默 12s→daemon 回收 + 日志确认。
- 验证：fmt/双 clippy/workspace 40 段/deny 全绿；壳 27 测试。windows-gnu 下 use super::* 触发 unused imports（模块只依赖外部路径）——测试模块导入按需收窄。
- 部署注意：线上 0.2.58 daemon 旧契约，PATCH tray_owner 会 400；随下次部署切换。留下轮：Windows client.token、通知点击聚焦、bundle job CI 首跑、alphaValue 隐藏的真机观感。

## 2026-11-16 M3-r10：client.token 双 token 鉴权（§4.4）落地
- daemon：granted_tier(daemon, client, presented) 双凭据；issue_client/read_client/client_token_path（CREATE_NEW 原子、Unix 0600、Windows 复用保护 DACL + --data-dir 指用户 LOCALAPPDATA）；bind_and_spawn_with_token 同时签发两枚，client 失败仅 WARN。
- 壳：read_api_token 优先 client.token → api.token；通知点击定案不做（插件无点击回调，mobile-only on_action），记边界。
- 测试：apitoken 16（+7）、uihttp 集成 v1_client_token（Write 过/Admin 403/伪造 401）、壳 token 优先级。
- 门禁：fmt/clippy host+win-gnu 0 错；workspace 1017 全绿（首跑 phase1 §124 flake 复跑绿）；deny 四项 ok。
- M3 余项：bundle-desktop CI 首跑、alphaValue 真机冒烟、通知点击（原生 delegate）、409 interactive（opener 降权已覆盖）。

## 2026-11-16 M4：/ 切新 SPA + /legacy/ + 托盘深链接优先 + popover 退役
- 路由：/ = client/dist SPA（vite base /next/ → /，重建），/assets/* 固定资产（未知 404），/legacy/* = ui/index.html 单文件；get_index/INDEX 死代码删；next_url 同指 /。
- 托盘：open_client_or_dashboard（deep link 优先、浏览器回退）接 macOS openDashboard + Windows 左键/双击/IDM_DASHBOARD；Windows start 对未注册 scheme 偏乐观记边界。
- popover 全退役：platform/popover.rs（387 行）+ tray.rs Windows 子模块（385 行）+ IDM_PANEL + 菜单项/selector。
- 门禁：fmt/双 target clippy 0 错；workspace 1019 全绿；deny ok。真 daemon 45826 实证全路由。

## 2026-11-16 v0.2.59 发版 + bundle-desktop 首跑验证
- bump 三处版本位 + docs 基线（3db858a），ci 绿后 tag 触发 release。
- 首跑如实失败：--no-dev-server 是 tauri-cli v1 flag，v2 双平台秒死（43859667167）；fix → --ci（033d3b3），删 tag 重打重触发。
- 第二次全绿（43862007888）：.dmg 12m58s + .msi 16m12s 首次产出，Release 12 产物齐，publish ✓。
- 待办：deploy.sh --tag v0.2.59 生产切换（用户执行）；alphaValue 真机冒烟；M4 一周验收窗口自部署起算。

## 2026-11-16 M5 预收敛：legacy 退役横幅
- 盘点：/api/* 命中观察 §129 已全量（record + /api/v1/diagnostics legacy_routes），无需新代码，数据自部署起积累。
- ui/index.html 顶部注入退役横幅（迁 / 提示 + 链接），单文件形态不变。
- 门禁：fmt/双 target clippy/workspace 1019/deny 全绿；真 daemon 冒烟横幅两形态都在。

## 2026-11-16 M5 CLI 迁移第一波：pin 命令迁 v1
- 盘点：28 个 cmd_* ~230 处 loopback 调用全打旧 /api/*（v1 引用为零）；第一波选 pin（三臂 ↔ v1 pairing/pin 一一对应）。
- 映射：GET /api/pin→GET v1 pairing/pin（404=无 PIN）；POST /api/pin→rotate（201）；POST /api/pin/clear→DELETE（204 幂等）。
- RED 用 §129 观察设施：legacy_routes 断言真实抓到 3 路由命中，GREEN 后归零——观察数据首次当迁移验收工具。
- cmd_core 首个真 daemon 测试模块（NodeConfig::default + loopback_only + token 注入）；clippy 抓 "items after test module" 移到文件尾。
- 真 daemon 冒烟（:16459/:36717 避让生产 45716/45717）：三臂全过 + legacy_routes []。生产占 45717 时 run --port 0 不可用，隔离冒烟要显式挑端口。
- 后续波次：status/sys（Snapshot/system 对齐）→ peers list（devices 重排）→ 其余按 diagnostics 快照分批；Android 用到的路由等 Compose 改版（§9-4）。

## 2026-11-16 M5 CLI 迁移第二波第一刀：health 迁 v1
- §142 CI 绿。盘点 status/health/sys 三读取命令字段需求：health 唯一无缺口可 1:1 迁（GET /api/v1/health + /system 合成旧 HealthReport 形状）。
- status 迁移缺口备查：lan_only + transfer_stats 无 v1 资源，需先扩契约（ts-rs 漂移联动），单独立轮。
- RED（legacy_routes 抓 /api/health）→ GREEN：health() 两次 v1 调用合成，--json 保持旧 one-shot 形状（脚本兼容）。
- 门禁：fmt/双 clippy/workspace 1021/deny 全绿；真 daemon :16460 冒烟 plain+json 全字段 + legacy_routes []。

## 2026-11-16 §143 CI 败修复 + §144 peers 迁 v1 + Device.group 契约
- §143 CI 双平台败：health 测试 401——set_api_token 是进程级静态，cargo 并行测试竞态。修：set_port_api_token（per-port）。教训：bin 测试 token 一律 per-port。
- Device 契约补 group（peers/SPA group-by 都要）：contract RED → lib.rs+model.rs 三元组+hub 夹具+TS 绑定 bless 重生成。
- tetherly peers 迁 GET /api/v1/devices：扁平列表按 online/trusted 重建两段视图；--json 直接透传 v1 形状；self_id 头去掉。
- 门禁 1023×2 稳定（首跑 m3_9 §124 flake）+ deny ok + 真 daemon :16461 冒烟（发现设备含 group:null）。
- status/sys 还缺 v1 资源（lan_only/transfer_stats/系统遥测），等契约扩展轮。

## 2026-11-16 §145 契约扩展 + status/sys 迁 v1 —— CLI 读取命令全迁完
- 契约：System.lan_only、Transfers.stats（新 TransferStats DTO）、GET /diagnostics/sys（SysTelemetry 字段名逐字镜像旧 /api/sys，打印器零改动）。contract +3，TS bless 重生成。
- status：四 v1 资源组合（system+pairing+devices+transfers）合成旧形状，--json/print 零改动；sys：单行替换到 v1。
- RED 双确认（/api/state、/api/sys 命中）；冒烟 :16462 全过 + legacy_routes []。
- §124 补充：spawn_ui 的 set_api_token 全局覆盖是 401 竞态根源；run2/run3 与冒烟 daemon 抢 CPU 放大，清场后 ×2 稳定 1028。spawn_ui per-port-only 改造列为候选小轮。
- 五个读取命令（pin/health/peers/status/sys）全迁 v1；旧路由剩旧控制台+24 个写型模块分批。

## 2026-11-17 §146 spawn_ui per-port-only + config 迁 v1 + dial 定案
- spawn_ui 移除 set_api_token 全局调用（不变量测试 RED 抓泄漏）+ tray 三点换 api_token_for(port)。诚实记录：401 机制未复现，属原则性修复。
- config 读 = system+settings 合成旧形状；写 = PATCH /settings（SettingsPatch 字段一一对应）。坑：loopback_request token 显式参数不走 per-port 表。输出裁剪 data_dir/memory_clip/ancs（无 v1 资源，明示）。
- dial 不迁移：connect_peer 只服务已知设备，首配对拨号（addr+PIN）无 v1 资源，等 POST /pairing/dial 契约扩展。
- m3_9 workspace 并行负载下连败两次但单独跑绿（零 spawn_ui/loopback 路径）——负载型竞态实证，非本轮因果。清场后 1030 ×2 稳定。
- 六命令迁完（pin/health/peers/status/sys/config）。

## 2026-11-17 v0.2.59 生产部署 + alphaValue 冒烟 + 壳接管链三 bug 修复
- 用户委托执行 deploy.sh：双端 0.2.59 上线（mac 67992 / win 4308），保护进程完好，M4 验收窗口起算。
- 客户端 .dmg 与 daemon bundle 同名不同 id——ditto 装 /Applications/Tetherly Client.app 规避覆盖。
- alphaValue 三连拍像素级确认：claim 后 2s 图标消失，13s watchdog 回收后恢复。AX 定位 item（"⇄ Tetherly" 随菜单栏漂移）。
- 三 bug：①events.rs 成功路径从不 emit Live（SinkEvent 枚举修复）②setup 先 spawn 后注册监听器（首个 Live 落空，顺序倒转）③PING 15s 不喂 keepalive 且 > 10s 回收窗口（5s + ping 臂补喂）。③在生产日志 5 次 reclaim 实证，确定性 bug。
- 端到端 t+5/17/32s 三测全 client。门禁 1032×2 + 壳 28 + 双 clippy + deny。三修复待发 v0.2.60。
- 教训：托盘冒烟要先清场（本地 e2e daemon 的托盘与生产托盘同貌，位置漂移干扰判定）；watchdog 在 bin 不在 Node，测试需轮询复刻才不假绿。

## §149 (2026-11-17) — peers 写命令迁 v1 + v0.2.60 发版/部署/生产复验
- 五写命令（connect/block/unblock/alias/unpair）→ v1 设备动作；RED 断言五旧路由命中归零；冒烟 :16465 五错误路径 + legacy_routes: []。附录更新：剩余 tags（读改写 §150）/blocked（契约缺 reason）/disconnect（无 v1 动作）/link·heartbeat（diagnostics 域）。
- 门禁：首跑 10 连败=冒烟 daemon 并发负载竞态（§124 家族），清场后 1034 ×2 稳定；CI 链 §147 复跑/bump/§148/§149（43921252135）全绿。
- 发版：tag 314f996 → release 43923712802 七 job 绿、11 资产；deploy 双端绿（mac pid 21775 / win 20976，保护进程 3/3，SYSTEM）。
- 生产壳接管复验通过：claim 即时 + t+40s 存活 + 壳重启再 claim t+28s 存活；daemon 日志 tray reclaim 部署前 11 次 → 部署后 0 次（决定性证据）。
- 教训：①macOS 26 移除了 NSStatusItem 的 AXAlphaValue，像素/AX 探针失效，reclaim 日志计数是接管状态权威证据；②open 已运行实例不换进程，claim 是 per-process once，复验前先 kill 旧壳；③截图取证在时钟秒数刷新区永远有抖动，别在菜单栏右侧做像素 diff。

## §150 (2026-11-17) — tags 迁 v1 读改写
- GET devices（id/alias/name 匹配）→ 本地集合运算（幂等/case-blind）→ PATCH 全量 tags；校验两端一致。旧路由 RED 归零；冒烟 :16470 干净。
- 发现：legacy add 对合法未知 id 静默建档（意外行为），收敛为显式 404；seed_trusted_peer 是 cfg(test) 库内后门，bin 测试走 trusted.json 种子。
- 坑：target/debug/tetherly 可能是旧二进制（clippy 不产 bin）——冒烟前必 cargo build -p tetherly-node --bin tetherly。
- 1036×2 稳定；门禁全绿。peers 写域剩 blocked/disconnect/link·heartbeat（均待契约）。

## §151 (2026-11-17) — 契约扩展 block_reason + disconnect 动作
- Device.block_reason: Option<String>（Parts.blocked bool→Option<BlockedPeer>）；POST /devices/{id}/disconnect Device 回读（already_disconnected 折入 online）。
- CLI disconnect/blocked 迁 v1；blocked --json 合成 {ok,count,peers} 形状；排序插入序→id 序。
- 冒烟发现：resolve_peer 名字匹配只扫 trust 表，discovered-only 名字 404、全 id 可用——非回归（legacy 同路径），名字解析覆盖 discovered 是候选小轮。
- 1038×2；门禁全绿；bindings 再生。peers 写域只剩 link/heartbeat/ping（diagnostics 域）。

## §152 (2026-11-17) — 链路质量域契约扩展 ×5 + link/ping/probe/heartbeat 迁 v1
- 契约：HeartbeatConfig/HeartbeatPatch/LinkQuality/LinkQualityPeer/LinkProbeRequest/LinkProbeReport/DevicePingReport；grade enum→String；契约 21。
- v1：GET /link、GET/PATCH /link/heartbeat（空 patch 400）、POST /devices/{id}/ping|probe；CLI 四命令迁 + auto_target_peer 移 CLI 侧（0/1/多 peer 语义保持）。
- 坑：批量 python 替换 allowed() anchor 因 fmt 漂移静默未命中——m2a 405 断言 404；重读文件手动补行后绿。批量编辑后必须 grep 验证落盘。
- 冒烟 :36732（LAN :46732 避让生产 :45717，生产未动）：heartbeat 15→30→15、405/400/404 矩阵、legacy_routes 0。
- workspace 第 2 遍 phase1 1 失败（§124 flake 家族，隔离重跑 2 遍全绿）；门禁全绿；bindings 无漂移（link DTO 不在快照面）。
- 22 命令迁完，peers 写域清零。剩余：dial 契约扩展、logs/metrics CLI 迁移。

## §153 (2026-11-17) — 流量遥测契约扩展 + metrics/traffic 迁 v1
- 契约：ChannelTraffic/ProtocolMessages/TrafficTelemetry 逐字段镜像 telemetry.rs；ok:true 折掉；契约 22。
- v1：GET + PATCH /diagnostics/metrics（reset=幂等 PATCH 折入 GET 回读）；CLI handle_traffic_cmd 两旧调用迁 v1，打印机零改动。
- logs 判定更新：v1 端点 §145 已有，CLI 无 logs 子命令——旧 /api/log/* 改判"删除候选，等消费面核查"。
- 坑（复现确认）：--all-targets clippy 已固化默认；phase1 套件模式偶发超时 ×2（enterprise_file_limits/action_predefined），隔离重跑全绿。
- 冒烟 :36733：reset PATCH 回读、405 矩阵、legacy_routes 0。23 命令迁完。剩余：dial 契约扩展、logs 消费面核查。

## §154 (2026-11-17) — dial 契约扩展 + POST /pairing/dial + CLI 迁 v1
- 契约：DialRequest{addr,peer,pin}（deny_unknown_fields）/DialReport{ok,addr}；契约 23。
- v1：POST /api/v1/pairing/dial（pin 先 set 再 dial_peer，400 透传 legacy 文案）；CLI dial 换 URL，输出不变。
- 冒烟 :36734：400 三态（无 target/不可达/坏 pin）、405+Allow、deny_unknown_fields 报字段清单、legacy_routes 0。
- 24 命令迁完；dial 特殊项清账。M5 主体迁移完成，剩 logs 消费面核查。

## §155 (2026-11-17) — 旧路由删除轮：dial/metrics/log 8 条别名下线
- 删 uihttp 路由臂×3 + api_log 模块 + api_core 三 handler + DialBody；apitoken 门禁/测试表同步修剪。
- 测试改造：log 路由测试→404+503 双断言（v1 承接）；token-gate 503 探针换 v1 log-level（daemon/restart 真重启返 200 不能当探针——二次修正如实记）。
- 事故复盘：后台全量测试超时强杀→孤儿测试节点级联重启→用户机器死机。整改（用户指令）：本地禁产物构建/禁全量测试（交 CI）、禁后台强杀、单测面+残留扫描。
- 门禁全绿；lib352/bin16/契约23；全量交 CI。M5：24 命令 + 8 条旧路由删除。
- §155 CI 修复链（如实）：e15d041→21a768a→31a917f→d0fabcc→44f4bfb→a05a490 共 6 提交，4 轮红。①phase1 metrics/dial 旧调用漏扫（教训：删除前全仓扫含根 tests/）；②python 插入行未 fmt；③TrafficReport→TrafficTelemetry 反序列化 ×5——注意 tetherly_node::TrafficTelemetry 是 node 内部结构（字段私有无 Deserialize），测试要用 tetherly_api::TrafficTelemetry（根 Cargo.toml dev-deps 补 tetherly-api）；④reset PATCH 走 loopback_request 显式传 None token 吃 401——v1 非 GET 一律要 token（v1_required_tier→Write），须传 api_token_for(port)，与 CLI 同法；loopback_get 自动附带所以 GET 全过。第 4 轮 mac/win/android/deny 绿，ubuntu 独红 m3_2（§124 flake 家族，隔离重跑绿，run 44070194176 五作业 success）。§155 收口。
- v0.2.61 发版（2026-11-17）：bump f7007da（Cargo.toml/Cargo.lock/client package.json+tauri.conf.json 同步 0.2.61），release 工作流 11 资产全绿，deploy.sh --target all 双端成功——Mac pid 71252、home-win 20976→19304（NTFS 原子热重启），版本门双端 0.2.61，受保护进程 SRBMiner/komari-agent/verge-mihomo 部署前后 3/3 完好，双端 live_peers=1。M5 §149-§155 战果上生产。

## §156a (2026-11-17) — profiles/caps 域契约 + v1 端点 + CLI 迁移 ×8
- 契约：Patch<T>{Leave/Set/Clear} 三态枚举——serde 双 Option 塌缩坑实测（外层 visit_none 先触发，JSON null 与键缺失不可区分，/tmp/probe_dd 探针验证）；PeerProfileDto/PeerProfiles/PeerProfilePatch/PeerProfileUpdated + CapsUpdate（deny_unknown_fields）+ LocalCaps/PeerCaps；契约 25；bindings 再生。
- v1：GET /profiles、GET/PATCH /profiles/{id}、GET/PATCH /caps、GET /devices/{id}/caps、GET /caps/peers（全量 peers 裸数组）。
- CLI：cmd_profile ×5（set 稀疏 PATCH、group clear 用 null→Clear）+ cmd_core caps ×3（PATCH 带 api_token_for；单 peer 打印机改单对象直读；both 臂 count 取数组长度）。
- 门禁：fmt/clippy --all-targets 绿；契约 25/bin 16/lib 352 全绿。全量与冒烟交 CI。

## §156b (2026-11-17) — topology/routes/net 域契约 + v1 端点 ×13 + CLI 迁移 ×13
- 契约 ×15（Topology×11 + Routes/Net×8 组）：HashMap→平行向量、枚举折小写字符串；Option<f32> 不能 derive Eq（E0277 坑）；契约 27 绿 + bindings 再生。
- v1：GET /topology(/peers/policy)、PATCH policy/config、POST links/unlinks/sync、GET/POST/DELETE /routes（DELETE ?peer=）、GET /net、POST /net/refresh。
- CLI：cmd_topology ×8（打印机双 zip + timestamp_ms 现算）+ cmd_peers ×5（反序列化换 tetherly_api:: 契约类型，node 内部结构 ok 折掉后不匹配）。
- 门禁：fmt/clippy --all-targets 绿；契约 27/bin 16/lib 352 全绿。全量交 CI。

## v0.2.62 发版 (2026-11-18) — §156a+§156b 上生产
- §156a（profiles/caps 域 + Patch<T> 三态契约，run 44105392730 全绿）+ §156b（topology/routes/net 域 15 DTO + 13 端点 + CLI ×13，run 44117110707 全绿）合并发版。
- bump Cargo.toml/package.json/tauri.conf.json 0.2.61→0.2.62；发版后 deploy.sh --target all 双端部署。
- v0.2.62 双端部署完成 (2026-11-18)：tag 触发注意——release 工作流只认 push tags v*，bump 提交必须补打 tag（这次漏打后补 v0.2.62）。release run 44127719173 全绿 11 资产；deploy.sh --target all 成功——Mac pid 1148、home-win 19304→23648（NTFS 原子热重启），版本门双端 0.2.62，双端 live_peers=1，受保护进程 SRBMiner/komari-agent/verge-mihomo 部署前后 3/3 完好。§156a+§156b 上生产，待用户线上测试。

## §157-§160 总优化轮 (2026-11-18) — 深度 review 收敛后四批实施
- review 结论先行：三路代理 + 逐行实证交叉验证——runtime 3 P0 全误报（guard 块内释放，未采纳）；坐实 API 3 P1 + CLI/core 2 P1 + P2 批。
- §157 API P1：core_error 逐变体分类映射（400/409/413/503，caller-state 409）；五个路径段提取器 percent-decode（空格/UTF-8 别名恢复可寻址）；requires_token 注释对齐 ADR-139（inbox-open v1 是 Write 级不改行为）；V1_ADMIN_PATHS pinning 测试。
- §158 CLI/core P1：LockoutTable sweep 有界化（floor 64，锁过期即刑满放弃粘性重锁——语义取舍已记录）；status/sys/link status 三个 --watch 瞬态容错（首轮 fatal 保留、后续 warn+continue）。
- §159 P2 批：unblock --json 404 软化；八处 map_err 收敛 core_error；TopologyUnlinkReport/TopologyConfigReport 补契约 DTO（+bindings）；SSE 写超时（4×PING，防卡死客户端绕过 lag 检测）；drag_controller/drag_target 毒锁恢复 into_inner；peer_caps_all 错误传播。
- §160 M5 第九批：Discovery×3 + DaemonRestart/Shutdow×3 契约 DTO；GET/POST /api/v1/discovery + POST /api/v1/system/shutdown/restart（**V1_ADMIN_PATHS 首个现存 Admin 路由正式接线**）；CLI discover/stop/restart ×3 迁 v1（Admin 级显式 api_token_for）。
- 门禁：契约 28 / bin 16 / lib 356 全绿；clippy --all-targets 0 错；集成测试 v1_discovery_reads_and_lifecycle_gates 一次过（403 forbidden 包络 + 401 fail-closed + 405）。全量交 CI。

## §161 M5 第十批 (2026-11-18) — notify/reply/OTP + message 域迁移 v1
- 契约 ×31：notify 域 21 + message 域 10；NotifyDismissReport 单条+批量双形态折叠；NotifyConfigReportDto 把 legacy 平铺计数折进嵌套 totals；MessageDispatchReport selector 折 as_label 字符串；契约测试 +2（30）+ bindings 再生。
- v1 ×16：POST /notify（0 peer→409）、GET /notify/candidates|replies|config、PATCH /notify/config、POST /notify/dismissals|otp|copies|insertions|open|test、POST /replies(/take)、GET|POST|DELETE /messages（clear 用 DELETE?peer=——DELETE 无 body）。错误全走 core_error。
- CLI ×16：cmd_notify 12 + cmd_messages 4；banner config 写改 PATCH（loopback_request 显式 token）、读改嵌套形状；message clear 改 DELETE；watch SSE 不动（M6 范围）。
- 坑：send_message_to_peer("") 的 peer 解析先于空文本校验，无 peer 时 404（PeerNotFound）而非 400——测试按真实语义断言。
- 门禁：fmt --all / clippy 0 错；契约 30 / bin 16 / lib 357 全绿。全量交 CI。

## §162 M5 第十一批 (2026-11-18) — 文件域迁移 v1（inbox/offers/transfers/outbox/storage/resume/policy）
- 契约 ×26：§162a InboxOpenReport（open 204→200 升级）+ §162b 文件域 25 DTO（FileOffer/Decision（单+批双形态折叠）/Send/Dispatch/Batch/Outbox/ResumeMarker/Storage/AutoAcceptPolicy…）；Patch<T> 三态复用（Deserialize-only，PATCH 体不 derive Serialize）；契约测试 +3（34）+ bindings 再生。
- v1 ×14：inbox GET/DELETE/POST + content 裸字节 + open；offers GET + accept/reject（id 单条 / peer+all 批量折叠）；transfers 单发（path|data_base64，FILE_MAX 前置，0 peer→409）/multi/batch；outbox GET/DELETE（status FromStr 别名表，未知→400）+ resend；storage GET/clean（orphaned_only 缺省 true）；resume GET；policy GET/PATCH（空 patch→400，null→Clear）。
- CLI ×24（cmd_files 全域）：transfers/inbox/offers/send 三态/outbox/storage/resume/auto-accept；clear 类用前后快照差值算计数；send 走 loopback_post_with_deadlines 300s（daemon 全文件哈希慢）；send-multi body 键 `"type"`→`"device_type"` 修正；打印机改读平铺 policy（legacy 嵌套 policy 折掉）。
- 资源语义取舍：GET /transfers 返回全量，id/active/terminal/direction 过滤留 CLI 客户端侧；GET /inbox search/limit 服务端（count 镜像 legacy=过滤后截断前）。
- 坑：Option 字段序列化是显式 null 而非键缺失（无 skip_serializing_if）——测试断言按 null 判；OutboxStatus 无 Display 用 as_str()；FileDecisionRequest 空 {} 是 400（缺 id/all/peer）非 404。
- 测试：集成 v1_file_domains_serve_and_gate（空资源/status 别名/policy 三态/决策 400+404/inbox 内容往返+穿越拒绝/client token 门禁/405）；门禁 fmt/clippy --all-targets 绿；契约 34 / uihttp 78 / apiv1 28 全绿。全量交 CI。

## §163 M5 第十二批 (2026-11-18) — clip + audit 域迁移 v1
- 勘察修正：字面扫描否定旧估计——cmd_core/cmd_peers/cmd_profile 的 legacy "存量"全为测试守卫断言与注释，实际为 0；真实存量 = clip 9 + audit 2 + 小命令域 ~110 + screen 165。
- 契约 ×14：clip 10（ClipPolicyDto 平铺 + Patch Deserialize-only + ClipPollReport dispatched_peers 保持 legacy 数字）+ audit 4（AuditTrail/Query/Clear + EntryDto 枚举折字符串）；契约测试 +2（36）+ bindings 再生。
- v1 ×9：GET/DELETE /clip + status + history + sync（peer 已连接校验 + 1 MiB 上限）+ poll + GET/PATCH policy（body+query 折叠 body 胜出）+ GET/DELETE /audit（坏枚举 400）。
- CLI ×11：cmd_clip 9 + cmd_audit 2；print_clipboard_policy 改读扁平资源（嵌套 policy/stats 折掉）；policy 写 PATCH 显式 api_token_for；clear 双域动词修正为 DELETE。watch SSE 不动（M6）。
- 坑：审计环与策略共享 runtime 态——policy PATCH 自身留审计痕迹，集成测试按 total=1 断言而非空环；u64/usize 边界（max_payload_bytes）用 try_from 饱和；parse_query_map 临时值借用 E0515——先落 qmap。
- 门禁：fmt/clippy --all-targets 0 错（bool_comparison 1 处修）；契约 36 / bin 16 / uihttp 79 / apiv1 28 全绿。全量交 CI。

## §164 M5 第十三批 (2026-11-18) — action + bench + recommendation 域迁移 v1
- 历史帧建模修正：运行时 ActionResultFrame 无 direction/peer_id/peer_alias 字段（CLI 打印器同名读取全是 unwrap_or 兜底），契约按真实字段建模，不虚构数据。
- 契约 ×16：action 12（LocalActions/RemoteActions/TriggerReport/History/PolicyDto 平铺 + 3 个 strict Request + ActionPolicyPatch）+ bench 7（BenchStatus/BenchDecision/BenchPolicyDto 平铺 + BenchStartRequest/Patch + Session/Report DTO）+ 推荐 8（StreamingRecommendationDto + 4 tier DTO codec 折字符串 + RecommendationDto/Summary/Status）；全部折 ok/message 信封；契约测试 +2（38）+ bindings +122 行。
- v1 ×13：action local GET/POST + DELETE /local/{id}（legacy ?id= 折路径参数）+ remote GET/refresh/trigger（body+query 兼容）+ history + GET/PATCH policy；bench GET/start/stop/GET/PATCH policy；recommendation 单 peer/聚合。PATCH/POST 显式 api_token_for。
- CLI ×17：cmd_action 9 + cmd_bench 8；POST/PATCH 全走 loopback_request 显式 token；打印机零改动（资源逐字段同形 + unwrap_or(v) 兜底天然兼容扁平资源）。
- 语义细节：legacy query 变体对 action/bench policy allow-list 是「追加去重」语义（body 是替换）——PATCH handler 用 from_query 标记区分；max_timeout 过 ActionValidator::clamp_timeout；bench direction 长宽表 lenient 解析（pull/download/rx/recv）。
- 坑：route() 是返回 UiResponse 的函数体而非闭包——dispatch arm 里 `?` 与 `return Err(...)` 不可用，arm 早退写 `return error(...)`、拆出的 async fn（返回 Handled）才用 `Ok/Err` 包裹；parse_body 有 `T: Default` 约束，strict DTO 用新增 parse_body_strict；clippy extend_with_drain 4 处（extend(drain)→append）。
- 门禁：fmt/clippy --all-targets 0 错；契约 38 / uihttp 80（新增 v1_action_bench_serve_and_gate）全绿。全量交 CI。
## §165 第一批（2026-11-18）：11 小域契约层 DTO-only 全量落 v1

- §164 CI windows 失败为 m3_2_peer_gone_returns_cursor 抖动（5s 上限 vs 2.5s 宽限期），rerun success——非代码问题，§164 关闭。
- 契约 ×93（power5/media10/ancs8/input12/handoff11/focus9/camera7/mic7/audio8/locate8/display7，以 runtime 结构体为准）+ 测试 +3（41 绿）+ decl ×93。clippy -D warnings 干净。
- 端点/CLI 未切（cmd_input 曾提前切 8 处已回退）；下批按域逐个：先端点后 CLI。

## §165 第二批（§166 节，2026-11-18）：11 小域端点 + CLI 全量迁移 v1 完成

- 端点 ×49 + model mappers 全套 + CLI 调用点 ×57（power8/media4/ancs6/input8/handoff7/focus7/camera6/mic6/audio6/locate5/display5）。每域垂直切片（mappers→端点→gate→CLI→check），lib/bins 每 步 0 错。
- 契约修正：`ActiveDisplaySessionDto.mode` String→`DisplayModeDto`（runtime 是结构体；CLI 读 mode["width"]）。TS bindings 再生（2 行 diff），客户端无引用。
- CLI 陷阱：input 请求键 forward_*→allow_*（deny_unknown_fields 400）；ancs remove-rule 改 DELETE 带 JSON 体；policy 写 POST→PATCH；printing 全部适配平铺 DTO。
- 门禁：fmt 干净；clippy api+node --all-targets 0 错（修 9 个 model.rs 未用 import）；apiv1 28 / 契约 41 全绿；bindings 无 BLESS 一致。集成测试 + 全量交 CI。
- 遗留：/legacy/ 别名、SPA legacy 路径、SSE watch、screen 大域 → §167。
- §165 第二批 CI 修复链（4 轮）：e2ca137 fmt 门禁（import 清理后未复跑 fmt --check）→ 9e343b0 clippy：route arm GET /input 重复 + 3 个 mirror wrapper too_many_arguments（events.rs 同款 allow）→ d81b2a7 gate 表 allowed() GET 前缀组也含 /input（遮蔽 §165 arm）→ 178743a **全绿**（ubuntu/macos/windows/deny/android）。教训：v1.rs 改动 gate 表与 route arms 必须同查同改；推送前固定跑 clippy --all-targets + fmt --check + 单目标测试。

## §168（2026-11-19）：screen 域全量迁移 v1 + /legacy/ 删除

- 契约 ×113（56 报告 + 51 patch + 6 响应；生成器从 runtime 源码提炼防手抄错）+ v1 端点 ×~150 + CLI ×151（6 文件）+ /legacy 与 ui/index.html（10230 行旧控制台）删除。SPA 零 legacy 调用已核实。
- 迁移器两坑：GET 路径透传未重写（全局字面量二次替换补刀）；变量路径 `&uri` 进 resolve 字符串拼接被破坏（undo/redo 误指根路径——index 定位修复）。
- 契约细节：`pts` alias `pts_us`、`loop` rename、annotation `from_str`/`to_str` canonical 化；AnnotationKind tagged 镜像；`wire_enum` 泛型保证枚举 wire 串与 legacy 逐字节一致。
- 门禁：fmt check 绿 / clippy --all-targets 0 错 0 警 / apiv1 28 + 契约 42 + uihttp 79 绿 / bins 0 错。legacy /api/* 路由面（~900 arms + 27 文件）成死代码 → §169 收敛删除。

## v0.2.63 release（2026-11-19）：§168 发版 + 部署本机

- CI 44708af ubuntu 首轮红：phase3 m3_2_peer_gone_returns_cursor 超时（cursor did not return home）。取证：input.rs/phase3.rs 在 §168 零改动、该 job 整体慢 8 分钟 → runner 时序抖动；rerun 全绿。与 §164 同款抖动，第二次出现——后续可考虑把 5s 上限放宽到 8s 一劳永逸。
- release 流程教训：tag 前置 bump 必须四文件齐（Cargo.toml + Cargo.lock `cargo update -w --offline` + client/package.json + client/src-tauri/tauri.conf.json）。首版只 bump 了 Cargo 侧，dmg/msi 文件名仍 0.2.62 → 撤 tag 重打（tag 移交 0ea60eb），第二轮 release 44374655160 全平台 0.2.63。
- 撤销错 tag 的顺序：`git push origin :refs/tags/v0.2.63` → `git tag -d` → `gh run cancel <run-id>`（release.yml publish 有 `if tag` 守卫，取消 run 即不会发布半成品）。
- 部署链路（本机 Mac）：`scripts/package-macos-app.sh --tag v0.2.63 --restart`——下载 release 产物 → 暂存 bundle ad-hoc 签名（DR 锚 bundle id，TCC 辅助功能权限跨版本保留）→ 原子替换 /Applications/Tetherly.app → API token 优雅 shutdown 旧守护 → LaunchServices 重启。Windows 侧产物 tetherly-windows-x86_64.exe / tetherly-cli-windows-x86_64.exe / .msi 已在 release assets，按需分发。
- 部署后 live 验证（v0.2.63 @ 127.0.0.1:45716）：/legacy 404、/api/v1/{screen,input,mic,health} 200、screen/tile GET→PATCH→DELETE 往返 200、watermark POST enable→PATCH off 往返 200（该组无 DELETE 语义，405 属 gate 表设计）、未知路径 404、无 token 401——全符合契约。
- 事故复盘：清理残留资产时误用 `gh release delete v0.2.63 --yes` 把整个 release 删掉（应为 `gh release delete <tag> <asset> -y`）。恢复：run 44374655160 的 artifacts 仍在 Actions 侧，`gh run download` 全量拉回后 `gh release create v0.2.63 --generate-notes dist/*` 重建——比重新触发 CI 快 15 分钟。

## §169（2026-11-19）：legacy /api/* 路由面整体删除（-33,796 行）

- 取证：SPA/Android/CLI 活代码全在 /api/v1（含 SSE）；bins 剩余 legacy 字面量全是测试 starts_with 断言。/api/state→system/devices/pairing、/api/peers/*→devices/{id}/{action} 全覆盖。
- 删除：429 route arms + 27 api_*.rs + dto.rs 1308→46 + requires_token + seed_trusted_peer + 107 个纯 legacy 集成测试 + uihttp mod tests 57 个。保留 hub.legacy 计数器（外部旧客户端迁移观测）。
- 坑：批量删 fn 后孤儿 doc 注释/impl/尾括号成串（phase1 mod tracing_capture 差点被误删——OTP 日志泄漏测试依赖，从 git 抢救）；CORS 测试改 v1 路径（health POST 先 403，Origin gate 先于 route 分发）。
- 门禁：fmt 绿 / clippy 3 crates 0 错 0 警 / node lib 300 绿 / 契约 42 绿 / bins 0 错。phase 全量交 CI。

## v0.2.64 release（2026-11-19）：§170 flake 根治 + legacy 生效
- §170 两件套（fdf069e）：m3_2 宽限窗测试预算 5s→8s（慢 runner 两次偶发的证据链写进注释）；m3_12 取证埋点——`receive_input_tunnel_resp` ACCEPTED 分支补决策环 `takeover` 条目、target pump peer-session-gone teardown 在 release 前补 `pump_exit` 条目，下次 windows CI 红灯可区分「pump 见通道死亡」vs「release 竞态」。零行为变更。
- **§169 生效时点勘误**：legacy 删除落在 main 晚于 v0.2.63 tag，故 v0.2.63 产物仍含 legacy 路由（双端实测 /api/peers 200 属预期）；v0.2.64 起 legacy 404 真生效（双端已验证）。hub.legacy 观测从本版起才是有效窗口。
- 发布链顺利：四文件 bump（28b4f28）→ tag → release 44406288837 全 7 job success → 11 assets（dmg/msi 名 0.2.64 正确）。部署：Mac package-macos-app.sh + Windows deploy.sh 双端 0.2.64，保护进程 3/3、SYSTEM、live_peers 1。
- **连锁修复（本轮发现）**：deploy.sh / package-macos-app.sh / 三个 verify_*.sh 全部还在打 legacy 端点——deploy.sh 版本门 /api/health→/api/v1/health、restart→/api/v1/system/restart（Admin tier）、package-macos-app shutdown→/api/v1/system/shutdown、ensure→GET /api/v1/input、peers→/api/v1/devices（响应形状 live[]→typed array，jq 断言同步改）、screen/start|allow|twcc→/api/v1/screen/*。本次 Windows 部署版本门是手工绕过验证的，脚本已修，下轮部署即走新链。

## v0.2.65（2026-11-19，§171）input-link 清账 + Android Compose + 缺口核对
- 837ea2e 代码 / d534f16 文档+bump：4 修（F 滚轮 /120、C 冷启动 edge 镜像、B tb dwell 离边清、D rem 坐标同步 + 2 守护测试）+ 3 取证关闭（A/E/G）+ H 能力缺口；Android Compose 改版（plugin.compose + BOM + M3，MainActivity 重写行为等价）
- Windows viewer（macOS-only stub 确认）/ Edge Strip / Ghost Overlay / 被动拖拽核对为设计层未实施，todo.md 口径更新
- 本地验证：rm target → fmt 绿 → clippy -D warnings 0 错 → core 332 + node 300 + phase3 13 全绿
- CI：ci 44422469814 五作业绿；release 44422469739 首跑 build-android Maven Central 429（网络抖动，同代码 ci android 绿）→ rerun --failed 绿，11 assets
- 部署：Mac health 0.2.65；Windows 热重启 9076→19764 SYSTEM，受保护三进程不动；Mac /api/v1/input + trace 环 200 正常
- 遗留观察：真机滚轮一格滚动量验证（F 修复体感）；doc-check 4/4 中 unittest 项红为 agent-fleet 会话的 infra-services.md 重复行，非本轮

## 2026-11-20 · §175 hook 并发启动治理（本地定向 GREEN）
- 取证（修复计划 §5 Needs-confirmation 第 2 条）：
  - `ensure_input_capture`（input.rs）为对 `input_capture_active` 的 check-then-act；该标志在 `spawn_input_capture` 返回后才置位，而 `start_input_hooks` 的 tap 线程要阻塞等就绪 → 1s link-watcher tick 与 tunnel accept 路径可同时过门重复 spawn。
  - macOS `start_input_hooks` 二次调用无任何守卫：会产生第二个 CGEventTap + 第二消费线程；运行层只持第一份 HookControl，孤儿 tap 按无主 `keyboard_armed` Arc 陈旧状态吞/放事件。
  - Windows 有 SENDER OnceLock 拒绝二次启动，但拒绝前已执行 `system_cursor::restore(true)`（无谓副作用，未修，风险低）。
- 修复：
  - `runtime.rs` Shared 增 `input_capture_spawning: AtomicBool`；`ensure_input_capture` 改经 `try_claim_capture_spawn`（compare_exchange 单飞）admit，spawn 完成/失败后释放；争用方返回 Ok(true)（armed-pending，等下个 tick 复查）。
  - `platform.rs` macOS 增进程级 `HOOKS_STARTED` AtomicBool 守卫，二次启动报 "input hooks already started"；Accessibility 未授权路径先释放守卫允许后续重试。
  - shutdown 定级 confirmed design：tap 设计为进程生命周期（dock-swipe 注释锚定），会话拆线只 disarm 不拆 tap，不做 in-process 拆除。
- 验证：新增 `capture_spawn_claim_admits_exactly_one_contenting_caller`（8 线程竞争恰 1 胜者 + 释放可再占）；`cargo test -p tetherly-node --lib --locked` = 318 passed（基线 317+1）；clippy -D 级 0 lint；rustfmt + git diff --check 通过。
- 文档：REVIEW_REMEDIATION_PLAN.md §5 条目划掉并记录定级 + §8 追加本批；todo.md 增 §175 条目。
- 残余：随下个 tag 走 CI 三平台 + Android 门禁；无新增 manual-required。

## 2026-11-20 · §176 smooth-wheel 子 detent 治理（本地定向 GREEN）
- 决策：用户拍板「承诺平滑滚轮（修复）」——修复计划 §5 第 3 条 Needs-confirmation 销项。
- 取证：Windows hook WM_MOUSEWHEEL/HWHEEL 每事件 `raw/120` 独立整除；精度滚轮单事件 |raw|<120 的增量被整体丢弃且无跨事件累积，远程端滚动无响应直至单事件凑满一格。macOS tap 直发行 delta、wire 协议（i16 行）与目标端注入均不受影响。
- 修复（platform.rs）：
  - `WHEEL_DELTA=120` 常量 + `wheel_accumulate` 纯函数：saturating 累积 → truncated `/`/`%`（保号余数）→ ±32 detent 钳制。
  - `hook_wheel_state` thread-local 双轴余数（LL 滚轮事件单线程纪律同 hook_move_state）；两处 wheel 路径改走累积器，0 行事件不推空帧。
  - 初版用 div_euclid：负向首事件即溢出 -1（余数 +105）——测试 RED 捕获，改 truncated 除法后负向余数保持待发方向。
- 验证：`wheel_accumulator_emits_detents_across_sub_delta_events`（8×15 跨事件凑格、整格直通、负向保号、互抵不虚构、100k 敌意流饱和）；node lib 319/319；clippy 0 lint；rustfmt/diff-check 通过。
- 编辑事故记录：本轮两次 Edit 误锚到相邻同名/相邻块（screen_size stub 撞 Linux cfg 属性、bogus 测试头被挤），均由编译/结构检查当场发现并修复——大文件 Edit 必须紧跟结构验证。
- 文档：计划 §5 划线 + §8 追加；todo.md 增 §176 条目。
- 残余：随下个 tag 走 CI；Windows 真机滚轮手感为 manual 观察项。

## 2026-11-20 · §177 前端/Tauri 层深审 + activity 通知风暴治理（本地定向 GREEN）
- 范围：client/src-tauri/src（events 613 行/proxy/token/tray/trayui/trayowner/lib）+ client/src/lib（transport/stores/App.svelte）全量通读。
- 审查结论（确认为设计，不修）：
  - Tauri SSE（events.rs）：握手 status+Content-Type 校验、idle 45s、last_event_id 续传、seq 去重/rewind 拒绝全部在位，8 个 loopback 测试锚定。
  - http.ts SSE：handshake timeout + per-read idle watchdog（reader.cancel 后走既有 reconnect），正确。
  - proxy.rs read-to-EOF：依赖 daemon Content-Length + Connection: close 契约，有 mock-daemon 回环测试锚定；path/method 白名单在位。
  - token.rs：token 只在 Rust 进程，WebView 只见 invoke；client.token 优先级与 §4.4 一致。
  - lib.rs ExitRequested 同步 block_on：退出一次性路径，2s 超时兜底，低风险记录。
- 缺陷（confirmed）§177：activity.changed 通知风暴。daemon 侧任一 Activity 主题事件（MessageSent/Received、HandoffInboxUpdated、Candidate* 等）都整段重发全量 Activity 资源（hub.rs Topic::Activity → ActivityChanged）；tray.rs notice_for 每帧无状态重数 unread/opened_ms==null → 用户不读消息/不点开接续，每次相关变更都弹同一条通知（风暴）。
- 修复：
  - tray.rs：`NoticeSeen`（HashSet<String> 身份集）+ `notice_for(..., &mut NoticeSeen)`；unread message / un-opened handoff 按 `msg:<id>`/`hand:<id>` 记身份（缺 id 回退内容字段），只有本帧**新出现**的身份进正文计数；未重复身份静默。
  - lib.rs：SSE EVENT_CHANNEL listener 持有跨帧 `Mutex<NoticeSeen>`（去重状态必须活过帧，不能放 notice_for 内部）。
- 验证：新增 `activity_republish_does_not_rebuzz_known_items`（同身份重发静默 / 新消息只报 1 条新 / handoff 打开后新 handoff 报 1 个）；旧测试迁移新签名。shell tests 33 passed（31+2）；clippy 0 lint；pnpm test 36 passed；pnpm check 0 errors。
- 文档：计划 §8 追加本批；todo.md 增 §177 条目。
- 残余：随下个 tag 走 CI（frontend/Tauri job + 三平台）；§175/§176/§177 三批一起发版。

## 2026-11-20 · §178 node 支撑模块深审（纯审查，零缺陷）
- 范围：crates/tetherly-node/src 支撑层 7 模块全量通读——cred.rs(520)/apitoken.rs(641)/telemetry.rs(307)/sidecar.rs(263)/logsink.rs(261)/lan.rs(223)/ancs.rs(88)。
- 结论：零确认缺陷，全部 confirmed-design。要点：
  - apitoken：tier 数学闭环（client Write < Admin 路径）、V1_ADMIN_PATHS.len()==8 pin 测试、token_matches 空 expected fail-closed、api.token 每启动轮换 + CREATE_NEW + protected DACL、read_client Unix >0600 拒绝。
  - logsink：rename 失败 → 原地截断兜底，硬上限 2×cap；blocked-rename 测试钉死；Windows set_len 陷阱走独立写句柄（注释+测试）。
  - lan：诊断报告与 binder 共用 classify（测试断言 report==bind set）；5s 缓存防 syscall 风暴；bind 拒 0.0.0.0/非 LAN。
  - telemetry：AtomicU64 计数器 + 零负载红线；snapshot saturating 求和；文件通道按设计 tx/rx=0。
  - cred：vault+identity.bin 双权威幂等修复；macOS 默认文件存储；Windows SYSTEM 不进 vault。
  - sidecar：RPC/CLI 双发现均带 probe 超时。
  - ancs：装配层，真机 iPhone 配对 Manual-required 如实标注。
- 定级：本批无 RED/GREEN（零代码变更）；node lib 基线 319。
- 文档：计划 §8 追加；todo.md 增 §178 条目。
- 下一步：v0.2.69 发布（§175/§176/§177 攒批 + §178 审查结论）——版本四文件 bump、tag、CI 门禁。

## 2026-11-20 · v0.2.69 发布 + CI 死代码 lint 热修
- 版本 bump：Cargo.toml workspace / Cargo.lock（cargo update 同步）/ client/package.json / client/src-tauri/tauri.conf.json → 0.2.69；release workflow 校验 tag==三文件版本。
- 首发 tag v0.2.69（1e30c07）触发 CI 后发现：上一轮 main CI 已红——clippy 的 lib-only（无 --tests）构建把 §176 WHEEL_DELTA/wheel_accumulate 与 §175 spawn-claim 判死代码（这些符号只被 Windows hook / windows+macOS cfg 路径引用，测试引用让本地 --tests 检查漏掉了）。macOS/Ubuntu check 双红，Windows 不红（符号真被引用）。
- 热修 73f2721：cfg_attr(not(windows)/not(any(windows,macOS))) allow(dead_code) + 注释锚定原因；本地复核 cargo clippy -p tetherly-node --lib（rc=0，仅遗留 2 个 workspace-manifest 旧 warning）与 --tests（0 error）双过；319 tests GREEN；fmt/diff-check 过。
- tag 重打：取消首发 release run（44753838397，防陈旧 artifact），远端删 tag → 重指 73f2721 → push。
- 教训：本地定向门禁必须包含与 CI 相同形态的检查（lib-only clippy），不能只用 --tests 形态；「测试引用掩盖非测试目标死代码」属系统性盲区。

## 2026-11-20 · v0.2.69 发布完成
- CI v0.2.69（44755134354）六 job 全绿：mac/ubuntu/win check + android + frontend + deny。
- release（44755134347）九 job 全绿：prepare/frontend/build-android/bundle-desktop×2/build-linux/build-desktop×2/publish。
- 正式发布 **Tetherly v0.2.69**（isDraft=false）：12 assets — Tetherly_0.2.69_aarch64.dmg / Tetherly_0.2.69_x64_en-US.msi / tetherly-{macos-arm64,windows-x86_64.exe,linux-x86_64} / tetherly-cli-{三平台} / tetherly-viewer-macos-arm64 / tetherly-android-arm64-v8a.apk / libtetherly_android-arm64-v8a.so / SHA256SUMS。
- main CI（44755121390）success。
- Obsidian 速记已落 `00.MOC/生产变更速记.md`；doc-check PASS 4/4。
- Manual-required 残余（不随本发布阻塞）：Windows 真机滚轮手感（§176 观察项）、Android 15 dataSync timeout/长稳、partial/sidecar 现场观察、双端部署（待用户安排 deploy 窗口）。

## 2026-11-20 · §179 net 层深审（一处 P3 修复）
- 范围：crates/tetherly-net 全 5 模块（filechan 869 / session 750 / dispatch 269 / codec 133 / mdns 125）。
- confirmed-design 要点：session rekey 取消安全 + outbox 进度跨 cancel；pairing trust 后置提交失败回滚；codec TaggedReader 敌意长度不预分配；filechan 三级显式 deadline + AAD 通道位置绑定 + header 即拒 cap；mdns 精确实例匹配。
- 修复（P3）：FILE_CHUNK(core)↔CHUNK(net) 独立常量隐式耦合——const assert 编译期钉死 + 注释说明 fail-closed 后果。
- 验证：net 28 / core 334 tests 全绿，clippy 0 error，fmt/diff-check 过。
- 文档：计划 §8 追加；todo.md 增条目。

## 2026-11-21 · §180 core/input.rs 深审（一处 P2 修复）
- 范围：crates/tetherly-core/src/input.rs 全 2886 行（协议、InputServer/InputClient 状态机、band gate、far-push 回落、30+ 测试）+ 调用方取证（node runtime/input.rs:4414 Leave handler、platform.rs warp-echo gate）。
- confirmed-design 要点：round-to-nearest norm/denorm 无剪切；band_safe_dock_y 泊 band 外（v0.2.26 教训）；wire economy 不发冗余 Move；InputClient seq 窗口去重 + release_all_inputs。
- 缺陷（P2）：`on_remote_leave`（target Leave → controller 收回本地座位的唯一引擎路径）Top/Bottom 泊 y=1/height-1——band 内 + y≤8 into-edge 区；Leave handler 仅 disarm 键盘 hook，鼠标 hook 继续喂 drive_input_delta，stray dy≤0 事件 arm dwell → 120ms 后 re-cross（v0.2.26 ghost-controller 同机制）。force_local/far-push 回落已修，唯此路径遗漏。
- 修复：Top/Bottom 改 band_safe_dock_y 泊位 + x=denorm_x(rem_x) 保位置映射；Left/Right 不动。RED（y=1 断言失败）→ GREEN。
- 验证：core 335（+1）/ node lib 319 回归全绿；clippy 双形态 0 error；fmt 过。
- 文档：计划 §8 追加；todo.md 增条目。

## 2026-11-21 · §181 core/topology.rs 深审（两处修复）
- 范围：topology.rs 全 750 行 + wire 取证（dispatch.rs:168 → lifecycle.rs:1885 → receive_topology_state → frame.sanitize；node input.rs:3989 反向 takeover entry）。
- P2：sanitize 裸 String::truncate(64)——多字节 UTF-8 跨字节 64 边界 panic（独立 rustc 程序证明 is_char_boundary assertion），一个已认证 peer frame 崩 session task。修复：复用 frame.rs truncate_utf8（pub(crate)）。
- P3：map_edge_exit_to_entry 8 个正交/同边 fallback 截断 vs 主分支 round-to-nearest，正交拓扑每次往返剪切 1px。修复：统一 scale()。
- 验证：core 337（+2）/ node 319 / net 13 全绿；clippy 双形态 0 error；fmt 过。
- 文档：计划 §8 追加；todo.md 增条目。

## 2026-11-21 · §182 core/frame.rs 深审（零缺陷，全部 confirmed-design）
- 范围：frame.rs 全 710 行 + 下游取证（clip.rs apply_remote、filexfer.rs offer_resumable、notify.rs ingest_push、dedup.rs、node notify/files/lifecycle 接收路径、前端 activity 轮询）。
- 结论：帧层预算/校验/truncate 规范全闭环；各 wire 结构的下游防线完备。
- P4 观察项（不行动）：NotifyPush uid/app_id/app_name 无 cap；notify hub evict_expired 仅 UI 拉取触发（无周期任务）。rate limiter + TTL 下增长有界，备查。
- 文档：todo.md 增条目。

## 2026-11-21 · §183 core/clip.rs 深审（一处 P3 修复）
- P3：SensitiveContentGuard::is_aws_key_at 字节切片 panic——本地剪贴板含 AKIA/ASIA+多字节文本即崩 auto-sync 轮询（node clipboard.rs:754）。修复：字符边界回退。
- 其余 confirmed-design：preview/commit 分离、echo 防环、apply_remote 顺序、环形历史、guard 其余检测无切片。
- 验证：core 338（+1）/ node 319；clippy 双形态 0；fmt 过。

## 2026-11-21 · §184 core/notify.rs + dedup.rs 深审（零缺陷）
- 全部 confirmed-design；§182 P4 观察项回访后维持不行动（candidates TTL 驱动 / replies 覆盖式，皆有界）。
- 无代码变更，core 基线 338 不变。

## 2026-11-21 · §185 core/filexfer.rs 深审（一处 P3 修复）
- P3：prune 无条件保留 Accepted；auto-accept 下恶意对端 256 个 abandoned accept 饱和 INCOMING_MAX，接收永久瘫痪。修复 accepted_ms + ACCEPTED_TTL_MS=1h。
- confirmed-design：is_safe_transfer_id 白名单、offer 校验序与所有权锁、resume 链（claim 三重校验→start!=hint 硬拒→HKDF AEAD→sidecar 对账）、发送方本地闭环。
- 验证：core 339（+1）/ node check / clippy 双形态 0 / fmt 过。

## 2026-11-21 · §186 core/handoff.rs 深审（一处 P3 修复）
- P3：receive_resource_handoff 无限流 + 每帧 fsync 全 inbox 重写，session 泵可被对端洪水独占。修复 HandoffLimiter 2/s+burst5 入口前置。
- confirmed-design：validator 切片安全（ASCII 分隔符论证）、allowlist fail-closed、SystemOpener 无 shell。
- 验证：core 339 / node 320（+1）/ clippy 0 / fmt 过。

## 2026-11-21 · §187 core/input_tunnel.rs 深审（一处 P2 修复）
- P2：resp/release/close 三处缺 sender 绑定 + 可猜 session_id → 跨 peer 拆除/劫持隧道。修复：resp 双匹配、release/close 仅 peer 授权。
- 验证：3 测试 RED→GREEN；node 323（+3）/ core 339 / clippy 0 / fmt 过。

## 2026-11-21 · §188 core 批量审查（otp/insertion/liveness/keymap，纯审查，零缺陷）
- otp.rs：字节索引安全 by construction；输入由 notify truncate 钳制；打分规则按模块头冻结。
- insertion.rs：plan() 保守语义，无 panic 型操作；reason 静态标签零明文。
- liveness.rs：saturating_sub 防时钟回拨；15s/45s 常量与三连 ping 设计一致。
- keymap.rs：u16 查表闭合；Meta+Any→VK_LWIN 惯例代表值；serde/preset 测试锚定。
- 无代码变更，core 基线 339 不变；不需要 CI 门禁（无 diff）。

## 2026-11-21 · §189 core 支撑模块批量审查（16 模块，纯审查，零缺陷）
- overlay 字节扫描 ASCII 边界 by construction；pin 三级校验后索引；focus 显式 char_boundary 回退；replay wrap-safe；recommend fail-closed 复验；bind classify 单一决策源。
- P4 不行动：locate 注释 bytes=chars；action 多字节单字符超限 ≤3B。
- 无代码变更；不需要 CI（无 diff）。

## 2026-11-21 · §190 core 大模块深审（media/remote_session/camera/drag，一处 P3 修复）
- P3：media command/state 无入口限流（同层 clip/notify/handoff 均有），每帧状态机+事件+audit。修复 MediaLimiter 5/s+burst10。
- confirmed-design：media char_boundary 回退、RS handler §187 peer 绑定、camera chunk 双匹配、drag wire 预算。
- 验证：RED→GREEN；node 324（+1）/ core 339 / clippy 0 / fmt 过。

## 2026-11-21 · §191 core/ancs.rs 深审（纯审查，零缺陷）
- DataAssembler 重组边界全安全；有界性 cap 全测试锚定；协议不变量（订阅序/串行 CP/重试 reset/PreExisting/Removed 清理）锚定。
- P4 不行动：pending/events deadline 约束、received 含重复 attr、write 失败丢请求。
- 无代码变更；不需要 CI（无 diff）。

## 2026-11-21 · §192 core 根目录收尾三模块深审（gesture_map/microphone/audio_routing，两处修复）
- 调用链取证：platform 源自 peer hello / std::env::consts::OS（input_peer_platform → live peer 表；"darwin" 可达 sequence_for，且 "darwin".contains("win")==true）；audio detach 哨兵直接参与 session_id == 比较。
- 修复①（P2）：sequence_for mac/darwin/ios 判断前置 + to_ascii_lowercase；RED（旧逻辑 darwin 得 Alt+Tab）→ GREEN（darwin_platform_routes_to_macos_backend）。
- 修复②（P3）：audio_routing Attach/Detach sanitize 哨兵 "invalid_audio_session" 改 blank 模式（clear()），对齐 remote_session.rs no-sentinel 不变量；RED → GREEN（sanitize_blanks_invalid_session_id_not_sentinel）。
- 验证：core 341（+2）/ node gesture 8 + audio_routing 2 回归绿；clippy 双形态 0 error；fmt 过；CI 提交后权威门禁。
- microphone.rs 全 confirmed-design；core 根目录全部审毕，下一批 screen/（~26k 行分批）。
- CI 44840651065 首跑 Windows job 唯一失败 test_action_runtime_policy_local_allowlist_and_sandbox_execution（5s 轮询窗内子进程历史未落，"never recorded its history entry"）——与本批 diff（gesture_map/audio_routing+docs）无交集，此前 8 连绿、mac/ubuntu 同测试过；--failed 重跑后全绿 6/6，判定 Windows runner 慢 flaky。登记备查：该测试 5s deadline 在慢 runner 上余量不足（tokio spawn + 子进程 echo 全链），后续若再现可加大 deadline 或改事件驱动。

## 2026-11-21 §193 screen/ 协议五件套深审（wire/overlay/sender/receiver/sink+playout）
- 范围：screen/ 第一批 6 文件 ~5500 行全读（wire 1906 / overlay 350 / sender 1634 / receiver 996 / sink 349 / playout 308）；至此 screen 协议三件套 + 重组/呈现链审毕。
- 缺陷（P4，已修）：wire.rs:1831 decode_payload doc 注释 "Header is 24 bytes" 实际 31 字节（代码 const HEADER=31 正确），纯注释过期。
- 关键正面论证：wire decode 31B header + 白名单 + 8MiB decoded cap 防 decode-bomb；assembler TMF1 重组 count==div_ceil 校验防短 count 空洞完成、delivered_seq 水位防 straggler 复活且 reset 保留水位、旧 seq 分片不打断新流；receiver apply seq 窗口容忍 next_seq=1 重启（last_seq!=0 才判跳变）、trace_pairs 区分 sender reset 与 HARQ 重复；on_peer_gone 全复位；playout 迟帧仅在队列>1 丢（保最后一帧呈现）。
- P4 观察项登记：receiver apply profiler 硬编码 stage 值为占位（真实数据走 TYPE_SCREEN_PROFILE，node screen_stream.rs:281/1828 接线在位）；playout 名义间隔 33_333us 对高 fps 略保守（仅 EMA 平滑度）。
- 验证：定向 cargo test -p tetherly-core --lib screen:: 89 全绿；fmt 过；一行注释修复随 CI 门禁。

## 2026-11-21 §194 screen/ 传输链深审（fec/retransmit/pacing，纯审查）
- 范围：fec.rs 698 + retransmit.rs 454 + pacing.rs 554 = 1750 行全读；零代码缺陷，无 diff 无 CI。
- 关键论证：NACK decode 长度自洽（36+count*2 覆盖 rtt）；Cauchy x≠y 恒成立 gf_inv(0) 不可达、奇异矩阵对合法分片不可达；decode_block fail-closed + 8MiB 二次把关；重复分片高斯消元每 block 至多一次（凑齐即交付清空）；RetransmitBuffer age+capacity 驱逐；trendline var_x>1.0 / MIN_SAMPLES≥5 防除零。
- P4 登记：FecAssembler.received_shards 无显式 cap（天然上界 512 条/block、字节 1:1、seq 切换释放，较 TMF1 8MiB 松侧）。
- 验证：纯审查批次无门禁；§193 c3bef04 CI 44845004879 全绿 6/6 确认。

## 2026-11-21 §195–§199 screen/ 收官轮
- 用户指令「最好先收尾下，把能做的做完，能优化的都优化完我来验收」→ 对 screen/ 剩余全部模块连续 5 个审计节点纯审查（§195–§199），零代码缺陷，无 diff。
- 补读 overlay.rs（§193 已审，本轮复核确认 blend/disk/annotation_text 边界完备）。
- 登记：docs/REVIEW_REMEDIATION_PLAN.md §8 新增 5 条（§195/§196/§197/§198/§199）；todo.md 同步。
- screen/ 目录 33 模块 25,904 行全部审毕，等待用户验收。

## 2026-11-21 §200 media.rs P3 修复
- RED 证据固定于 git HEAD 原文；GREEN：test_media_command_frame_decode_sanitizes_hostile_fields + core 342 全绿。
- 登记 plan §8 §200 条目；待 CI 门禁后闭环。
- §201 信任与会话面（session/hello/bind/pin/otp/allowlist/liveness）纯审查收官，零代码缺陷；两项 P4 备查（otp 字节漂移、allowlist scheme fail-open 且取证确认非 wire 可达）。计划 §8 已登记。
- §202 core 根目录剩余 19 模块纯审查收官，零代码缺陷；tetherly-core 全 crate 审毕。计划 §8 已登记。
- §203 tetherly-crypto 全 crate 纯审查收官，零代码缺陷。核心四 crate 全部审毕。

### 2026-11-21 §204 node runtime 会话面深审（纯审查，零代码缺陷，无代码变更）
- 范围：runtime/{trust,clipboard,notify,actions,messages,audit,session,config}.rs 全读（~5.5k 行）。
- 结论：零代码缺陷；2 条 P4 观察项（get_by_id starts_with 前缀匹配；ANCS 泵线程 panic 无自动重启）登记 plan §8 备查。
- 正面论证要点：trust.rs admit_channel side-channel blocklist+expected-device 双门；clipboard.rs 有界投递状态机三测试钉死；notify.rs 抑制链有序 + ANCS 线程隔离；actions.rs 5s 冷却 + kill_on_drop 回归在位；messages.rs 零明文审计 + outbox resend 受控路径复检；audit.rs 有界环；session.rs handshake 后 blocklist 复查；config.rs tray owner keepalive 窗口正确。
- 登记：plan §8 插入 §204 条目（§203 上方）；todo.md 同步。

## 2026-11-21 §205 屏幕面深审收官 + P3 修复
- screen_stream.rs 4979 行全部读完（tile 编码分发/LTR/PIR/SVC/delta/背压整帧丢弃/FEC pacing）。
- 定级并修复 P3：screen_recording_replay（screen_media.rs:153）缺 ParentDir 守卫，镜像兄弟函数补齐 + data_dir 相对解析；新增 uihttp 测试 v1_screen_recording_replay_rejects_traversal，RED→GREEN 证据链（stash 验证）。
- commit 11525b7 已推送，CI run 44869937409；plan §8 已登记。
- 教训重申：plan §8 插入必须用完整上一条标题行作 old_string（§203 教训）。

## 2026-11-21 §206 node runtime 收尾深审收官
- lifecycle.rs 3001 行全读完毕（700-2100 run/attach/handle_event，2100-3001 restart_daemon/preflight/cleanup + 测试）。确认 attach/release channel-identity ownership、keepalive weak sender、peer-down drag 清扫、restart staging 门 + preflight、lan promotion 身份核验全链在位。
- §206 全 13 模块（~10k 行）纯审查零缺陷；plan §8 登记（old_string 用 §205 完整标题行，lesson 复用成功）；.codex-memory 双文件更新。
- node runtime/ 层 35 模块深审至此全部收官：§204（会话面，零缺陷）、§205（屏幕面，1×P3 修复 11525b7 + CI 44869937409）、§206（收尾，零缺陷，3×P4 观察项）。

## 2026-11-21 §207 node 非 runtime 模块深审收官（2×P3 修复）
- 全读 ~15.8k 行：小模块面/store/platform/tray/cursor_shield/screen 两平台/ancs 三平台/apiv1 全家/model/uihttp 全家/viewer。tetherly-node crate 深审全部完成。
- P3-1 applescript 转义（platform.rs）：commit 76c28fb，CI run 44874306386 SUCCESS。
- P3-2 whiteboard import 先读后校验 + model.rs 4×expect panic：commit 9899e50。修复中发现 `?` 不能直接作用于 CoreError（Handled 错误侧是 UiResponse，无 From<CoreError>），改 `.map_err(core_error)?`——新增教训：v1.rs handler 内 CoreError 传播一律走 core_error 映射，勿裸 `?`。新增回归测试 v1_whiteboard_import_rejects_bad_paths_before_read。
- 门禁：clippy 双形态 0 error；uihttp 25 + model 4 + 新增 1 全绿；fmt 过。plan §8 登记（§207 条目置顶于 §206 完整标题行前，lesson 第三次复用成功）；.codex-memory 双文件更新。
- CI 补录：44877084905（9899e50）windows 失败——回归测试用 "/etc/passwd" 当绝对路径，但 Windows 无盘符路径不算 is_absolute()；改为 temp_dir 平台无关绝对路径后 6c81f02 推送，44878772315 首跑 windows phase3 m3_5 "session did not register in time" 时序 flake（与改动无关），rerun success。教训：路径守卫回归测试的绝对路径用例必须用 temp_dir 构造，裸 "/..." 在 Windows 不成立。
## 2026-11-21 §208 loopback+api 深审收官
- tetherly-loopback 与 tetherly-api 两 crate 全读，零 crate 内缺陷。经验：契约 crate（纯 DTO + 少量逻辑面）审计重心应放在仅有的逻辑点（EventEnvelope/Patch<T>/Topic parse）并核对契约测试是否逐一覆盖；CLI→loopback 调用方要单独核查 query 编码纪律，`url_encode` 一律走 core `encode_query_value`（仅 unreserved，/ 与 CRLF 也转义）。
## 2026-11-21 §209 tetherly-net 收官
- 验证性审计经验：对已被 Batch 批次系统性加固过的 crate，深审重点是「逐行确认加固未回归」而非找新问题——outbox 提交/发送两阶段、rekey 取消恢复、filechan header 即拒/AAD 绑定、proptest 面均在位。7 crate（node/core/api/net/android/crypto/loopback）深审全部收官，剩余面为 client/Tauri 前端 TS。
## 2026-11-21 §210 client 面收官
- 前端壳审计经验：Tauri 壳的安全重心在 token 生命周期（Rust 独占 + fragment 即剥）与 WebView 能力最小化（proxy 白名单双守卫）；SPA 审计重心在事件流单写者纪律与 SSE 客户端的 seq 交叉校验。全项目深度 review 迭代（§171→§210）至此全部交付面收官，后续迭代以回归守护与新需求为主。
## 2026-11-21 §211 Android 层收官 + 本机 Android 门禁经验
- Kotlin 层审计经验：self-echo 是 NotificationListener 类 app 的经典缺陷——ingress 首行就该过滤自身包名；node 下游无防线时上游守卫是唯一屏障。
- 本机 Android 门禁踩坑三连：①sdkmanager 12.0 无法解析仓库 XML v4（报「XML 文件結構…」）→ 直接 curl dl.google.com 的 build-tools_r34-macosx.zip + platform-35_r02.zip 解包进 ~/Library/Android/sdk/{build-tools/34.0.0,platforms/android-35}；②gradle 走 maven 下载遇 "Remote host terminated the handshake" → 系统代理 127.0.0.1:7897 需以 -Dhttps.proxyHost/-Dhttps.proxyPort 传给 gradle CLI（GRADLE_OPTS/JAVA_TOOL_OPTIONS 不生效于该路径）；③无 gradle 安装时用 ~/.gradle/wrapper/dists 里已有的 8.13 + Android Studio 自带 JBR（JAVA_HOME）。
- 2026-11-21 §212：scripts/workflows 全读收官。Batch 5 主干（tag checkout/版本契约/immutable publish/Windows rollback 锚点/全量 /api/v1）逐项验证在位；修 4×P4（两处 digest 缺口、sed 锚点、arch gate）+1 注释漂移。教训：release.yml 版本提取 sed 此前靠键序巧合（`.*"` 会吃段内任意带数字引号行）——用 `^version = "` / `^  "version": "` 行首锚；package-macos-app.sh 与 deploy.sh 消费同一 CI 产物但只后者验 digest，「同一产物多入口必须同标准」。
- 2026-11-21 §213：仓库门面收官。教训：README Quick Start 的 `tetherly-cli pair` 从未存在（CLI 无 pair 子命令，配对经 daemon `dial` 走 loopback API）——门面文档的命令示例必须从 cli.rs 枚举反查而非凭记忆；zh 架构图 CJK 框线用 East Asian width 脚本验证，不靠目测。

## §214 docs/ 三文档深审（2026-11-21）
文档面收官：三文档全读 ~1,061 行，逐条对照代码。核心发现：两文档「legacy /api/* 整体删除」表述不准——GET /api/events + /api/events/stream 因 18 个 CLI 调用点消费原生 UiEvent（v1 为投影 EventEnvelope，语义不同）而存活，已补勘误块 + 收尾条件；CONTROL_CLIENT_DEVELOPMENT 状态漂移（设计稿→已上线）已修；FILE_TRANSFER_MODULE 零缺陷。登记 plan §8，提交 + CI 跟踪。

## §215 legacy SSE 收尾条件深审（2026-11-22）
复核 §214 勘误：收尾条件「CLI watcher 切 loopback_v1_events_connect」是伪条件——v1 只有 12 个资源事件，CLI 消费的约 40 种诊断 kind 被 project() 丢弃（Vec::new 臂），invalidated 无载荷且 CLI 零处理。改写勘误：/api/events 定性为常设 CLI 诊断事件通道（Read tier 门禁在位），真实删除前提=诊断事件 v1 契约化。零代码变更。

## §216 诊断事件 v1 契约化（2026-11-22）
落地 §215 确立的真实收尾前提：Event::Diagnostic 契约（flatten 使 wire data={kind,payload}）→ hub 对 ~40 种遥测 UiEvent 逐条透传 → 双树 CLI 36 watcher 切 /api/v1/events（next_diagnostic 展平一层，字段解析零改动）→ 删除 /api/events 路由、handle_sse、SseStream facade、loopback legacy fn。workspace 955 测试全绿，clippy 三门过，svelte-check/vitest 过。§169 遗留的最后一条 legacy 路由清零。

## §217 桌面客户端重做 + v0.2.70 发布（2026-11-22）
按 awesome-ui-kit 规范在独立分支 feat/desktop-client-redesign 重做 Tauri/Svelte 客户端：Tailwind v4 随包打入、玻璃侧栏壳 + ⌘K 命令面板 + 主题切换、kit 组件移植为 Svelte 5（Icon/StatusIndicator/Glass/Switch/ProgressBar/AnimatedCounter/EmptyState/ToastStack/CommandPalette/PageHeader）、九个路由全部重写（文案/testid/托盘契约保留）、新增可视化布局编辑器（/api/v1/topology/*）。Vite 固定产出 daemon 嵌入的三文件。e2e mock server 自 M4 base 改 "/" 后只服务 /next/ 导致 4 用例白屏（接手前即坏），修为同时服务根路径并补 fixture。门禁：svelte-check 0/0、vitest 54、playwright 5、cargo uihttp::next 4、build:ci dist 零差异。教训：dist 必须在最后一次源码改动后重建再提交，否则 build:ci 零差异门会红（本次曾因布局槽位加宽后未重建被门拦下）。Tauri 壳 productName=Tetherly 与 /Applications/Tetherly.app（daemon 包装）同名，壳 dmg 不能直接覆盖安装；新 UI 经 daemon 内嵌 dashboard 上线。版本 bump v0.2.70（含 §179–§216 全部未发布修复）。
- 2026-11-22 §218 macOS QoS 治理（高 CPU 鼠标卡顿修复）：platform.rs raise_daemon_responsiveness mac 实装 + set_current_thread_user_interactive；main.rs on_thread_start 接线；platform.rs tap 线程 + runtime/input.rs 捕获线程接线；3 新测试，lib 331 全绿。

## §219 品牌图标/主题 + v0.2.70 发布失败复盘 + v0.2.71（2026-11-22）
v0.2.70 release run 44951566623：prepare/frontend/android/desktop/bundle 全绿，build-linux 在 §212 新加的「verify binary architecture」步骤挂（debian:bullseye 容器内 `file: command not found`，apt 清单从未装 file；§212 commit f54cb59 首次随 tag 进入 release 才暴露），publish skipped，tag v0.2.70 无 release 资产。发布不可变 → 不动 v0.2.70，修 apt 清单后并入 v0.2.71。同批落地用户要求的 mango 品牌：源图来自 Obsidian `Note/Infra/brand-icon.md`（assets/brand/mac47054639-1024.png 入库，派生 tray-32x32.rgba、menubar-72x72.png），`cargo tauri icon` 重生成 client/src-tauri/icons 全套（bundle + brand.ts logo 直接受益），index.html 内联 data-URI favicon（daemon 只嵌三文件，不能多出静态资源），Tailwind @theme 新增 brand/leaf 色阶并把全部 indigo/violet 工具类替换（浅色文字用 brand-700 保对比度）。托盘：Tauri 壳 GLYPH 由 const 位图改 `*include_bytes!` 品牌图 + tinted_icon 画右下状态徽标（测试同步改）；daemon Windows 托盘用 CreateIcon 在运行时把 RGBA 转 bottom-up BGRA 生成 HICON（零新依赖、失败回退 IDI_APPLICATION），macOS 菜单栏 NSImage::initWithData 18pt + NSCellImagePosition::ImageLeft（仅加 objc2-app-kit `NSCell` feature）。package-macos-app.sh 复制 icon.icns 为 Contents/Resources/Tetherly.icns 并写 CFBundleIconFile。未做：exe 资源图标（需 embed-resource 构建步骤，发布风险换收益不值）、Tauri 壳 dmg 安装（与 /Applications/Tetherly.app 同名冲突仍在）。门禁本地：svelte-check 0/0、vitest 54、playwright 5、tauri cargo test 16、clippy tetherly-node -D warnings 绿（Windows 侧 CreateIcon 由 CI check(windows) 把关）。 追加根因：§217 记的「dist 必须最后重建」其实是 Tailwind v4 自动源扫描把已提交的 client/dist 当类名来源，每次构建都继承上一次产物的工具类（本次 indigo 类在换色后仍残留在 css 里），build:ci 零差异门要两轮才收敛；app.css 加 `@source not "../dist"` 后连续两次构建字节一致，一轮即过。 CI check(windows) 44954239546 红：clippy `useless_vec` 指向 brand_icon 的 `and_mask = vec![0u8; 128]`（只取 as_ptr，应为栈数组）；本机 macOS clippy 不编译 Win32 分支，故只能由 CI 发现——改 `[0u8; SIDE*SIDE/8]` 后重推。
- 2026-11-22 §218-b review 跟进：注释改号、lift_current_thread_user_initiated 收窄 on_thread_start、macOS 报告字段事实化、qos_sys extern 收口 + capture 失败日志。

## §220 v0.2.71 真机深测 → 5 处客户端/壳缺陷 + REST 补水 + v0.2.72（2026-11-22）
v0.2.71 双端部署（health 0.2.71，受保护进程未动）后用 Playwright 直打真 daemon（Mac 127.0.0.1:45716；Windows 经 `ssh -L 45799` 隧道）做全页面走查，发现新 UI 对着真 daemon 概览空白、控制台 `t.devices is not iterable`，诊断页 404。根因五条：(1) SSE `data` 是 `Event` 的 serde tag 外壳（`{"snapshot":{…}}`/`{"device":{…}}`/…），HttpTransport 与 TauriTransport 都没剥壳就喂给 reducer——e2e mock 与单测一直发裸 payload，所以本地全绿；(2) HttpTransport.request 用 `API_PREFIX + path` 而 path 已含 `/api/v1` → `/api/v1/api/v1/...` 404；(3) TauriTransport 把 Rust `WireEvent.seq` 原样转发，store 期望 `id`；(4) Tauri 壳 tray.rs SnapshotSlice 要求 `general`，真快照 `{system,devices,pairing,input}` 无此字段 → 托盘折叠整条丢弃；(5) daemon 订阅首发只有 Snapshot（uihttp/v1.rs:7331），layout/settings/transfers/inbox/activity 仅在变更时推 `*.changed`，新 UI 这五页永久卡在「等待…数据」。修法：transport.ts 加 `unwrapEventData(type,data)`（只在对象恰好只有对应 key 时剥壳；device.removed/invalidated/diagnostic 透传），两端 transport 复用；Rust 侧 events.rs 同步加 `unwrap_payload` 保证壳内托盘/通知拿裸 payload；request 直接 `fetch(path)`；seq→id 映射；`general` 改 `Option + serde(default)`；新增 stores/hydrate.ts `hydrateSections`——每次收到 snapshot 后并行 GET 五个 REST 端点，形状守卫 + 过期判定（期间若已收到该 section 的 `.changed` 则丢弃补水结果）→ 以 `${section}.changed` 合成事件灌 reducer。e2e mock server 改成与真 daemon 一致（首发只 snapshot + 五个 GET 路由），http.test 用抓到的真帧（id 1795300619328017）做回归，flows.test 加「snapshot 触发补水」流程。门禁：svelte-check 0/0、vitest 61、playwright 5、tauri cargo test（parse_frame 5 / tray 16）、clippy -D warnings、fmt。真机复验（本地 dist 经 page.route 注入 + 真 daemon）：Mac/Windows 九页全部有数据、零 console error、零失败请求。教训：mock 必须对齐真 wire 形状（`cargo test` 契约测试只保证 daemon 侧，SPA 侧没有消费真帧的测试）；隧道测 Windows 时 Origin 端口≠daemon 端口会被 origin_allowed 403，需在 Playwright route 里改写 Origin/Referer（测试手段，非产品缺陷）；布局页「本机 1920×1080 缩放 100%」是 topology_layout.json 持久化默认值而非探测值（daemon 有 platform::screen_size 但不回填，待改进）。

## §221 布局页本机分辨率 daemon 自动探测回填 + v0.2.73（2026-11-22）
§220 遗留：`topology_layout.display` 是 topology_layout.json 的持久化默认值 1920×1080@1.0，既显示给用户也被广播给对端并用于 runtime/input.rs 跨屏入口坐标（clamp 入口、SessionTarget 屏幕尺寸、map_edge_exit_to_entry local_display），而真实光标 warp 用 platform::screen_size() → 两者不一致会让对端算出的入口位置错位。落地：platform.rs 新增 `local_display_probe()`（mac 走 CGMainDisplayID/CGDisplayBounds 取 point 尺寸 + CGDisplayPixelsWide 推 scale；Windows 复用 screen_size()，scale 未知；其它平台 None）；store.rs `TopologyLayout.display_manual: bool`（serde default false）；runtime/topology.rs 纯函数 `apply_display_probe(layout, probe)`（手动固定/None 直接跳过，经 DisplayRect::new 走 TopologyValidator 夹取，scale 未知时保留旧值，返回是否变更）+ `probe_local_display()`（cfg(test) 固定 None 保证测试跨平台确定性）+ `Node::topology_refresh_auto_display()`，在 lifecycle 启动加载后、`topology_layout()` 报告前、`topology_broadcast_state()` 广播前调用——刻意不在 input 映射热路径调用（锁 + 系统调用）。契约：`Layout.display_auto: bool`（serde default）、`TopologyLayoutReport.display_auto`、`TopologyConfigPatch.display_auto: Option<bool>`——PATCH 带 width/height/scale_factor 即置 `display_manual=true`，`display_auto:true` 清除并立即重探；两者同时出现时几何胜出。客户端布局页本机框显示「自动检测」/「手动设置」徽标，手动态露出「恢复自动」按钮 → `PATCH {display_auto:true}`。门禁：tetherly-node 335+16+3 测试（含 4 新 auto_display_tests、store roundtrip 含 display_manual）、tetherly-api 43 契约 + bindings 重 bless、clippy -D warnings、fmt、svelte-check 0/0、vitest 62、playwright 5、dist 重建。CI 首推三平台 check 全红：根 tests/phase3.rs 也调 topology_update_config（本机只 grep 了 crates/、只 clippy 了 -p tetherly-node）+ Windows clippy unnecessary_lazy_evaluations（`.then(|| 元组)` 应 `then_some`）——推前必须跑与 CI 同款 `cargo clippy --workspace --all-targets -- -D warnings`，不能用 -p 子集替代。待真机验证：Windows 侧 `GET /api/v1/layout` 若回 1024×768 说明 session-0 服务进程的 GetSystemMetrics 拿不到交互桌面尺寸，需改走 EnumDisplayMonitors/用户会话查询。

- 2026-11-22 §218-c 二轮收尾：worker hook 失败 debug 日志；bins/tetherly-cli 手写 runtime 接线（#[tokio::main(builder=…)] 只收字面量，改 build_cli_runtime + block_on）+ tetherly-node 依赖；qos_sys pub(crate) 统一。门禁 fmt/lib 331/clippy node+cli/build 全绿。v0.2.71 已部署 /Applications/Tetherly.app（原 Mac 端跑的 v0.2.66 是卡顿未消主因），health 回读 0.2.71 + input tunnel 连通。
- 2026-11-22 §218-c CI + 线上验证收口：§218-c 提交推 main 后被另一会话 cherry-pick 为 3a98050（本地 3997a5e 弃用，reset --hard 对齐）；其上 §221 的 CI 失败（phase3 E0061 缺 display_auto 实参 + Windows clippy unnecessary_lazy_evaluations）由另一会话在 7e3a5c1 修复，CI run 44961109218（3a98050）全绿。线上：/Applications/Tetherly.app 已由另一会话升级（bundle 0.2.73，daemon health 0.2.72，07:28 启动），启动日志验证三项全过——① `daemon responsiveness priority_ok=true`（QoS 提升生效）② `throttling_disabled=false working_set_pinned=false`（§218-b 诚实报告字段上线，旧 v0.2.71 假报 true 只在历史日志）③ input tunnel 与 tdev_rguuuxmtz7e2hfpm 正常建立 + auto-reconnect 工作，且无 worker QoS 拒绝提升的 debug 日志。§218 全链路（QoS 治理 + 复审修复 + 部署）收口，余用户真机高负载手感验证。

## §222 布局/设置页控件 busy 期间 disabled 吞键盘操作 → aria-busy（2026-11-22）

- 来源：v0.2.72 真机（Mac 直连 + Windows 经 `ssh -L '[::1]:45716:127.0.0.1:45716'` 隧道）Playwright 写流程走查。边缘环绕 Switch 往返正常；推动阻力 slider 键盘 ArrowRight 15→20 后 ArrowLeft 不生效，API 停在 20（已 PATCH 回 15）。根因：`disabled={busy}` 在 PATCH 在途时禁用了获焦控件，Chrome 会 blur 被 disabled 的元素，下一个按键落到 body。
- 修法：Layout 四个 range + 四个 Switch、Settings 三个 Switch 改 `aria-busy={busy|saving}`（控件保持可聚焦；整页按钮「同步到对端/恢复自动/断开」仍 disabled）。新增 layout.test「keeps sliders and switches focusable while a PATCH is in flight」（MockTransport 挂起 Promise，断言 disabled=false、aria-busy=true、activeElement 不变、同步按钮仍 disabled）+ 新建 settings.test.ts（3 例：switch/name PATCH 精确体 + 空名/同名不发、pending 时 switch 仍获焦、等待态两张卡各一条占位）。vitest 66 / svelte-check 0 / playwright 5 / dist 重建。
- 走查工具教训：Windows 侧浏览器走查若隧道本地端口 ≠ 45716，module script / CSS 的 Origin 是 forbidden header，Playwright route 改写无效 → daemon `origin_allowed` 端口不匹配回 403 `cross-origin request rejected`（curl 不带 Origin 所以 200，误以为产品 bug）。正确做法：隧道绑 `[::1]:45716`（Mac daemon 只占 IPv4 127.0.0.1:45716），浏览器直接打开 `http://[::1]:45716`，Origin 端口天然匹配。

## §223 概览页配对 PIN 卡：未激活时「显示 PIN」打 GET 404（2026-11-22）

- 来源：v0.2.72 真机 PIN 走查（Mac + Windows 同现）：PIN 未激活时点「显示 PIN」→ `GET /api/v1/pairing/pin` 404 `no active pairing PIN`，toast 报错、控制台红；「轮换」才真正生成（201）；「撤销」204 正常。daemon 语义本就是 GET 只读当前 grant、生成走 POST rotate，是客户端按钮语义错位，不改 daemon。
- 修法：Overview `pinActive = $derived(pairing.pairing?.pin_active)`；主按钮未激活时文案「生成 PIN」（key 图标）走 rotate，激活时「显示 PIN」走 GET；轮换/撤销在未激活时 disabled；pairing.changed 报 active→inactive（过期/他处撤销）时清掉已显示的数字（只在 true→false 边沿清，避开生成后事件晚到的竞态）。flows.test 新增「PIN card generates when inactive and shows when active」全流程（生成→显示→关闭）。vitest 67 / playwright 5 / svelte-check 0 / dist 重建。
- 2026-11-22 §218-d 编码重路径移出 runtime worker：encode_pump_outbound 抽取 + FEC spawn_blocking + 探针双槽语义（关键发现：tokio blocking 池与 worker 同名 tokio-runtime-worker 且共享 on_thread_start hook，池身份只能靠闭包位置契约）。测试 m5_1_encode_stages_run_off_the_runtime_workers RED→GREEN。门禁 phase5 19/node lib 335/workspace clippy+tests/fmt 全绿。待：CI+重部署+真机手感。另：workspace 全量测试历史出现过一次 1 failed（数量吻合 phase1），37+ 轮重跑未复现，判定环境级偶发非稳定 flaky。
- 2026-11-22 §218-d 部署收口：f7436c9 CI 44974615830 全绿（含 Windows check）→ bump v0.2.74（2472bba）→ tag 触发 release 44975796589 全绿 → package-macos-app.sh --tag v0.2.74 --restart 部署（SHA256SUMS 校验过）。验证：health ok version=0.2.74（PID 94107，10:44 本地启动）、daemon responsiveness priority_ok=true throttling_disabled=false working_set_pinned=false（§218-b 诚实字段）、input tunnel tun_… accepted peer=tdev_rguuuxmtz7e2hfpm、overlay→LAN promote 正常。§218-d 全链路收口，余真机高 CPU 手感验证。
- 2026-11-22 §218-d 端到端 smoke（双端 0.2.74）：Windows deploy.sh --tag v0.2.74 热重启成功（old_pid 21308→new_pid 11368，版本门过，受保护进程 3/3，SYSTEM）；Windows 侧 consent 需在其本机 CLI screen allow（v1 screen/allow 是被控端本地同意语义，viewer 侧 allow 不影响 host 判定——首次 start 被 "consent stop reply" 拒是预期 ADR-138 路径）。allow 后 start：host streaming 229 帧 / acked 229 / e2e latency 22ms / p95 39ms / FEC auto 30% / trendline underusing；viewer received 173 帧 / rx 462MB / gaps 0 / dropped 0。stop + revoke 干净收场。注：network_health 报 critical 但 trendline underusing + 无丢包（gaps 0），疑似 loss_ratio 采样口径偏保守，留观察不下结论。真机高 CPU 手感验证留给用户。
## §224 控制面板抄 Magpie UI（2026-11-22）

- 来源：用户要求「控制面板和前端深度参考 magpie 的控制面板和前端，抄下 ui」。用 `magpie web` 起侧端口（不碰 :3425 / 不重启托盘 PID 1132）用 Playwright 走了 magpie 全部页面，抓 light/dark 计算样式与 DOM（scratchpad magpie/tokens-*.json、panel.css、shots/），提炼设计语言：`--bg #f4f4f6` 灰底 + 白色 hairline 卡（radius 12、`--line #e3e3e8`）、分组侧栏（muted 组标签 + 白色 pill 选中 + 底部钉住「设置」+ 绿点状态行）、面包屑顶栏（muted 父级 / 粗体当前 + 右侧搜索 pill）、48px `.row` 列表 + `--line-2` 分隔、pill select / 分段控件 / 黑色 iOS toggle、SF/Segoe 13px/1.4、`cubic-bezier(.2,.8,.2,1)` 缓动。品牌色保留 mango 橙 `#f87800`。
- 落地：app.css 全量重写（Magpie token + `:root.dark` + `@layer components` 组件类，去掉 Glass/backdrop 与旧 `@utility`）；App.svelte 壳重写（分组导航 / 钉住设置 / 守护进程地址 / 顶栏面包屑 h1 + `header .link` 契约保留，⌘K 入口改为搜索 pill；导航计数 `aria-hidden` 以保持按钮可访问名 = 纯标签）；PageHeader 不再出 h1（标题在面包屑，避免 e2e `main h1` 双匹配）；Switch/ThemeToggle/StatusIndicator/EmptyState/ProgressBar 换皮；九个路由全部改为 `.list-title` + `.list` + `.row` 结构（Layout 边缘槽/本机框/设备列表/滑杆/策略 Switch 全部 testid、aria-label、`data-edge`、`data-device-chip` 契约不变）。Tauri：主窗 `titleBarStyle Overlay + hiddenTitle`（macOS 原生红绿灯悬浮在侧栏上方，侧栏 `pt-9`；Windows 忽略），`side-head`/`topbar` 加 `data-tauri-drag-region` + `core:window:allow-start-dragging` 权限。
- 门禁：svelte-check 0/0、vitest 67、playwright 5、build:ci（dist 重建两次字节一致）。真机（Mac v0.2.74 daemon + 本地 dist 经 page.route 注入）九页走查零 console error / 零失败请求，light/dark 截图与 Magpie 对照（scratchpad live/mg/）。走查中修两处：flows.test 要求「在线设备（N）」单文本节点，list-title 保持该写法；内容区原 `mx-auto max-w-3xl` 居中与面包屑左缘不齐且留大片空白，改为贴左铺满 `max-w-[1180px]`；`pre.log` 在 `.list` 内加 `max-width:100%` 防右侧裁切。
- 2026-11-22 §224 发版部署收口：CI aa47e34 全绿；bump 906d968 的 check(windows) 首轮红 = `m3_2_peer_gone_returns_cursor` 第三次抖动（8s 上限仍超，bump 零 Rust 改动），rerun 绿；tag v0.2.75 → release run 45012479573 全绿 12 资产；`deploy.sh --target all --tag v0.2.75`：Mac health 0.2.75，home-win NTFS 热替换 old_pid 11368 → new_pid 30196，health 0.2.75，受保护进程 komari-agent/SRBMiner-MULTI/verge-mihomo 前后 3/3 完好；Windows token 已轮换重取。真机走查内嵌面板（不注入本地 dist）：Mac 直连 + Windows 经 `[::1]:45716` 隧道，九页 h1 对齐、零 console error、零失败请求、⌘K 跳转正常。`GET /api/v1/layout`：Mac 1470×956 display_auto=true、Windows 1920×1080 display_auto=true（§221 待验项：Windows session-0 探测得到真实桌面尺寸，非 1024×768）。观察：Mac 探测 scale_factor=1.0 而 Retina 实际 2.0（CGDisplayPixelsWide 在 scaled 模式下可能返回点数），不影响跨屏坐标（入口映射用点尺寸），列为后续小修。
- 收尾补证：`Tetherly_0.2.75_aarch64.dmg` 解包到 scratchpad 本地拉起壳（不安装），`screencapture` 全屏截图确认 macOS Overlay 标题栏：红绿灯悬浮在侧栏 `pt-9` 留白区，品牌行 / 面包屑 / ⌘K 搜索 pill 均未被遮挡；壳进程已 kill，`magpie web` 侧端口已停，`:3425` 仍归 PID 1132。

## §225 菜单栏 UI 抄 Magpie 面板（2026-11-22）
用户反馈：§224 只做了控制面板，菜单栏仍丑，要求重点参考 Magpie 菜单栏 icon 点开的面板。设计源：`magpie web` 侧端口 127.0.0.1:3499 抓取的面板规格（380 宽、46px 头部 18px logo + fw600 品牌 + 36px 图标动作、卡片 radius 10 用 `--card-edge` 阴影代替边框、行 40/36px、文本按钮页脚、面板底 `color-mix(var(--bg) 93%, transparent)` radius 12、菜单栏只放单色 template 字形）。
- **macOS 原生（crates/tetherly-node/src/platform/tray.rs，objc2 0.6 / objc2-web-kit 0.3 新依赖）**：状态项改为 18×18 template 字形（`assets/brand/menubar-template-36x36.png`，mango 芒果轮廓 alpha 图，随系统深浅自动反色）；左键 → 无边框非激活 `NSPanel`（NSPopUpMenuWindowLevel、CanJoinAllSpaces|FullScreenAuxiliary|Transient）内 `NSVisualEffectView`(Popover, 12pt 圆角) + 透明 `WKWebView` 加载 daemon 自带 SPA 的 `#tray&token=…` 路由；右键/Ctrl 点 → 干净文本菜单（运行中 (alias) / 打开控制台 / 辅助功能权限 / 退出）。JS↔native 经 `webkit.messageHandlers.tetherlyTray` 发 JSON：open-main / close / quit / resize / error / toggled。resignKey 自动收起 + 300ms 防重开守卫。**面板高度随内容**：SPA ResizeObserver 汇报 `header+stack+padding+footer`，native 夹到 [200, 640] 后以 `setFrame_display_animate` 顶边锚定缩放（借用先释放再动画，避免 run loop 重入 borrow panic）。
- **SPA（client/src/routes/Tray.svelte 重写 + traybridge.ts + App.svelte）**：`#tray` 识别放宽为 `/^#tray(?:&|$)/`，token 从 fragment 读取后 `history.replaceState` 只保留 `#tray`；`html[data-tray=panel]` 置 body 透明；状态/设备/传输三卡 + 页脚「打开主窗口 / 退出」。状态文案契约不变（tray.test 原 4 例 + 新增 2 例：token 握手与 webkit bridge 投递）。
- **Tauri 壳**：面板 380×468 起步，`resize` 动作 `set_size` 跟随；Windows 菜单字符串去 emoji（"Tetherly · 运行中"/"打开控制台 (127.0.0.1:P)"/"键鼠漫游：活跃"/"退出 Tetherly"），WebView2 面板未做（范围外）。
- 门禁：svelte-check 0/0、vitest 69、playwright 5、vite build、`cargo clippy --workspace --all-targets -D warnings`、`cargo test -p tetherly-shell` 35、fmt。
- 冒烟（隔离第二节点，不动生产 PID 72089）：`./target/debug/tetherly --data-dir <scratch>/live/smoke --name tetherly-smoke run --port 45817 --no-overlay --file-port 45818 --input-port 45819 --screen-port 45820 --ui-port 45899`（45799 被旧 ssh 隧道 PID 25862 占用并实际答 Windows daemon 的 health，勿误用）。AX 读到状态项 868,4 36×24（旧版 91×24 文字项并存）；AX click 走左键路径弹出面板，`screencapture -R` 截到 696,33 380×338 的深色面板，布局与 Magpie 对齐，空态不再留 130pt 空白。注意：本机 `screencapture` 菜单栏行全黑（系统限制），字形只能靠 AX 尺寸 + 肉眼。
- 学习：`WKWebView::loadRequest` 需 feature `WKNavigation`；`initWithFrame_configuration` 需 `objc2-core-foundation`；`NSImage::alloc()` 要 `use objc2::AnyThread`；`NSPopUpMenuWindowLevel` 是安全 static（unsafe 块会触发 unused_unsafe）。
- 2026-11-22 §225 发版部署收口：CI caee762 / bump 41372b3 全绿（无抖动）；tag v0.2.76 → release run 45020240922 全绿 12 资产；`deploy.sh --target all --tag v0.2.76`：Mac health 0.2.76（daemon PID 72089 → 53003），home-win old_pid 30196 → new_pid 23668，health 0.2.76，受保护进程 komari-agent/SRBMiner-MULTI/verge-mihomo 前后 3/3 完好，Windows token 已轮换重取。真机：AX 读到生产状态项 961,4 36×24（旧 91×24 文字项已消失），点开面板 789,33 380×321，内容自适应（1 台在线 DESKTOP-VST7EIE 3ms，传输空态），截图 scratchpad live/panel/prod-panel.png；再点状态项面板收起（windows=0）。`magpie web` 侧端口 3499 已停，`:3425` 仍归 PID 1132。
- 2026-11-22 §218-d 后真机卡顿排查（用户报"还是卡"→"现在不卡了"）：取证显示卡顿发生时 screen 处于 IDLE（无流），§218-d 编码路径根本不在场；input 链路无 stall——engine return push≈required_push（非 shove-hard 症状）、无 queue full、无 WriteStalled、tunnel 每次 accepted、gen 37 会话 10:28:10 干净关闭（controller left roaming）。倾向判定为 Windows 端系统级瞬时负载（属 §218 老链路外因素）。§218-d 编码路径未发现异常，保持观察；如复现优先取 Windows 侧 input apply/enter 日志 + 同秒系统负载对照。

## §226 Windows 托盘面板 + 图标颠倒/发糊修复（2026-11-22）
- 用户反馈：Windows 左键点不开面板（只开浏览器），任务栏图标是"倒着的老图标"。根因两处：① `brand_icon` 用 `CreateIcon` 且把 RGBA 翻成 bottom-up 再喂 XOR 位——`CreateIcon` 的 XOR 位本来就是 top-down，于是上下颠倒；② 只内嵌 32px 一档，任务栏 16px 由系统缩放发糊。修法：`assets/brand/tray-{16,20,24,32,48}.rgba`（LANCZOS 自 1024 源图派生，删 tray-32x32.rgba），`GetSystemMetrics(SM_CXSMICON)` 选最近 ≥ 档，`CreateDIBSection` 负 `biHeight` 预乘 BGRA + 全零 1bpp 掩码 → `CreateIconIndirect`；托盘线程 `SetThreadDpiAwarenessContext(PER_MONITOR_AWARE_V2)`。
- 面板：新文件 `crates/tetherly-node/src/platform/tray_win_panel.rs`（tray.rs 顶层 `#[path]` 声明 `windows_tray_panel`，`windows_tray` 内 `use super::windows_tray_panel as panel`；`#[path]` 在内联 mod 里会嵌目录，故必须顶层声明）。`webview2-com 0.39` + `windows 0.62`（Cargo 别名 `windows062`，与现有 windows-sys/windows 0.59 只在裸 HWND 指针处相遇）。WS_POPUP 工具窗 + DWM 圆角 + 透明 WebView2 加载 `#tray&token=`，`Shell_NotifyIconGetRect` 锚定、工作区推断任务栏方位；`AddScriptToExecuteOnDocumentCreated` 注入 `globalThis.tetherlyTray` 垫片→`chrome.webview.postMessage`；`traybridge.ts` 发射顺序改为 globalThis.tetherlyTray → webkit → Tauri → console。WebView2 缺失一次性降级为开浏览器；右键菜单多 `打开面板`；双击收面板开主界面。
- 校验：本机 mac 不编 Win32 分支，建 scratch crate（scratchpad xcheck，symlink 真源 + assets）`cargo check/clippy --target x86_64-pc-windows-msvc --offline -- -D warnings` 绿。踩坑：windows 0.59 无 `implement` feature；`DwmSetWindowAttribute` 第二参 u32；闭包包在 unsafe 块里触发 unused_unsafe；clippy 1.100 `chunks_exact_to_as_chunks` → `as_chunks::<4>()`。mac 全 workspace clippy 一度红：Tauri 壳 `client/src-tauri/src/tray.rs` 的 GLYPH 仍 include 已删的 tray-32x32.rgba → 改 tray-32.rgba（同格式）。门禁：workspace clippy -D warnings 绿、workspace tests 40 套全绿（含 shell 35）、vite build / svelte-check 0/0 / vitest 69 绿。
- 2026-11-22 §226 发版部署收口：CI 14af404 的 check(windows) 红——`clippy::incompatible_msrv`（`as_chunks` 1.88 起稳定，workspace MSRV 1.82；本机 scratch crate 没写 rust-version 才漏掉，已补）→ fixup e3674c4 改 `chunks_exact`，CI 全绿；bump 4a20260 全绿；tag v0.2.77 → release run 45037127416 全绿 12 资产；`deploy.sh --target all --tag v0.2.77`：Mac health 0.2.77；home-win old_pid 23668 → new_pid 30856（SessionId 1，SYSTEM），health 0.2.77，受保护进程 komari-agent/SRBMiner-MULTI/verge-mihomo 前后 3/3 完好，Windows token 已轮换重取。新 daemon 日志无 tray/icon warn（brand_icon 未回退 IDI_APPLICATION）；WebView2 Evergreen 157.0.4341.60 在位，面板按需懒建（`%LOCALAPPDATA%\tetherly\webview2` 首次左键后才出现）。无远程截图通道：图标朝向/清晰度与左键面板待用户肉眼确认。

## §227 Windows 托盘单色图标 + 面板子进程化（2026-11-23）
- 用户反馈：图标方向对了，但要像 mac 一样黑白；左键点图标报错。报错根因：守护进程以 LocalSystem 跑在 Session 1（`TetherlySystemLauncher`），WebView2 在 SYSTEM 下 `CreateCoreWebView2Controller` → `HRESULT 0x8000FFFF`（WebView2Feedback #1907/#4731/#2030），§226 的进程内面板必败——不是我们的 COM/窗口代码问题。
- 修法（面板）：`tray_win_panel.rs` 重写为两半。守护进程侧 `launcher`（`toggle/hide/is_running/set_data_dir`）：`spawn_as_console_user`（platform.rs 原 `open_as_console_user` 抽成通用 `spawn_as_console_user(app, cmdline, extra_env)`，环境块复制后追加 `TETHERLY_API_TOKEN`）拉 `tetherly.exe --data-dir … tray-panel --ui-port/--anchor-hwnd/--anchor-x/--anchor-y`；非 SYSTEM 用 `std::process::Command`+`CREATE_NO_WINDOW`；`AllowSetForegroundWindow(pid)`；thread_local 持进程句柄，`WaitForSingleObject(h,0)` 判活，`GetProcessTimes` 退出时刻 <300 ms 当"关"；`hide` = `FindWindowW("TetherlyTrayPanel")`→`WM_CLOSE`，没窗则 `TerminateProcess`。子进程侧 `run_tray_panel_process`（cli.rs 隐藏子命令 `Cmd::TrayPanel`，main.rs 在建 tokio runtime 之前接管主线程：`SetThreadDpiAwarenessContext` + `CoInitializeEx(STA)` + 消息泵，dispatch.rs 的臂只 bail "Windows-only"）：原窗口/WebView2 代码照搬，`user_data_dir` 变成用户自己的 `%LOCALAPPDATA%\tetherly\webview2`；`hide` ≡ 退出；`quit` 用裸 TcpStream `POST /api/v1/system/shutdown`（`uihttp/v1.rs:6564`，Admin 档，`X-Tetherly-Token` 头）请守护进程退出；10 s 看门狗 / 150 ms 点击外兜底轮询；WebView2 失败 → 开控制台 + 退出码 2。`spawn_tray` 多第三参 `data_dir`（cmd_core 在 `Node::start` 前 `cfg.data_dir.clone()`）。右键菜单 `打开面板` 恒显示。CLI 层 `TETHERLY_API_TOKEN` 本就被 main.rs 读入 `set_api_token`，子进程里 `api_token_for(port)` 自然回落到它。
- 修法（图标）：`assets/brand/tray-mono-{16,20,24,32,48}.a8`（仅 alpha，scratchpad `gen_mono.py` 自 1024 源图派生，笔画目标 16:1.05/20:1.2/24:1.4/32:1.8/48:2.4 px，首版过度膨胀被否后调低），删彩色 `tray-{16,20,24,48}.rgba`，`tray-32.rgba` 留给 Tauri 壳 GLYPH。`brand_icon(light_taskbar)`：白墨（深色栏）/黑墨（浅色栏）预乘 BGRA；`taskbar_is_light()` 读 `SystemUsesLightTheme`——HKCU，SYSTEM 下读 `HKEY_USERS\<SID>`（`console_user_sid()`：WTSQueryUserToken→GetTokenInformation(TokenUser)→ConvertSidToStringSidW）；`WM_SETTINGCHANGE` → `refresh_icon_for_theme`（翻转才重建 + `NIM_MODIFY` + `DestroyIcon` 旧图）；§6.3 takeover 重 `NIM_ADD` 时也补齐 uFlags/hIcon/tip。Cargo 加 `Win32_System_Registry`。
- 校验：scratch crate（stub `cred::process_runs_as_local_system`/`platform::spawn_as_console_user`/`apitoken::open_client_or_dashboard -> bool`，补 windows-sys features Registry/Threading/Security/Security_Authorization/RemoteDesktop/SystemInformation/Input_KeyboardAndMouse）`cargo clippy --target x86_64-pc-windows-msvc --all-targets -D warnings` 绿（含 `quote_arg` CRT 引号单测）；整包 `cargo check --target msvc -p tetherly-node` 在 mac 上过不去（openh264-sys2/zstd-sys build script 需 msvc cc）→ platform.rs 的 Win32 改动只能靠 CI check(windows) 兜。mac：fmt 过、workspace clippy -D warnings 绿、workspace tests 全绿。

- 2026-11-23 四指切换粘性和弦（用户报：只能等价 alt+tab，没法一直朝一个方向切）：根因是 Windows Alt 松开即提交，旧实现每次滑动发完整 Alt+Tab+Alt-up，连续滑动永远在两窗间横跳。修法（gesture_map.rs 新 StickyChordState + sticky_chord_steps）：首滑 Alt↓(保持) + Tab 点按；续滑只发纯 Tab 点按；反向滑 Shift↓(保持) + Tab；静默 1.2s 后由 1s watcher 的 input_tunnel_status() 携带提交 Alt↑（gesture_chord_commit）；真实按键/点击先强制提交 Alt↑ 再放行（drive_input_key/drive_input_button）；release_input_tunnel_local 丢和弦不加 Alt↑（出站通道随授权消亡）。chord 仅限 Windows 目标的 AppSwitchNext/Prev；其余动作/平台走原 per-swipe 序列且先提交在挂和弦。测试：core 13（5 新增）+ node lib 338（3 新增：续滑纯 Tab、静默提交、真实键先提交；bare-channel 测试改为 chord 语义断言无 Alt-up）。CI 45041000674 绿 → bump v0.2.78（59ea42c）→ release 45042124158 绿 → 双端部署：Mac health 0.2.78、Windows 热重启 30856→32556（health 0.2.78，受保护进程 3/3，SYSTEM）。真机手感（连续四滑同向多窗推进）待用户复测。
- 2026-11-23 §227 发版收口：65b29f6 CI 六 job 全绿；v0.2.78 已被另一会话（四指粘性和弦）占用，故 bump v0.2.79（e52b190）→ tag → release.yml 全绿 12 资产 → `deploy.sh --target all --tag v0.2.79` exit 0：Mac health 0.2.79，home-win 原子热重启 old_pid 32556 → new_pid 32284（SYSTEM，Session 1），受保护进程 3/3 不动，Windows api.token 已轮换重取。日志验证：启动即 `tray icon inked for the taskbar theme light=false`（深色任务栏→白字形）。子进程链冒烟：ssh（Session 0）直接跑 `tetherly tray-panel` 必败 `WebView2 controller 0x80070578`（非交互窗口站，WebView2 拒绝该 HWND）——是环境问题不是 bug；改用 `Register-ScheduledTask -LogonType Interactive -RunLevel Highest` 在 Session 1 以用户身份跑同一命令：`child up` → 230 ms `WebView2 ready` → 约 5 s 后 click-away 正常退出 exit 0。WebView2 初始化期间未见 WA_INACTIVE 误关。待用户真点击覆盖 SYSTEM→`CreateProcessAsUserW` 拉起段（日志 `tray panel: child spawned`）。旧 `GET /api/log/tail` 路由已随 legacy `/api/*` 删除（logging.rs 顶部注释过期），远程看日志直接 PowerShell `Get-Content -Tail`。
- 2026-11-23 §228 面板「打开主窗口」报错：日志证明面板链已通（真点击 `child spawned as_system=true`→`WebView2 ready`），报错出在 `handle_panel_action("open-main")`→`apitoken::open_client_or_dashboard`。远程探针：HKCR/HKCU/HKLM 均无 `tetherly` 类（机上只有 `E:\code\tetherly\target\release\Tetherly.exe`，未装 msi，scheme 从未注册），`start "" tetherly://open` rc=0 但弹 OpenWith 对话框（我在 Session 0 的探针也留下两只 OpenWith.exe，已杀）。修法：`apitoken_win.rs` 注册表门控 + `ShellExecuteW`（避免无控制台子进程起 cmd 闪窗），`open_client_or_dashboard` 重排为 `client_ok` → 回退 dashboard URL。msvc scratch 需把 stub `apitoken` 改成文件模块才能 `#[path]` 挂 `apitoken_win.rs`（内联 mod 内 `#[path]` 会嵌目录）。966c83a + bump v0.2.80 b21cfd7 推 main，等 CI。
- 2026-11-23 §228 发版收口：CI 45053142761 绿 → tag v0.2.80 → release 45054109452 绿（12 资产）→ deploy all exit 0（Mac 0.2.80；home-win 32284→8360 SYSTEM Session 1，受保护进程 komari-agent 6920 / SRBMiner-MULTI 23580 / verge-mihomo 21272 不动）。「打开主窗口」真机效果（Chrome 开 `http://127.0.0.1:45716/#token=…`，不再弹「需要新应用」）只有用户点击能覆盖——`open_client_or_dashboard` 没有独立 CLI 入口。若将来装了 msi（注册 `tetherly://`），深链路径自动恢复。
- 2026-11-23 §228 四指切换模式选项化（用户纠偏：默认必须仍是逐次 Alt+Tab 直切，连续切换做成可选）：`AppSwitchMode` 枚举（`per_swipe` 默认 / `sticky_chord` 可选，serde snake_case + `#[default]` 派生），随 gesture_map.json 持久化，旧文件缺字段自动回落 per_swipe（`legacy_map_without_mode_defaults_to_per_swipe` 测试）。新增 `GET/PATCH /api/v1/input/gesture`（Write tier，`GestureMapDto`/`GestureMapPatch` 全 String 字段 + deny_unknown_fields，ts-rs 绑定已重新 bless），`Node::set_gesture_map` 落盘 + 换内存 map（切换模式时丢弃在挂和弦不注入 Alt↑）。`drive_gesture_swipe` 按 `map.app_switch_mode` 分流：sticky 且 Windows 目标走和弦状态机（含 Shift 保持反向续滑），否则先提交在挂和弦再走原 per-swipe 序列。前端：client.ts 加 `gestureMap()`/`patchGestureMap()`，Settings.svelte 自取自更（不进 settings 事件流）加「手势 → 四指滑动连续切换」开关，副文案说明仅 Windows 宿主生效。门禁：fmt/clippy -D warnings 绿（clippy 修了 derivable_impls 手写 Default → `#[derive(Default)]`+`#[default]`、field_reassign_with_default 测试写法）、core gesture 15 / node gesture 13 / node lib 340 全绿、svelte-check 0/0、vitest 69（settings.test 两例适配：PATCH 断言加 gesture GET 首调、pending 用例按 method 过滤）。CI 首轮 frontend 红 = committed dist 未随源码重建（CI 端 `test -z "$(git status --porcelain -- dist)"` 验证失败）→ `pnpm build` 重出 dist 后 fixup 9a53654 绿。CI 45062227860 绿 → bump v0.2.81 bc04f8f（0.2.78/79/80 已被占用）→ tag/release/双端部署待续。
- 2026-11-23 §229 Windows「键鼠在本机」反了：根因 `Input.seat` 节点视角（controller remote=光标出去；target remote=controller 光标进来），前端按 controller 语义渲染 → target 全反。修：DTO 加 `role`+`cursor_here`（daemon 折叠 role+seat），api.ts 重生成，Tray/Overview 按 `cursor_here` 标注且 peer id 解析为设备名，收回仅 controller。TDD 红→绿，dist 重建，1ebd600 推 main。另一会话已占 v0.2.81（手势模式），本次 bump v0.2.82（c2b15ac）→ CI/release 全绿 12 资产 → deploy all exit 0：双端 health 0.2.82；events 快照 Mac input role=controller cursor_here=true，home-win role=target cursor_here=false（daemon PID 18608→26632 SYSTEM Session 1，受保护进程 3/3）。待用户肉眼确认 Windows 面板改显「键鼠在 <mac名>」。
- 2026-11-23 §228 代码审查根因修复（用户要求严格 review 后根因修复所有问题并部署）：review 发现并修复全部 6 项——① **P1 ABBA 死锁**：`input_tunnel_status` 静默提交原本持 chord 锁嵌套 engine 锁，而 `drive_input_key/button/swipe` 全持 engine→chord 序，滑完停手即打字每秒一次撞车窗口，双线程互锁冻结 hook 线程；修法为严格两段式——先在 chord 锁内 `take_commit()` 取出 ups 并**释放锁**，再在 engine 锁内构建 key-up 事件（take_commit 一次性语义保证无重复/丢失）。② **P2 方向翻转单向**：`sticky_chord_steps` extending 分支原本只加 Shift 不移除，反向滑一次后前向永远倒着切；补 `!shift && shift_held` 分支——从 held 移除 Shift、push Shift-up（在 Tab down 之前），core 新增 `sticky_chord_flip_back_to_forward_releases_shift` 断言。③ **P2 PATCH 400 化**：gesture PATCH 校验错误由 `CoreError::Json`（core_error 映射 500）改 `InvalidConfig`（400），uihttp.rs 新增真实 HTTP 路由契约测试（合法 patch 200+GET 回读、非法 mode/action/未知字段各 400、失败 patch 不改状态）。④ **P3 注释修正**：`set_gesture_map` 文档改成与 `set_input_policy` 一致的「persist first, swap second」。⑤ **P3 button 提交单测**：新增 `test_gesture_sticky_chord_commits_before_real_button`（Alt-up 先于 button down）。⑥ **P3 杂项**：模式切换时把在挂和弦 best-effort 注入提交（消除 Windows overlay 尾巴，失败静默），删除无调用方 `AppSwitchMode::describe()`；`test_gesture_mode_switch_changes_next_swipe` 相应断言切换时先收到 commit Alt-up。门禁：fmt/clippy -D warnings 绿、core gesture 16、node gesture 15、node lib 342（含新 button + uihttp 路由测试）、bindings、svelte-check 0/0、vitest 69 全绿。CI 45069951808 绿 → bump v0.2.83（30b741d，v0.2.82 已被 §229 占用）→ tag v0.2.83 → release 45071980640 全绿 12 资产 → `deploy.sh --target all --tag v0.2.83` exit 0：Mac health 0.2.83；home-win 热重启 old_pid 26632 → new_pid 22728（SYSTEM，Session 1），受保护进程 komari-agent 6920 / SRBMiner-MULTI 23580 / verge-mihomo 21272 前后 3/3 不动。
