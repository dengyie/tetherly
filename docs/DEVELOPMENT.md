# Tetherly 开发文档

| 项 | 值 |
|---|---|
| 产品名 | **Tetherly** |
| 文档版本 | **1.68** |
| 日期 | 2026-10-19 |
| 状态 | **v1.68 跨端发件箱生命周期治理、发送历史可观测性、传输状态追踪与安全审计清除全闭环（ADR-061）**——(1) **发端与收端结构性不对称破局与发件箱生命周期模型 (`OutboxStatus`, `OutboxEntry`, `crates/tetherly-node/src/runtime.rs` & `lib.rs`)**：定义强类型发送状态（InFlight / Completed / Rejected / Failed / Cancelled），建立完整发件箱条目模型（outbox ID、transfer ID、目标 peer 及别名、文件名、文件大小、SHA-256、本地文件源路径、状态、错误描述、起止时间戳、速率）；(2) **定额内存环形发件箱引擎 (`OutboxEngine`, `crates/tetherly-node/src/runtime.rs`)**：构建定额 200 条内存环形缓冲区，支持 $O(1)$ 均摊压入、按 transfer ID 逆序匹配状态更新 (`update_status`)、多维检索过滤 (`query`: peer, status, limit)、按条件或全量清空 (`clear`) 与基于 ID 逆向定位 (`get_by_id`)；(3) **一键重发流水线 (`Node::resend_outbox_file`)**：基于发件箱历史记录验证物理源文件存在性，一键自动重新发起跨端文件提议 (`offer_and_send_file`)，打通传输提议复用断层并输出安全审计轨迹；(4) **Zero-Plaintext 零明文事件总线可观测性 (`crates/tetherly-node/src/events.rs`)**：新增 `UiEvent::OutboxUpdated` 与 `UiEvent::OutboxCleared` 零明文事件变体，仅广播传输元数据与状态码，杜绝文件正文与私钥凭据外泄；(5) **回环控制面端点全闭环与 403 强隔离防御 (`crates/tetherly-node/src/uihttp.rs`)**：新增 `GET /api/file/outbox`、`POST /api/file/outbox/query`、`POST /api/file/outbox/clear`、`POST /api/file/outbox/resend`，严格执行 127.0.0.1 物理绑定与 Host 403 白名单拦截；(6) **双端 CLI 完整同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**：新增 `outbox`（别名 `out`）子命令族（`list`、`clear`、`resend`、`watch`），提供格式化表格渲染、状态筛选、一键重发与 SSE 实时更新监听；(7) **Web UI 发件箱治理面板 (`ui/index.html`)**：增设「已发送文件与发件箱治理 (Outbox)」独立面板，支持状态/对端下拉过滤、卡片流高亮展示、一键重发、一键清空与 SSE 实时联动；(8) **端到端集成测试与全量安全基线全绿**：`tests/phase1.rs` 增设完整集成测试验证发件箱生命周期、接收端确认/拒收状态流转、重发流水线、多维过滤检索、安全清空治理、SSE 零明文与 Host 403 恶意阻断，全工作区测试全绿、Linux 交叉编译、Clippy 零告警与 Cargo Deny 100% 达标。+ v1.67 跨端点对点安全文本/便签即时交互通道、离线消息缓冲、消息历史治理与双向全闭环（ADR-060）+ v1.66 节点安全审计轨迹、环形结构化审计日志、多维安全检索与遥测全闭环（ADR-059）+ v1.65 对端加密连接保活心跳治理、动态心跳间隔调谐、多维链路质量画像与主动探活遥测全闭环（ADR-058）+ v1.64 节点系统资源深度遥测、进程级性能画像与运行时健康度量闭环（ADR-057）+ v1.63 跨端桌面原生系统级通知推送、横幅弹窗治理与免打扰闭环（ADR-056）+ v1.62 跨端收件箱文件内容提取、流式下载、系统级直开与多媒体预览全闭环（ADR-055）+ v1.61 局域网设备自发现治理、信标可观测性与一键拨号配对闭环（ADR-054）+ v1.60 跨端收件箱生命周期治理、已接收文件多维检索与安全审计清除全闭环（ADR-053）+ v1.59 跨端多文件批量发送提议、断点续传可观测性画像与传输治理全闭环（ADR-052）+ v1.58 Linux BlueZ D-Bus GATT 传输栈架构与跨平台三端 ANCS 全闭环 (Windows / macOS / Linux)（ADR-051）+ v1.57 macOS ANCS CoreBluetooth 传输栈架构、跨平台外设地址抽象与守护进程链路修复（ADR-050）+ v1.56 ANCS 苹果通知服务全生命周期治理、白名单规则动态增删查、状态机事件总线闭环与双端 CLI 全同构（ADR-049）+ v1.55 网络拓扑全景透视、网卡物理分类动态重探治理、EasyTier 侧车穿透可观测性与事件总线闭环（ADR-048）+ v1.54 远程屏幕与键鼠共享全生命周期治理、实时遥测可观测性与事件总线闭环（ADR-047）+ v1.53 跨端文件传输提议生命周期治理、轻量级提议透视与终端全同构控制闭环（ADR-046）+ v1.52 跨端剪贴板生命周期治理、元数据遥测审计与历史可观测性闭环（ADR-045）+ v1.51 通知全生命周期治理、跨端双向消除同步与通知历史可观测性闭环（ADR-044）+ v1.50 对端黑名单访问控制、恶意连接防御与动态接入鉴权治理（ADR-043）+ v1.49 对端连接会话全域治理、主动按需直连与优雅断开控制闭环（ADR-042）+ v1.48 对端能力全景透视、多维特性协商与动态功能边界治理（ADR-041）+ v1.47 全协议网络流量透视、多通道遥测度量与运行时可观测性闭环（ADR-040）+ v1.46 物理寻址路由表全景透视、静态路由注入与断线自动重连治理（ADR-039）+ v1.45 节点运行配置全景透视、动态运行时特性开关与轻量级配置回环治理（ADR-038）+ v1.44 对端设备别名治理、友好命名持久化与对端标识智能解析（ADR-037）+ v1.43 配对控制面治理、PIN 码安全生命周期管理与 CLI 拨号配对全同构（ADR-036）+ v1.42 文件传输通道全景可观测性、轻量级传输查询与命令行同构治理（ADR-035）+ v1.41 对端加密通道往返时延 (RTT) 实时测速、网络质量度量与主动链路探测（ADR-034）+ v1.40 节点事件总线实时流式推送 (SSE)、实时事件可观测性与 CLI 事件监听闭环（ADR-033）+ v1.39 节点健康检查探针、轻量级守护进程可观测性与优雅停止机制（ADR-032）+ v1.38 节点数据目录全景可观测性、存储配额分析与孤儿分块垃圾回收（ADR-031）+ v1.37 ANCS 苹果通知服务深层可观测性、白名单规则治理与命令行回环全同构（ADR-030）+ v1.36 网卡物理分类透视、EasyTier 侧车穿透诊断与网络探测回环闭环（ADR-029）+ v1.35 对端拓扑全景可观测性、物理路由缓存透视与轻量级检索（ADR-028）+ v1.34 远程屏幕与键鼠通道全域控制、定向拨号与无头命令行交互闭环（ADR-027）+ v1.33 通知候选生命周期全域治理、OTP 命令行安全提取与双向快速回复闭环（ADR-026）+ v1.32 跨端安全剪贴板主动同步、回环控制与全端闭环（ADR-025）+ v1.31 信任关系主动解绑与设备安全生命周期治理（ADR-024）+ v1.30 传输生命周期全域管控与会话级度量（ADR-023）+ v1.29 全双工传输任务取消机制（ADR-022）+ v1.28 64 GiB 上限与全景可观测性（ADR-021）+ v1.27 大文件传输实时追踪与 Web UI 进度条（ADR-020）+ v1.26 流式大文件引擎（$O(1)$ 内存界、64 KiB 块、增量 SHA-256）+ v1.25 文件发送生产闭环（/api/file/send 与 CLI send）+ v1.24 文件大小真门与网卡分类真理源 + v1.23 WinRT BLE GATT ANCS + v1.22 帧预算推导 + v1.21 背压与存活检测 + v1.20 CLI 回环客户端 + v1.19 多端采集 + v1.18 APK 闭环 + v1.8 DXGI 远程屏幕 |
| crate / 二进制 | `tetherly-core`、`tetherly-crypto`、`tetherly-net`、`tetherly-node`（`tetherly` 二进制）、`tetherly-cli`、`tetherly-android`（`libtetherly_android.so` + `android_probe`） |

本文是实现合同。与代码冲突时改文档或改代码，必须一致。旧名 InterLink 作废。

**1.68 相对 1.67(跨端发件箱生命周期治理、发送历史可观测性、传输状态追踪与安全审计清除全闭环)**

- **动机(发端与收端结构性不对称痛点、发送历史黑盒与一键重发诉求)**:
  在 Tetherly 陆续完成跨端点对点安全文本/便签即时交互通道 (v1.67 / ADR-060)、节点安全审计轨迹 (v1.66 / ADR-059)、对端连接保活与链路画像 (v1.65 / ADR-058) 及收件箱生命周期管理 (v1.60 / ADR-053) 后，底层通信与接收端文件管理已高度完善。然而，在**文件传输子系统（File Subsystem）**维度，系统存在严重的“发端与收端结构性不对称（Inbox vs Outbox Asymmetry）”痛点：
  1. **发件箱与发送历史黑盒（No Outbox & Sent Files Observability）**：
     收件箱端具备完整的磁盘文件扫描与治理功能（`GET /api/file/inbox`, `download`, `raw`, `open`, `delete`, `clear`）；但在发送端（Sender Side），当用户通过 `tetherly send` 或 `POST /api/file/send` 发起文件传输后，缺乏发件箱模型或持久化发送历史记录；仅有的 `active_transfers` 是保留最近 64 条的临时记录，无法系统追踪：曾向哪些设备发过什么文件？文件大小与 SHA-256 为何？对方是否成功接收或拒绝？耗时多久？
  2. **缺乏发件箱多维检索与清除治理（No Outbox Query & Clear Governance）**：
     缺乏回环控制端点以按目标对端（`peer`）、传输状态（`status`: Completed / Failed / Rejected / Cancelled / InFlight）、条数上限（`limit`）进行结构化查询与过滤，缺乏清空或按条件重置发件箱历史的治理原语；
  3. **一键重发与传输提议复用断层（Resend / Re-offer Gap）**：
     当文件发送失败、被对端拒收或因网络抖动中断后，用户必须重新在终端中定位原始文件物理路径并重新输入命令。发件箱应记录原始文件路径（若为本地物理文件），并提供一键重新提议发送流水线；
  4. **Zero-Plaintext 零明文安全铁律（Zero-Plaintext Invariant）**：
     发送状态流转必须向事件总线分发实时事件（`UiEvent::OutboxUpdated`, `UiEvent::OutboxCleared`），仅广播传输 ID、对端设备号、文件名、大小、状态与时间戳，严禁泄露文件正文或私钥凭据。

- **架构与协议设计(ADR-061 跨端发件箱生命周期治理、发送历史可观测性、传输状态追踪与安全审计清除全闭环)**:
  1. **发件箱状态与条目数据模型 (`OutboxStatus`, `OutboxEntry`, `crates/tetherly-node/src/runtime.rs` & `lib.rs`)**:
     - 强类型发送状态枚举 `OutboxStatus`：`InFlight`（传输中）、`Completed`（传输完成）、`Rejected`（对端拒收）、`Failed`（传输失败）、`Cancelled`（已取消），提供 `as_str()` 与 `FromStr` 支持；
     - 强类型发件箱条目模型 `OutboxEntry`：`id`（唯一发件记录 ID `out_{transfer_id}_{now}`）、`transfer_id`（内容寻址传输 ID `tr_{sha256[..16]}`）、`peer`（目标设备 ID）、`peer_alias`（目标设备别名缓存）、`filename`（安全文件名）、`size`（文件大小）、`sha256`（SHA-256 十六进制摘要）、`source_path`（本地物理源文件路径）、`status`（当前状态）、`error`（错误原因）、`started_ms`（发起时间戳）、`completed_ms`（完成时间戳）、`speed_bps`（测算吞吐速率）；
  2. **定额内存环形发件箱引擎 (`OutboxEngine`, `crates/tetherly-node/src/runtime.rs` & `lib.rs`)**:
     - 定额环形缓冲区 `OutboxEngine`（默认容量 200 条，`VecDeque<OutboxEntry>`）：保证 $O(1)$ 内存界限与均摊压入，超出上限自动置换最旧记录；
     - 状态逆序流转更新 `update_status`：按 `transfer_id` 从最新记录逆向匹配更新状态、完成时间与速率；
     - 时间序多维检索 `query`：支持按 peer、按 status、限制 limit 进行快速过滤；
     - 条件安全清除 `clear`：支持按 peer 与 status 进行精确清理或全量清空；
     - 逆序定位 `get_by_id`：支持根据发件记录 ID 前缀或 transfer_id 快速检索条目；
     - 节点运行时原语：`Node::outbox_history(peer, status, limit)`、`Node::clear_outbox(peer, status)`、`Node::resend_outbox_file(id_or_transfer_id)`；
  3. **一键重发提议流水线 (`Node::resend_outbox_file`, `crates/tetherly-node/src/runtime.rs`)**:
     - 解析历史记录定位 `source_path`，校验磁盘物理文件完整存在性；
     - 自动复用文件名与对端映射，调用 `offer_and_send_file` 发起全新传输提议；
     - 记录 `AuditCategory::File` 下 `outbox_resent` 审计事件，实现端到端安全审计闭环；
  4. **Zero-Plaintext 零明文事件总线 (`crates/tetherly-node/src/events.rs`)**:
     - 扩展 `UiEvent`：新增 `OutboxUpdated { id, transfer_id, peer, filename, size, status, timestamp_ms }` 与 `OutboxCleared { count, peer, status, timestamp_ms }` 零明文事件变体；
     - 审计埋点：记录 `AuditCategory::File` 下 `outbox_staged`、`outbox_completed`、`outbox_rejected`、`outbox_cleared`；
  5. **回环控制面端点全闭环与 403 强隔离防御 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/file/outbox`（别名 `/api/outbox`）：支持 URL Query 参数 `peer`, `status`, `limit`；
     - `POST /api/file/outbox/query`（别名 `/api/outbox/query`）：接收 JSON 请求体执行多维查询；
     - `POST /api/file/outbox/clear`（别名 `/api/outbox/clear`, `DELETE /api/file/outbox`）：条件/全量清空发件箱；
     - `POST /api/file/outbox/resend`（别名 `/api/outbox/resend`）：接收 `{"id": "..."}` 重新发起文件提议；
     - 严格执行 127.0.0.1 物理绑定与 `Host: 127.0.0.1|localhost` 403 Forbidden 强隔离防御；
  6. **双端 CLI 完整同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - 新增 `outbox`（别名 `out`）子命令族（`list`、`clear`、`resend`、`watch`）；
     - `list`：表格化对齐输出 `ID`, `FILENAME`, `SIZE`, `PEER`, `STATUS`, `STARTED`, `SPEED`；
     - `clear`：支持 `--peer` 与 `--status` 筛选清除；
     - `resend`：接收条目 ID 或 transfer ID 一键重新提议；
     - `watch`：通过 SSE 实时流监听发件箱更新事件；
  7. **Web UI 发件箱治理面板 (`ui/index.html`)**:
     - 增设「已发送文件与发件箱治理 (Outbox)」独立面板；
     - 提供状态下拉筛选（全部 / 进行中 / 已完成 / 已拒收 / 失败 / 已取消）与对端实时过滤；
     - 文件卡片流展示安全状态徽标、目标对端、发起时间、耗时与传输速率；
     - 提供「重新发送」一键提议交互、「刷新」与「清空发件箱」；
     - 深度接入 SSE `outbox_updated` 与 `outbox_cleared` 实时局部刷新；
  8. **端到端集成测试与全量安全基线全绿**:
     - `tests/phase1.rs` 增设 `node_outbox_file_lifecycle_query_clearing_resend_and_zero_plaintext_via_loopback_api`：
       覆盖发件箱初始化、文件发送后自动登记 InFlight、对端接收后流转 Completed、对端拒收后流转 Rejected、多维状态与对端过滤、一键重发流水线与新提议生成、部分状态清除与全量清空、SSE 零明文事件校验、以及 Host 403 恶意阻断。

**1.67 相对 1.66(跨端点对点安全文本/便签即时交互通道、离线消息缓冲、消息历史治理与双向全闭环)**

- **动机(跨设备即时文本交互缺失、剪贴板污染痛点与轻量交互闭环诉求)**:
  在 Tetherly 陆续完成节点安全审计轨迹与多维检索 (v1.66 / ADR-059)、对端连接保活与链路画像 (v1.65 / ADR-058)、节点系统资源深度遥测 (v1.64 / ADR-057) 与跨端通知横幅治理 (v1.63 / ADR-056) 后，节点间已具备完备的底层安全与监控能力。然而，在**日常设备间点对点文本与便签交互（Direct Peer-to-Peer Secure Messaging & Notes）**维度，系统面临以下核心痛点：
  1. **跨设备即时沟通与文本传递严重依赖剪贴板同步（Clipboard Sync Pollution）**：
     此前用户在跨设备传递一段网址、便签、代码片段或问候语时，只能强行触发全局剪贴板同步（`tetherly clip copy`）。这会直接覆盖对端用户的系统剪贴板，打断对方正在进行的复制粘贴工作流，造成剪贴板污染，体验极差；
  2. **通知单向推送与快速回复的局限性**：
     既有通知系统仅针对手机推送至电脑的通知进行回复（Quick Reply），无法由桌面端主动向对端发送非通知类即时便签或文字；
  3. **缺乏定额内存环形历史记录与未读状态流转（Message Engine Governance）**：
     收发的消息必须在节点内存中具备结构化留存，既要支持离线查看历史会话，又要严格防范内存膨胀（Memory Bound Invariant），同时提供直观的未读标记治理；
  4. **Zero-Plaintext 零明文安全铁律（Zero-Plaintext Invariant）**：
     SSE 事件总线与公共日志严禁泄露任何消息明文正文，仅允许广播 ID、对端设备号和字符长度；文本明文仅能通过受 127.0.0.1 及 Host 白名单强隔离保护的回环端点获取。

- **架构与协议设计(ADR-060 跨端点对点安全文本/便签即时交互通道、离线消息缓冲、消息历史治理与双向全闭环)**:
  1. **协议帧与载荷模型 (`TYPE_TEXT_MESSAGE: 0x0202`, `crates/tetherly-core/src/frame.rs` & `lib.rs`)**:
     - 新增 Inner Frame 类型 `TYPE_TEXT_MESSAGE: u16 = 0x0202`；
     - 强类型载荷模型 `TextMessage`：`id: String` (如 `tmsg_<uuid>`)、`timestamp_ms: u64`、`sender_id: String`、`text: String`；
     - 编码规范：`[id_len: u8][id_bytes][timestamp_ms: u64_be][sender_len: u8][sender_bytes][text_bytes]`；
     - 严格单帧上限：`TEXT_MESSAGE_MAX = 32 * 1024` (32 KiB)，确保在 Noise Transport Plaintext 65,519 字节物理上限内原子单帧交付；
  2. **定额环形消息缓冲引擎 (`MessageEngine`, `crates/tetherly-node/src/runtime.rs` & `lib.rs`)**:
     - 抽象强类型消息模型 `PeerMessage`（`id`, `peer`, `direction`: `Inbound` / `Outbound`, `text`, `timestamp_ms`, `read: bool`）；
     - 定额环形缓冲区 `MessageEngine`（默认容量 200 条，`VecDeque<PeerMessage>`）：提供 $O(1)$ 均摊压入 `push`、时间序多维检索 `query`（支持按 peer 过滤、未读过滤 `unread_only`、条数限制 `limit`）、原子批量已读标记 `mark_read`、按对端或全量清空 `clear`；
     - 节点运行时原语：`Node::send_message_to_peer(peer_target, text)`（支持对端别名/ID解析，当仅有一个对端时自动解析目标）、`Node::message_history(peer, limit, unread_only, mark_read)`、`Node::clear_messages(peer)`；并在 `Snapshot` 中暴露实时 `unread_messages_count` 统计；
  3. **Zero-Plaintext 零明文安全红线与事件总线 (`crates/tetherly-node/src/events.rs`)**:
     - 扩展 `UiEvent`：新增 `MessageReceived { id, peer, len, timestamp_ms }` 与 `MessageSent { id, peer, len, timestamp_ms }` 零明文事件变体；
     - 严格审计埋点：记录 `AuditCategory::Peer` 下的 `message_sent`、`message_received`、`message_cleared` 审计事件，仅记录元数据与长度，恪守零明文铁律；
  4. **回环控制面端点全闭环与 403 强隔离防御 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `POST /api/message/send`（别名 `/api/msg/send`）：发送即时便签或消息；
     - `GET /api/message/history`（别名 `/api/message/list`, `/api/msg/history`）：支持 `peer`, `limit`, `unread_only`, `mark_read` 查询参数；
     - `POST /api/message/query`：接收 JSON 请求体执行高级过滤；
     - `POST /api/message/clear`：安全清空当前消息；
     - 严格执行 127.0.0.1 物理绑定与 `Host: 127.0.0.1|localhost` 403 Forbidden 强隔离防御；
  5. **双端 CLI 完整同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - 增设 `tetherly message` / `tetherly msg` 子命令族：
       - `message send [--peer <target>] <text>`：发送安全便签；
       - `message history/list [--peer <target>] [--limit <n>] [--unread] [--mark-read] [--json]`：查询并渲染气泡格式消息列表；
       - `message clear [--peer <target>]`：清空消息缓冲；
       - `message watch [--peer <target>]`：实时监听接收与发送事件；
     - SSE `events` 监听器中提供 `[message_received]` 与 `[message_sent]` 高亮格式化输出；
  6. **Web UI 跨端即时对话与便签面板 (`ui/index.html`)**:
     - 增设「跨端即时消息与安全便签通道」独立面板，支持动态选择对端、未读过滤、消息数量徽标统计；
     - 采用安全防 XSS 转义的对话气泡布局（发送方深绿右对齐、接收方青蓝左对齐），提供多行文本发送框与一键刷新/清空交互；
     - 接入 SSE `message_received` 与 `message_sent` 实时刷新会话流；
  7. **端到端集成测试与全量安全基线全绿 (`tests/phase1.rs`)**:
     - 增设 `node_direct_secure_messaging_history_governance_and_zero_plaintext_via_loopback_api` 集成测试；
     - 验证两端加密直发、Noise 传输接收、未读标记流转与快照统计、双向交互、多维查询、清空治理、SSE 零明文校验与 Host 403 恶意阻断。

**1.66 相对 1.65(节点安全审计轨迹、环形结构化审计日志、多维安全检索与遥测全闭环)**

- **动机(安全审计可见性缺失、凭据泄露风险与审计闭环诉求)**:
  在 Tetherly 陆续完成对端加密连接保活心跳治理与链路画像 (v1.65 / ADR-058)、节点系统资源深度遥测 (v1.64 / ADR-057)、跨端桌面原生系统级通知推送 (v1.63 / ADR-056) 与跨端收件箱流式下载与直开 (v1.62 / ADR-055) 后，网络节点承载了跨设备配对认证、受信任设备解绑、黑名单接入鉴权、文件生命周期流转、跨端剪贴板内容同步以及静态路由寻址等大量关键安全操作。然而，在**节点安全审计轨迹与操作日志（Security Audit Trail & Governance）**维度，系统面临以下关键痛点：
  1. **安全关键事件轨迹黑盒与不可追溯**：
     此前节点对关键安全事件（如设备被加入黑名单、未授权握手拦截、PIN 码验证失败、文件被远程或本地删除、路由被篡改注入）仅通过松散的 `tracing::info!` 打出日志，没有统一的、结构化的安全审计轨迹与内存归档，无法向管理员或本地用户提供操作时间线；
  2. **内存泄露与磁盘耗尽风险（Memory Bound Invariant）**：
     若无界保存审计事件，在遭遇持续暴力探测或长周期运行时极易导致节点内存溢出（OOM）或日志磁盘打爆。必须引入严格定额的内存环形缓冲区（Bounded Circular Ring Buffer），实现自动老化置换与 $O(1)$ 常数级复杂度；
  3. **Zero-Plaintext 零明文安全铁律（Zero-Plaintext Invariant）**：
     审计日志绝不允许记录任何明文私钥、对称密钥、PIN 码、一次性验证码（OTP）、剪贴板正文或通知正文，仅允许持久化脱敏的安全摘要（Summary）与实体标识，杜绝因日志导出导致凭据二次泄露；
  4. **全端多维检索、清空治理与 403 强隔离防御**：
     需提供支持按安全分类（Category）、严重级别（Severity）、时间戳（Since）与数量上限（Limit）的多维检索与清空治理端点，双端 CLI 全同构治理，Web UI 实时交互卡片与 Host 403 防御。

- **架构与协议设计(ADR-059 节点安全审计轨迹、环形结构化审计日志、多维安全检索与遥测全闭环)**:
  1. **强类型数据模型与环形缓冲区审计引擎 (`crates/tetherly-node/src/runtime.rs` & `lib.rs`)**:
     - 抽象强类型审计分类 `AuditCategory`：`Security`（配对/PIN/信任/黑名单）、`Peer`（节点上下线/断开）、`File`（收件箱删除/清空/直开/传输）、`Clipboard`（剪贴板同步/清空）、`Notify`（通知/免打扰）、`Route`（路由注入/删除）、`Config`（配置/心跳调谐）；
     - 抽象四级安全严重度 `AuditSeverity`：`Info`、`Notice`、`Warning`、`Alert`，实现 `Ord` 关系支持最低级别过滤（`severity >= threshold`）；
     - 定义结构化条目 `AuditEntry`：自增唯一标识 `id`、时间戳 `timestamp_ms`、分类 `category`、严重度 `severity`、动作名称 `action`、目标实体 `target`、脱敏摘要 `summary`、执行结果 `success`；
     - 抽象定额环形缓冲区 `AuditEngine`（默认容量 500 条，`VecDeque<AuditEntry>`）：提供 $O(1)$ 均摊压入 `push`、多维筛选查询 `query` 与一键重置 `clear`；
  2. **全安全关键链路深度埋点与运行时原语**:
     - 运行时原语：`Node::record_audit(...)`、`Node::audit_report(...)`、`Node::clear_audit_log(&self) -> AuditClearReport`；
     - 在配对授权（PIN 验证、配对成功/失败、主动解除信任）、黑名单接入控制（添加/移除黑名单）、文件收件箱安全生命周期（文件接收、安全删除、清空、直开）、跨端剪贴板主动同步与清空、原生系统级通知与免打扰、物理寻址路由表增删、心跳保活动态调谐等关键路径全面埋点；
  3. **Zero-Plaintext 零明文安全红线与事件总线 (`crates/tetherly-node/src/events.rs`)**:
     - 新增 `UiEvent::AuditEventRecorded` 零明文事件变体，载荷仅分发脱敏摘要与元数据，严禁广播敏感机密；
  4. **回环控制面端点全闭环与 403 强隔离防御 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/audit`（别名 `/api/audit/logs`）：支持 URL Query 参数 `category`, `severity`, `limit`, `since`；
     - `POST /api/audit/query`：接收 JSON 请求体执行多维高级过滤检索；
     - `POST /api/audit/clear`：安全清空当前内存中的所有审计记录，并自动记录一条 Audit 清空审计日志；
     - 严格执行 127.0.0.1 物理绑定与 `Host: 127.0.0.1|localhost` 403 Forbidden 强隔离防御；
  5. **双端 CLI 完整同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - 增设 `tetherly audit` 与 `tetherly-cli audit` 子命令族：
       - `audit [list]`：支持 `--category <cat>`、`--severity <sev>`、`--limit <n>`、`--since <ms>`、`--json` 输出格式化表格；
       - `audit clear`：一键清空审计轨迹；
       - `audit watch`：基于 SSE 实时监听输出高亮审计事件流；
     - SSE `events` 监听器中提供 `[audit_event_recorded]` 高亮格式化输出；
  6. **Web UI 审计轨迹透视面板与实时日志交互 (`ui/index.html`)**:
     - 增设「节点安全审计轨迹与操作日志」独立面板，支持分类与严重度下拉筛选、条数限制、实时统计徽标（如 `匹配 10 / 累计 42 条`）；
     - 提供「清空审计日志」与「刷新日志」操作，实时渲染彩色列、状态徽标与脱敏摘要；
     - 接入 SSE `audit_event_recorded` 事件实时增量响应；
  7. **端到端集成测试与全量安全基线全绿 (`tests/phase1.rs`)**:
     - 增设 `node_security_audit_trail_filtering_purging_and_zero_plaintext_via_loopback_api` 集成测试；
     - 验证配对/路由/配置等操作的审计记录生成、多维条件筛选过滤、非法参数阻断、清空治理重置、SSE 广播捕获与 Host 403 恶意阻断。

本文是实现合同。与代码冲突时改文档或改代码，必须一致。旧名 InterLink 作废。

**1.65 相对 1.64(对端加密连接保活心跳治理、动态心跳间隔调谐、多维链路质量画像与主动探活遥测全闭环)**

- **动机(对端保活僵化、链路质量黑盒与动态调谐诉求)**:
  在 Tetherly 陆续完成节点系统资源深度遥测 (v1.64 / ADR-057)、跨端桌面原生系统级通知推送 (v1.63 / ADR-056)、收件箱流式下载与直开 (v1.62 / ADR-055) 与局域网设备自发现 (v1.61 / ADR-054) 后，局域网与跨网 Mesh 中持续维护着多条基于 Noise 协议的端到端双向加密长连接。然而，在**对端加密会话保活心跳机制与网络链路质量可观测性（Keep-Alive Heartbeat Governance, Link Quality Profiling & Active Liveness Telemetry）**维度，系统面临以下关键痛点：
  1. **静态固定心跳周期的局限性与无法动态调优**：
     此前节点的会话心跳间隔（Ping Interval）与判定离线超时时限（Liveness Deadline）在会话启动时被硬编码，无法根据动态网络环境（如弱网 Wi-Fi、跨城组网中继、高频交互对局域网低延迟诉求、移动端低功耗诉求）在运行时进行无感动态调节；
  2. **链路质量画像黑盒与单点 RTT 误导**：
     此前系统虽然记录单次往返时延（RTT），但缺少多维度的滑动窗口统计分析。网络环境通常存在抖动（Jitter）与偶发丢包，单个瞬时 RTT 无法真实反映对端连接的稳定度和通信等级，缺乏最小/最大/均值时延、网络抖动与丢包率的综合画像；
  3. **缺乏按需主动并发链路探活工具（Active Burst Link Probing）**：
     排查对端网络性能时，用户无法主动发起一组多包连续探活测速（Burst Ping Probing），只能被动等待周期心跳；
  4. **零明文安全与 Host 403 强隔离防御铁律（Zero-Plaintext & Loopback Isolation Invariants）**：
     探活与链路度量必须严格恪守零明文底线，严禁在事件或诊断报告中携带任何会话私钥、一次性验证码或敏感数据；所有新增回环接口必须物理绑定 127.0.0.1 并强制校验 `Host: 127.0.0.1|localhost`。

- **架构与协议设计(ADR-058 对端加密连接保活心跳治理、动态心跳间隔调谐、多维链路质量画像与主动探活遥测全闭环)**:
  1. **动态保活心跳配置与超时重载模型 (`crates/tetherly-core/src/liveness.rs`, `crates/tetherly-node/src/runtime.rs` & `lib.rs`)**:
     - 定义强类型配置模型 `HeartbeatConfig`（`interval_secs`, `deadline_secs`），默认配置为 15s 发送一次 Ping 保活心跳，45s 无任何入站数据即判定对端超时离线（3倍冗余容错）；
     - 提供运行时动态调优原语 `Node::update_heartbeat_config(&self, interval: Option<u64>, deadline: Option<u64>)`，修改后原子更新节点共享内存 `Shared.heartbeat_config`，正在运行的保活循环与探针无需断开重连现有会话即可无缝生效；
     - 严格参数校验：`interval_secs >= 1 && interval_secs <= 3600`，`deadline_secs > interval_secs && deadline_secs <= 86400`，不满足时阻断并返回清晰错误；
     - 在 `tetherly-core::liveness::Liveness` 状态机中新增 `pub fn set_deadline(&mut self, deadline: Duration)` 原语，支持动态重载离线判定阈值；
  2. **滑动窗口链路质量画像引擎 (`PeerLinkTracker` & `crates/tetherly-node/src/runtime.rs`)**:
     - 维护 32 样本环形滑动窗口（`VecDeque<u64>`），实时计算并提供多维链路画像：
       - 最小时延 `min_rtt_ms` 与最大时延 `max_rtt_ms`；
       - 移动平均往返时延 `avg_rtt_ms`；
       - 网络抖动 `jitter_ms`（公式：$\frac{1}{N-1}\sum_{i=1}^{N-1} |RTT_{i+1} - RTT_i|$）；
       - 发包丢包率 `packet_loss_rate`（公式：$\frac{\text{PingsSent} - \text{PongsRecv}}{\text{PingsSent}} \times 100\%$）；
     - 链路质量多级评价模型 `LinkGrade`（`Excellent` / `Good` / `Fair` / `Poor` / `Unreachable`）：
       - `Excellent`：RTT < 25ms 且丢包率 < 1.0%；
       - `Good`：RTT < 100ms 且丢包率 < 5.0%；
       - `Fair`：RTT < 300ms 且丢包率 < 15.0%；
       - `Poor`：RTT $\ge$ 300ms 或丢包率 $\ge$ 15.0%；
       - `Unreachable`：对端离线或完全不可达；
  3. **主动多包并发探活遥测原语 (`Node::probe_peer_link` & `LinkProbeReport`)**:
     - 运行时原语 `Node::probe_peer_link(&self, peer_id: &str, count: u32, delay_ms: u64) -> Result<LinkProbeReport, CoreError>`；
     - 支持在 1 到 20 个探针包、10 到 1000ms 间隔范围内对在线对端执行主动探测，探测结果自动汇入该对端的滑动窗口画像引擎，并返回包含已发包数、收包数、丢包率、最小/最大/平均 RTT 与抖动的即时报告；
  4. **Zero-Plaintext 零明文安全红线与事件总线可观测性 (`crates/tetherly-node/src/events.rs`)**:
     - 新增 `UiEvent::PeerLinkQualityRefreshed` 与 `UiEvent::HeartbeatConfigChanged`；
     - 严格遵循零明文铁律，载荷仅广播度量数值与时间戳，严禁携带敏感上下文、明文私钥或认证凭据；
  5. **回环控制面端点全闭环与 403 强隔离防御 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/peers/link`（别名 `/api/peer/link`、`/api/peers/quality`）：获取全网对端链路画像报告 `LinkQualityReport`；
     - `POST /api/peers/link/probe`（别名 `/api/peer/link/probe`）：发起主动链路突发测速探针；
     - `GET /api/peers/heartbeat`（别名 `/api/peers/ping`）：查询当前节点的保活心跳与离线超时配置；
     - `POST /api/peers/heartbeat`（别名 `/api/peers/ping`）：动态修改心跳参数并广播事件；
     - 严格执行 127.0.0.1 物理绑定与 `Host: 127.0.0.1|localhost` 403 Forbidden 强隔离防御；
  6. **双端 CLI 完整同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - 增设 `tetherly link` 与 `tetherly-cli link` 子命令族：
       - `link [status]`：表格化展示对端设备名、状态、时延、区间、抖动、丢包率、评级与当前心跳策略；
       - `link probe <peer> [--count <n>] [--delay-ms <ms>]`：执行多包链路测速并输出统计报告；
       - `link config [--interval <secs>] [--deadline <secs>]`：查询或更新运行时心跳间隔与判定离线阈值；
     - SSE `events` 监听器中提供 `[peer_link_quality_refreshed]` 与 `[heartbeat_config_changed]` 高亮格式化输出；
  7. **Web UI 链路画像透视卡片与一键探活测速交互 (`ui/index.html`)**:
     - 增设「对端链路质量画像与保活调谐」独立控制面板，实时渲染品质徽标、RTT 波动范围、Jitter 与丢包率；
     - 提供一键发起「探活测速」及保活间隔调谐弹窗；
     - 接入 SSE `peer_link_quality_refreshed` 与 `heartbeat_config_changed` 实时响应；
  8. **端到端集成测试与全量安全基线全绿 (`tests/phase1.rs`)**:
     - 增设 `peer_link_quality_profiling_keepalive_tuning_and_burst_probe_via_loopback_api` 集成测试；
     - 验证默认配置查询、动态心跳调谐、边界非法参数阻断、链路画像生成、突发多包探针、滑动窗口度量更新、SSE 广播与 Host 403 恶意阻断。

**1.64 相对 1.63(节点系统资源深度遥测、进程级性能画像与运行时健康度量闭环)**

- **动机(系统资源可观测性缺失、进程健康盲区与零新依赖诉求)**:
  在 Tetherly 陆续完成跨端桌面系统级通知推送与免打扰治理 (v1.63 / ADR-056)、收件箱文件内容流式下载与系统直开 (v1.62 / ADR-055)、局域网自发现治理 (v1.61 / ADR-054) 与收件箱生命周期管理 (v1.60 / ADR-053) 后，节点在处理长连接会话、大文件分块流式传输、蓝牙/网络事件监听时，由于涉及多线程并发与大文件 I/O 句柄流转，其**运行时物理资源消耗（Resident Set Size / Virtual Memory / Active Threads / Open File Descriptors / CPU Execution Time）**在生产环境下存在严重的可观测性盲区：
  1. **守护进程资源消耗黑盒**：
     此前系统仅提供基础的 `/api/health` 检查（仅返回节点运行版本、PID、启动时长与端口），完全无法感知守护进程是否存在内存泄露、句柄未关闭（FD leak）或线程失控堆积；
  2. **跨平台原生探测与零新依赖（Zero-New-Crate & Zero-AGPL Invariant）严苛红线**：
     主流系统信息库（如 `sysinfo` 等第三方 crate）往往体积庞大、依赖多、编译缓慢，或存在潜在许可污染（AGPL 依赖绝对禁止）。必须通过 Rust 标准库与已有操作系统底层底层调用（macOS Mach 内核 API、Linux `/proc`、POSIX `getrusage` / `/dev/fd`），实现零外部新 crate 的轻量高效探测；
  3. **单元与集成测试的确定性注入（Deterministic Test Mocking）**：
     依赖真实宿主状态的测试在 CI 或无特权容器中容易抖动，必须提供可测试的 `MemorySysProbe` 探针注入机制，以便在端到端集成测试中确定性断言各项度量指标与异常场景；
  4. **零明文安全铁律（Zero-Plaintext Telemetry Invariant）**：
     系统指标遥测严禁扫描或导出任何环境变量、命令行参数或进程上下文机密，仅允许传输纯粹的物理资源计数值与时间戳；
  5. **回环控制面与双端 CLI 全同构治理**：
     控制面需暴露 `GET /api/sys`（及其别名 `/api/system`、`/api/process`），双端 CLI 提供 `sys [--watch] [--interval <secs>] [--json]`，Web UI 实时展示资源画像卡片并接入 SSE 事件总线。

- **架构与协议设计(ADR-057 节点系统资源深度遥测、进程级性能画像与运行时健康度量闭环)**:
  1. **跨平台底层探测抽象与原生驱动 (`crates/tetherly-node/src/platform.rs`)**:
     - 定义强类型采样 DTO：`ProcessSample`（`rss_bytes`, `virt_bytes`, `thread_count`, `open_fds`, `cpu_time_ms`）与 `HostSample`（`total_memory_bytes`）；
     - 定义抽象特质：`pub trait SysProbe: Send + Sync { fn sample_process(&self, pid: u32) -> Result<ProcessSample, CoreError>; fn sample_host(&self) -> Result<HostSample, CoreError>; }`；
     - 原生跨平台实现 `PlatformSysProbe`：
       - **macOS**: 采用 Mach 内核原生接口：`mach_task_self()`，`task_info(..., MACH_TASK_BASIC_INFO, ...)` 提取 `resident_size` 与 `virtual_size`；调用 `task_threads(...)` 获得线程列表与线程数，并立即调用 `vm_deallocate` 避免 Mach port 泄漏；
       - **Linux**: 读取 `/proc/self/statm`（获取常驻与虚拟页数）与 `/proc/self/status`（获取 `Threads` 字段），结合 `sysconf(_SC_PAGESIZE)` 计算真实字节数；
       - **POSIX 通用 (Unix)**: 调用 `getrusage(RUSAGE_SELF, ...)` 获取 CPU 用户态与内核态微秒时间累计（转换为毫秒），并利用 `sysconf(_SC_PHYS_PAGES)` 计算宿主机总内存；
       - **打开文件描述符统计**: 遍历 `/dev/fd`（或 `/proc/self/fd`）安全统计打开句柄数，无需任何外部进程子调用；
     - 测试专用驱动 `MemorySysProbe`：提供线程安全 mock 适配器（`set_process`, `set_host`），实现完全确定性的自动化测试注入；
  2. **强类型数据传输模型与运行时控制原语 (`crates/tetherly-node/src/runtime.rs` & `lib.rs`)**:
     - `ProcessMetrics`：包含 PID、启动时长、RSS、Virt、线程数、FD 计数、CPU 耗时及友好格式化字符串（`42.00 MiB`）；
     - `HostMetrics`：包含操作系统类型、架构、CPU 核心数、物理总内存及格式化字符串；
     - `SystemReport`：聚合节点名称、设备 ID、进程画像、主机画像与时间戳；
     - 运行时原语 `Node::system_report(&self) -> Result<SystemReport, CoreError>` 与注入器 `Node::set_sys_probe(&self, probe: Arc<dyn SysProbe>)`；
  3. **Zero-Plaintext 零明文事件总线可观测性 (`crates/tetherly-node/src/events.rs`)**:
     - 新增 `UiEvent::SystemMetricsRefreshed`，仅广播资源指标数值，严格遵循零明文铁律；
  4. **回环控制面端点全闭环 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/sys`（别名 `GET /api/system`、`GET /api/process`）：调用 `Node::system_report()` 返回 JSON 报告；
     - 严格执行 127.0.0.1 物理绑定与 `Host: 127.0.0.1|localhost` 403 Forbidden 强隔离防御；
  5. **双端 CLI 完整同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - 增设 `tetherly sys` 与 `tetherly-cli sys` 命令，支持 `--watch`、`--interval <secs>` 与 `--json`；
     - SSE `events` 监听器中提供 `[system_metrics_refreshed]` 高亮格式化输出；
  6. **Web UI 节点系统资源与运行时画像专区 (`ui/index.html`)**:
     - 增设「节点系统资源与运行时画像」监控面板，展示宿主平台、内存总量、RSS、虚拟内存、线程数、FD 计数与 CPU 耗时；
     - 接入 SSE `system_metrics_refreshed` 动态刷新与周期性自动同步；
  7. **端到端集成测试与全量安全基线全绿 (`tests/phase1.rs`)**:
     - 增设 `node_system_telemetry_probe_injection_and_metrics_via_loopback_api` 集成测试；
     - 验证原生采样、MemorySysProbe 注入断言、回环 HTTP 访问、别名校验、SSE 事件捕获与 Host 403 恶意阻断。

**1.63 相对 1.62(跨端桌面原生系统级通知推送、横幅弹窗治理与免打扰闭环)**

- **动机(跨端桌面系统通知缺失、遗留悬空存根与打扰体验痛点)**:
  在 Tetherly 陆续完成收件箱文件内容流式下载与系统直开 (v1.62 / ADR-055)、局域网自发现治理 (v1.61 / ADR-054)、收件箱生命周期管理 (v1.60 / ADR-053) 及大文件传输引擎后，核心传输网已能高度可靠地在设备间传递通知帧与落盘文件。然而，在**桌面操作系统原生通知呈现与用户专注体验（Native System Notification Banners & Do-Not-Disturb Governance）**层面，存在长期未决的设计与实现断层：
  1. **历史悬空特质未贯通（Dangling Port Stub）**：
     在 SSOT §2494 规划的六大系统平台端口中（`Clock`, `Rng`, `TrustStore`, `Insertor`, `Clip`, `Notifier`），`Notifier` 接口定义一直处于悬空存根状态，未被核心运行时与平台适配器真正接入；
  2. **收到对端通知或大文件时桌面缺乏原生横幅感知（No Desktop OS Visual Awareness）**：
     当手机对端推送短信、系统通知，或局域网对端完成数个 GiB 的大文件传输时，桌面用户必须主动切到 Web UI 或终端查阅，缺乏如 macOS 通知中心横幅、Windows 任务栏气泡/Toast、Linux freedesktop 横幅的原生即时提醒；
  3. **缺乏集中式免打扰与声音控制治理（Lack of DND Governance & Sound Muting）**：
     用户在专注编码、开会或游戏时，如果无法一键静音或抑制普通弹窗，频繁的弹窗会造成严重的打扰；同时，如果完全关闭通知，又会错过银行、登录或支付等高优先级的关键动态验证码；
  4. **隐私防窥与零明文安全红线（Zero-Plaintext Invariant & Shoulder Surfing Protection）**：
     在公共办公区或屏幕共享时，弹出包含完整验证码数字的明文横幅极易造成隐私泄露。系统必须支持在弹出横幅时自动掩码（如掩盖验证码正文），且在任何 SSE 事件、遥测日志或公开传输中严禁广播正文与一次性验证码明文；
  5. **双端命令行与控制面缺乏治理能力**：
     双端 CLI 与 Web 控制台此前完全缺乏配置通知开关、免打扰级别、触发测试横幅与查看派发/压制指标的途径。

- **架构与协议设计(ADR-056 跨端桌面原生系统级通知推送、横幅弹窗治理与免打扰闭环)**:
  1. **核心通知特质重构 (`crates/tetherly-core/src/ports.rs`)**:
     - 升级 `Notifier` trait 为 `fn notify(&self, payload: &NotificationPayload) -> Result<(), CoreError>`；
     - 定义强类型 `NotificationPayload` 与 `NotificationUrgency`（Low / Normal / Critical），支持标题、正文、应用源、提示音开关与紧急程度标记；
     - 彻底贯通六大平台底层端口契约；
  2. **跨平台原生操作系统通知调度适配器 (`crates/tetherly-node/src/platform.rs`)**:
     - `SystemNotifier`：在 macOS 上采用 `osascript -e 'display notification ...'`；在 Windows 上采用 PowerShell 调用 `System.Windows.Forms.NotifyIcon` 弹出气泡；在 Linux 上调用 `notify-send`；
     - `MemoryNotifier`：提供线程安全内存容器（`Arc<Mutex<Vec<NotificationPayload>>>`），专为无界面无头 CI、单元测试及集成测试设计；
  3. **集中式免打扰 (DND) 与调度策略引擎 (`crates/tetherly-node/src/runtime.rs`)**:
     - `NotifyBannerConfig` 包含：`banners_enabled: bool`, `dnd_mode: DndMode`（`off` / `silent` / `all`）, `sound_enabled: bool`, `mask_otp: bool`, `allow_otp_in_dnd: bool`；
     - 调度原语 `Node::dispatch_notification_banner`：根据 DND 模式、OTP 识别结果进行压制检查与正文掩码处理，精准维护 `notify_dispatched_count` 与 `notify_suppressed_count` 计数器；
  4. **传输核心自动化事件接入 (`crates/tetherly-node/src/runtime.rs`)**:
     - 接收手机推送通知（`Node::ingest_local_push`）时自动调用横幅调度；
     - 接收对端文件流式传输落盘完成（`Node::receive_file_loop`）时自动弹出「文件接收完成」横幅；
  5. **零明文事件总线可观测性 (`crates/tetherly-node/src/events.rs`)**:
     - 新增 `UiEvent::NotificationBannerSent`（仅传输标题、应用源、是否 OTP 及时间戳，坚决剔除正文与验证码明文）；
     - 新增 `UiEvent::NotifyConfigChanged`，策略变更即时全端通知；
  6. **回环控制面端点全闭环 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/notify/config`（别名 `/api/notify/banner`）：读取当前配置与累计派发/压制指标；
     - `POST /api/notify/config`（别名 `/api/notify/banner`）：增量或全量更新配置；
     - `POST /api/notify/test`（别名 `/api/notify/banner/test`）：触发测试横幅；
     - 严格执行 127.0.0.1 物理绑定与 `Host: 127.0.0.1|localhost` 403 Forbidden 强隔离；
  7. **双端 CLI 完整同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - `tetherly banner config [--banners] [--dnd off|silent|all] [--sound] [--mask-otp] [--allow-otp-dnd]`；
     - `tetherly banner test [--title <str>] [--body <str>]`；
     - SSE `events` 监听器高亮输出 `[notification_banner_sent]` 与 `[notify_config_changed]`；
  8. **Web UI 桌面通知与免打扰控制台 (`ui/index.html`)**:
     - 增设桌面通知与免打扰卡片、开关状态标签、DND 模式下拉选择、一键测试横幅按钮及实时数据回显；
  9. **端到端集成测试全覆盖 (`tests/phase2.rs`)**:
     - `desktop_notification_banners_dnd_governance_and_otp_masking_via_loopback_api` 覆盖横幅派发、内存捕获、Zero-Plaintext 校验、OTP 掩码与高危提升、静音模式、全部免打扰模式过滤与白名单放行、总开关禁用与 Host 403 恶意阻断。

**1.62 相对 1.61(跨端收件箱文件内容提取、流式下载、系统级直开与多媒体预览全闭环)**
| crate / 二进制 | `tetherly-core`、`tetherly-crypto`、`tetherly-net`、`tetherly-node`（`tetherly` 二进制）、`tetherly-cli`、`tetherly-android`（`libtetherly_android.so` + `android_probe`） |

本文是实现合同。与代码冲突时改文档或改代码，必须一致。旧名 InterLink 作废。

**1.62 相对 1.61(跨端收件箱文件内容提取、流式下载、系统级直开与多媒体预览全闭环)**

- **动机(收件箱落地文件提取与系统交互断层痛点)**:
  在 Tetherly 陆续完成局域网自发现与信标治理 (v1.61 / ADR-054)、收件箱生命周期查询与安全删除 (v1.60 / ADR-053)、批量文件提议与断点续传可观测性 (v1.59 / ADR-052) 之后，核心传输网络已能稳定接收文件并入库落盘至 `<data_dir>/inbox/`。然而，在**收件箱落地文件的提取、使用与操作系统联动（Inbox Extraction, Native Open & Content Delivery）**层面，依然存在严重的使用闭环断层：
  1. **收件箱文件无法提取与下载（No Content Extraction / Download API）**：
     现存系统仅提供元数据查询接口（`GET /api/file/inbox`）与删除接口（`POST /api/file/inbox/delete`），完全缺乏文件内容提取与下载端点；Web 控制台只能查看文件列表，用户无法点击下载到浏览器本地目录或预览多媒体内容；命令行 CLI 缺乏导出子命令，在无头或远程模式下无法提取落盘文件；
  2. **既有回环客户端 UTF-8 损毁二进制（Binary Corruption in Text Splitter）**：
     既有 `loopback_get` 等客户端实现使用 `String::from_utf8_lossy` 按文本切分头部与载荷，导致下载非 UTF-8 二进制文件（如图片、视频、压缩包、二进制执行体）时产生严重的字节替换损毁（`\u{FFFD}`）；
  3. **缺乏操作系统级默认应用直开与所在目录定位原语（Native Open & Reveal In File Manager）**：
     在桌面操作系统（macOS / Windows / Linux）上，用户收到文件后最自然的高频交互是“双击/打开该文件”或“在系统文件管理器中显示收件箱”；既有 `SystemOpener` 虽然具备调起操作系统的能力，但仅用于打开白名单 URL，未与收件箱文件系统打通；
  4. **路径穿越防御红线（Strict Path Traversal Guard）**：
     文件读取与系统调起必须坚守路径穿越防御红线：坚决拦截任何包含 `..`、`/`、`\`、`\0` 或以 `.` 开头的文件名，校验 `clean_name == name`；
  5. **事件总线零明文审计与即时分发（Zero-Plaintext Invariant）**：
     当文件被系统打开或被读取时，向事件总线派发 `UiEvent::InboxFileOpened`；严格恪守零明文铁律：事件载荷仅允许携带脱敏后的文件名与时间戳，严禁携带私钥或文件内部机密正文。

- **架构与协议设计(ADR-055 跨端收件箱文件内容提取、流式下载、系统级直开与多媒体预览全闭环)**:
  1. **强类型 HTTP 响应抽象与二进制安全切分 (`crates/tetherly-node/src/uihttp.rs` & `bins/tetherly-cli/src/main.rs`)**:
     - 升级控制面响应模型为强类型 `UiResponse`，支持状态码、MIME 类型、动态 HTTP 响应头扩展与原始载荷；
     - 重构客户端 `parse_http_response` 为滑动窗口字节匹配 `b"\r\n\r\n"`，确保 100% 二进制无损传输；
  2. **收件箱内容提取与系统调起原子原语 (`crates/tetherly-node/src/runtime.rs` & `lib.rs`)**:
     - 扩展 `InboxOpenReport` 与 MIME 类型智能映射函数 `mime_type_from_name`（识别 PNG、JPEG、GIF、WEBP、SVG、PDF、TXT、JSON、HTML、CSS、JS、CSV、ZIP、GZ、TAR、MP3、WAV、MP4、WEBM、MKV 等）；
     - 实现 `Node::open_inbox_file` 与 `Node::read_inbox_file`，支持打开具体文件或打开 `<data_dir>/inbox` 目录；
     - 注入可插拔 `Opener` 支持测试隔离（`MemoryOpener`）与生产真实调起（`SystemOpener`）；
  3. **事件总线零明文审计与即时分发 (`crates/tetherly-node/src/events.rs`)**:
     - 新增 `UiEvent::InboxFileOpened { name: Option<String>, timestamp_ms: u64 }`，零明文审计，实时通知前端与 CLI 监听器；
  4. **轻量级回环控制面端点全闭环 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/file/inbox/download`（与别名 `/api/inbox/download`）：流式下载附件，设置 `Content-Disposition: attachment; filename="..."`；
     - `GET /api/file/inbox/raw`（与别名 `/api/inbox/raw`）：内联查看多媒体/文本，设置 `Content-Disposition: inline; filename="..."`；
     - `POST /api/file/inbox/open`（与别名 `/api/inbox/open`）：接收可选 `{"name": "..."}`，调起系统默认应用打开文件或收件箱；
     - 严格执行 127.0.0.1 物理绑定与 `Host: 127.0.0.1|localhost` 403 Forbidden 强隔离；
  5. **双端 CLI 完整同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - `tetherly inbox get <name> [--dest <path>] [--ui-port <port>]`：从收件箱流式提取落盘；
     - `tetherly inbox open [<name>] [--ui-port <port>]`：调起系统默认应用打开文件或收件箱目录；
     - SSE `events` 监听器高亮输出 `[inbox_file_opened]`；
  6. **Web UI 收件箱提取与交互联动 (`ui/index.html`)**:
     - 收件箱面板标题栏新增「打开目录」按钮；
     - 文件列表项新增「打开」、「下载」与「预览」按钮；
     - 监听 `inbox_file_opened` 事件；
  7. **端到端集成测试与全量安全基线全绿**:
     - `tests/phase1.rs` 增设 `inbox_file_download_raw_preview_and_native_open_via_loopback_api` 完整覆盖流式下载二进制保真度、Content-Disposition 响应头、raw 内联预览、系统直开调用捕获、路径穿越阻断、404 不存在处理与 Host 403 恶意阻断。

**1.61 相对 1.60(局域网设备自发现治理、信标可观测性与一键拨号配对闭环)**

- **动机(局域网发现监听黑盒与手动 IP 拨号摩擦痛点)**:
  在 Tetherly 陆续完成跨端收件箱生命周期治理与安全删除 (v1.60 / ADR-053)、批量文件提议与断点续传可观测性画像 (v1.59 / ADR-052) 及全桌面 ANCS GATT 接入闭环 (v1.58 / ADR-051) 之后，核心数据面与传输面已高度可靠。然而，在**局域网设备自发现治理与拨号链路（LAN Discovery & Dial Governance）**层面，系统长期存在控制与用户体验盲区：
  1. **单向广播与发现监听黑盒（One-Way Advertising without Browsing Blackout）**：
     节点在启动后虽然能够通过 `crates/tetherly-net/src/mdns.rs` 注册 `_tetherly._tcp.local.` mDNS 服务向外广播本机控制端口、设备 ID、公钥指纹与能力集；但节点**从未启动 mDNS 服务浏览（Service Browser）**或监听局域网对端广播信标，导致同一局域网内的 Tetherly 设备完全无法感知彼此的存在；
  2. **缺乏局域网设备发现画像与控制面查询端点（Discovery Observability Blackout）**：
     控制面缺乏 `GET /api/discovery`（别名 `/api/peers/discovered`）与 `POST /api/discovery/rescan` 端点；调用方、CLI 与 Web UI 无法探查局域网内当前发现了哪些可用对端、其 IP 地址列表、控制端口、设备公钥指纹、能力特性以及在线/信任状态；
  3. **拨号流程割裂与必须手动输入 IP:Port 痛点（Manual IP Dial Friction）**：
     既有 `tetherly dial` 与 `/api/dial` 强制要求用户手动查询对端 IP 并输入 `--addr <ip:port>`；无法基于已发现的设备友好名称、别名或设备 ID 直接一键拨号配对；
  4. **事件总线局域网发现事件断层（Event Bus Gap for Discovered Peers）**：
     当局域网出现新设备广播或设备离线/过期时，事件总线未派发任何事件；Web UI 无法实现即时动态感知；必须严格恪守 Zero-Plaintext 铁律：事件载荷仅允许携带脱敏的公开元数据（`device_id`, `name`, `addr`, `caps`），严禁携带任何私密密钥；
  5. **双端 CLI 与 Web UI 缺少自发现全同构支持**：
     缺少双端同构的 `discover` 子命令；`dial` 子命令不支持 `--peer <name|id>` 智能寻址；Web UI 缺乏「局域网自动发现设备」面板与一键「填入/连接」操作。

- **架构与协议设计(ADR-054 局域网设备自发现治理、信标可观测性与一键拨号配对闭环)**:
  1. **网络层 mDNS 浏览与注销原语 (`crates/tetherly-net/src/mdns.rs` & `lib.rs`)**:
     - 封装 `mdns_sd::ServiceDaemon` 的浏览能力：
       ```rust
       #[cfg(feature = "lan")]
       pub fn browse(daemon: &ServiceDaemon) -> Result<MdnsReceiver<ServiceEvent>, NetError>;
       #[cfg(feature = "lan")]
       pub fn stop_browse(daemon: &ServiceDaemon) -> Result<(), NetError>;
       ```
     - 为非 `lan` 特性提供匹配的 safe stubs；并在 mDNS 注册时将 `("name", hostname)` 写入 TXT 记录，确保设备名可被直接解析。
  2. **核心模型设计与 DTO 抽象 (`crates/tetherly-node/src/runtime.rs` & `lib.rs`)**:
     - 定义强类型画像 DTO：
       ```rust
       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct DiscoveredPeerDto {
           pub device_id: String,
           pub name: String,
           pub addrs: Vec<String>,
           pub port: u16,
           pub fp: Option<String>,
           pub caps: Vec<String>,
           pub is_trusted: bool,
           pub is_online: bool,
           pub last_seen_ms: u64,
       }

       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct DiscoveryReport {
           pub ok: bool,
           pub count: usize,
           pub peers: Vec<DiscoveredPeerDto>,
       }

       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct DiscoveryRescanReport {
           pub ok: bool,
           pub message: String,
       }
       ```
  3. **运行时自发现表维护与智能拨号寻址原语**:
     - 在 `Shared` 中维护线程安全自发现表 `discovered: Mutex<HashMap<String, DiscoveredPeerInfo>>` 与 `rescan_trigger: broadcast::Sender<()>`；
     - 自动过滤自身设备 ID（不向自发现表中记录自身）；
     - `pub fn discovered_peers(&self) -> DiscoveryReport`：单遍汇总当前发现的所有局域网设备，计算 `is_trusted` 与 `is_online` 标记，按 `last_seen_ms` 倒序排序；
     - `pub fn rescan_discovery(&self) -> DiscoveryRescanReport`：主动向组播网络发射信标重探请求；
     - `pub async fn dial_peer(&self, target: &str) -> Result<SocketAddr, tetherly_net::NetError>`：智能寻址解析引擎，支持：
       1. 直接物理 `SocketAddr` 解析（如 `192.168.1.50:45717`）；
       2. 自发现表中设备名称或设备 ID 候选解析，并对候选 IP 列表按优先级链式拨号（配备 2s 单目标快速超时，避免无响应 IP 阻塞）；
       3. 路由表缓存解析；
  4. **事件总线扩展与零明文审计 (`crates/tetherly-node/src/events.rs`)**:
     - 在 `UiEvent` 中扩展变体：
       ```rust
       PeerDiscovered {
           device_id: String,
           name: String,
           addr: String,
           caps: Vec<String>,
       },
       PeerExpired {
           device_id: String,
       },
       ```
     - 严格遵守 Zero-Plaintext 铁律，仅传输设备名称、脱敏 ID、主要地址与能力标签。
  5. **回环控制面端点群全闭环 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/discovery`（别名 `/api/peers/discovered`）：输出 `DiscoveryReport`；
     - `POST /api/discovery/rescan`（别名 `/api/peers/discover`）：输出 `DiscoveryRescanReport`；
     - 升级 `POST /api/dial`：除 `addr` 外支持可选 `peer` 参数，调用 `dial_peer` 智能寻址；
     - 全量端点严格绑定 `127.0.0.1` 物理回环并强制校验 `Host: 127.0.0.1|localhost`（不匹配返回 403 Forbidden）。
  6. **双端 CLI 全同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - 增设 `discover [--rescan] [--json]` 子命令；
     - 升级 `dial [--addr <addr>] [--peer <name|id>] [--pin <pin>]`；
     - `events` 监听器格式化输出 `[peer_discovered]` 与 `[peer_expired]`。
  7. **Web UI 局域网自发现交互面板 (`ui/index.html`)**:
     - 在配对区域增设「局域网自动发现设备」交互卡片；
     - 展现设备名称、ID、IP:端口、配对/在线状态与功能特性徽标；
     - 提供「填入」与「连接」一键操作以及「刷新探测」按钮；
     - 深度接入 SSE 事件流实现实时无感重绘。
  8. **端到端集成测试全覆盖 (`tests/phase1.rs`)**:
     - `lan_discovery_and_beacon_governance_via_loopback_api` 全面覆盖空状态查询、主动重探、模拟设备发现、智能寻址解析与拨号拒绝、设备过期、Host 403 恶意攻击阻断。

**1.60 相对 1.59(跨端收件箱生命周期治理、已接收文件多维检索与安全审计清除全闭环)**

- **动机(收件箱文件生命周期盲区与安全销毁原语缺失痛点)**:
  在 Tetherly 完成跨端多文件批量发送提议、断点续传可观测性画像与传输治理全闭环 (v1.59 / ADR-052) 之后，文件协同网络在发送端提议、网络层流式分块、断点续传和存储统计上已达成强一致性。然而，在**收件箱（Received Files / Inbox）文件生命周期与安全治理**层面，依然暴露出显著的功能盲区与审计空白：
  1. **收件箱文件生命周期与细粒度画像盲区（Inbox File Inspection Blackout）**：
     当对端推送的文件传输完成并原子校验入库落盘在 `<data_dir>/inbox/` 目录后，既有系统仅能在 `GET /api/storage` 中提供粗粒度的总量概览（`inbox_files_count` 与 `inbox_total_bytes`）；外部调用方、CLI 终端与 Web UI 无法探查收件箱内具体存有哪些文件（文件名、大小、修改/入库时间戳、文件后缀扩展名、安全文件名合规标记），缺乏多维条件过滤（按文件名搜索 `search`、按最近修改时间倒序、按数量分页 `limit`）；
  2. **缺乏已接收文件的安全治理与生命周期销毁原语（Secure Deletion & Safe Purge）**：
     目前系统仅有针对断点续传临时分块的 `POST /api/storage/clean`（仅清理 `.<id>.part` 与 `.<id>.resume.json`），缺乏针对已落盘完成文件的安全管理机制；当用户接收了敏感临时文件或需要释放存储空间时，无法通过回环控制面或 CLI 安全删除指定文件或一键清空收件箱；
  3. **路径遍历安全红线守卫（Path Traversal Guard）**：
     文件删除操作必须严格限制在 `inbox/` 根目录内部，调用 `tetherly_core::sanitize_file_name` 严密防范包含 `..`、绝对路径、隐藏文件或 NUL 字节的任意文件删除与目录逃逸攻击；
  4. **收件箱事件总线生命周期断层（Event Blackout on File Receipt & Removal）**：
     文件从收件箱删除或清空时，未向事件总线派发任何事件；Web UI 与 SSE 订阅者无法实现无缝即时局部刷新；必须严格恪守 Zero-Plaintext 铁律：事件载荷仅允许携带脱敏元数据（`name`, `freed_bytes`, `timestamp_ms` 等），严禁携带文件物理路径或内部内容；
  5. **双端 CLI 与 Web UI 缺少全同构治理入口**：
     缺少双端同构的 `inbox` 子命令族（`inbox list`、`inbox delete <name>`、`inbox clear`）；Web UI 缺乏已接收文件治理面板、检索框与行内删除/一键清空交互。

- **架构与协议设计(ADR-053 跨端收件箱生命周期治理、已接收文件多维检索与安全审计清除全闭环)**:
  1. **核心模型设计与 DTO 抽象 (`crates/tetherly-node/src/runtime.rs` & `lib.rs`)**:
     - 定义强类型画像 DTO：
       ```rust
       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct InboxFileDto {
           pub name: String,
           pub size: u64,
           pub modified_at_secs: u64,
           pub is_safe: bool,
           pub extension: Option<String>,
       }

       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct InboxReport {
           pub ok: bool,
           pub count: usize,
           pub total_bytes: u64,
           pub files: Vec<InboxFileDto>,
       }

       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct InboxDeleteReport {
           pub ok: bool,
           pub name: String,
           pub freed_bytes: u64,
       }

       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct InboxClearReport {
           pub ok: bool,
           pub deleted_count: usize,
           pub freed_bytes: u64,
       }
       ```
  2. **运行时原子控制原语与安全守卫**:
     - `pub fn inbox_report(&self, search: Option<&str>, limit: Option<usize>) -> InboxReport`：
       - 单遍扫描 `<data_dir>/inbox` 目录；
       - 自动跳过隐藏文件与 sidecar 侧车文件（`name.starts_with('.')`）；
       - 读取文件大小 `metadata.len()` 与 UNIX 修改时间 `modified()`；
       - 调用 `tetherly_core::sanitize_file_name(&name).is_ok()` 判定安全合规性；
       - 支持 `search` 关键字不区分大小写匹配；按 `modified_at_secs` 倒序排序；按 `limit` 截断；
     - `pub fn delete_inbox_file(&self, name: &str) -> Result<InboxDeleteReport, CoreError>`：
       - **严密路径穿越守卫**：首先校验文件名不为空且不包含 `/`、`\`、`..`、`\0` 且不以 `.` 开头；随后执行 `let clean_name = tetherly_core::sanitize_file_name(name)?`，校验 `clean_name == name`，否则坚决阻断并返回 `Err(CoreError::UnsafeFileName)`；
       - 严格限定在 `inbox_dir.join(&clean_name)`；
       - 校验目标存在且为文件（非目录）；读取文件尺寸后调用 `std::fs::remove_file`；
       - 广播 `UiEvent::InboxFileDeleted { name: clean_name, freed_bytes, timestamp_ms }`；
     - `pub fn clear_inbox(&self, before_secs: Option<u64>) -> Result<InboxClearReport, CoreError>`：
       - 扫描 `inbox` 目录下所有常规文件（跳过 `.` 开头的隐藏分块/断点续传文件）；
       - 若提供 `before_secs`，仅删除修改时间早于该时间戳的文件；
       - 统计累积 `deleted_count` 与 `freed_bytes`；
       - 广播 `UiEvent::InboxCleared { deleted_count, freed_bytes, timestamp_ms }`。
  3. **事件总线扩展与零明文审计 (`crates/tetherly-node/src/events.rs`)**:
     - 在 `UiEvent` 枚举中新增 `InboxFileDeleted` 与 `InboxCleared` 变体及其构造函数；
     - **Zero-Plaintext 铁律守卫**：绝不携带文件内部数据或绝对物理磁盘路径，仅传递脱敏的安全文件名、释放字节数与时间戳。
  4. **轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/file/inbox` 与别名 `GET /api/inbox`：支持 Query 参数 `?search=<term>&limit=<n>`，返回 `InboxReport`；
     - `POST /api/file/inbox/delete` 与 `DELETE /api/file/inbox`：接收 `{"name": "..."}` 或 Query `?name=...`，文件不存在返回 404，不合法文件名返回 400，成功返回 `InboxDeleteReport`；
     - `POST /api/file/inbox/clear` 与 `DELETE /api/file/inbox/all`：接收 `{"before_secs": ...}`（可选），返回 `InboxClearReport`；
     - **回环安全隔离铁律**：全量端点严格执行 `127.0.0.1` 物理绑定与 `Host: 127.0.0.1|localhost` 白名单校验（不匹配时强行阻断并返回 403 Forbidden）。
  5. **双端 CLI 全同构升级 (`crates/tetherly-node/src/bin/tetherly.rs` 与 `bins/tetherly-cli/src/main.rs`)**:
     - 增设 `inbox` 子命令族：
       - `inbox list [--search <query>] [--limit <n>] [--json] [--ui-port <port>]`：格式化表格展示收件箱文件（文件名、大小、时间、状态），底部汇总文件总数与总体积；
       - `inbox delete <name> [--json] [--ui-port <port>]`：安全删除收件箱内指定文件；
       - `inbox clear [--yes] [--before-secs <secs>] [--json] [--ui-port <port>]`：清空收件箱（带有交互式二次确认，`--yes` 或 `--json` 时跳过）；
     - SSE `events` 监听器高亮：格式化输出 `[inbox_file_deleted]` 与 `[inbox_cleared]`。
  6. **Web UI 实时交互面板 (`ui/index.html`)**:
     - 增设「已接收文件与收件箱治理 (Inbox)」卡片：
       - 展示收件箱文件列表（文件名、大小、修改时间、安全状态、删除按钮）；
       - 增设搜索过滤输入框（带 250ms 防抖）；
       - 增设「清空收件箱」二次确认清除操作按钮；
       - 对接 SSE 分发流中的 `inbox_file_deleted`、`inbox_cleared` 与 `file_done` 事件，实现无感局部重绘。
  7. **验证基线**:
     - `tests/phase1.rs` 增设 `inbox_file_lifecycle_query_deletion_and_safe_purge_via_loopback_api` 集成测试；
     - 全工作区 342+ 项测试全绿，Linux aarch64 交叉编译通过，clippy 零告警，cargo deny 全检通过。

**1.59 相对 1.58(跨端多文件批量发送提议、断点续传可观测性画像与传输治理全闭环)**

- **动机(批量文件提议割裂与断点续传标记黑盒痛点)**:
  在 Tetherly 完成三大桌面系统 ANCS GATT 全同构接入 (v1.58 / ADR-051) 以及跨端单文件传输提议治理 (v1.53 / ADR-046) 之后，核心传输管道在多文件传输场景仍存在显著操作与体验断层：
  1. **多文件发送提议割裂（Fragmented Multi-File Transfers）**：
     此前双端 CLI（`tetherly send <path>`）与回环控制面（`/api/file/send`）仅支持单路径输入。当用户需要向对端批量传输多份照片、日志或数据集时，必须反复多次调用命令，导致接收端被连续多个独立的弹窗打扰；同时缺乏全局统一的批次标识（Batch ID）与聚合统计回报（传输总文件数、成功/失败计数、累计字节数）。
  2. **断点续传标记不可见与黑盒盲区（Invisible Resume Markers）**：
     Tetherly 底层虽然支持分块断点续传并在 `inbox/` 目录落盘 `.<transfer_id>.resume.json` 侧车文件与内存 `ResumeTable`，但外部完全缺乏断点续传画像端点。用户或前端 UI 无法知晓当前系统存在哪些可恢复任务、其续传偏移量、块对齐完整性与百分比，更无法直观审计残余 `.part` 文件的占用空间。

- **架构与协议设计(ADR-052 跨端多文件批量发送提议、断点续传可观测性画像与传输治理全闭环)**:
  1. **批量文件提议管道与强类型 DTO 设计 (`crates/tetherly-node/src/runtime.rs`)**:
     - 定义批量提议输入结构体 `BatchSendRequest`：
       ```rust
       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
       pub struct BatchSendRequest {
           pub peer: Option<String>,
           #[serde(default)]
           pub paths: Vec<String>,
           #[serde(default)]
           pub items: Vec<BatchSendItem>,
       }
       ```
     - 结构化子项 `BatchSendItem`（支持磁盘物理路径 `path` 或内存 Base64 负载 `data_base64`）；
     - 定义批次传输回报 `BatchSendReport` 与子项状态画像 `BatchTransferItemReport`：
       每个批次计算生成格式为 `tbt_<hex12>` 的确定性唯一批次标识，记录 `total_files`、`successful_files`、`failed_files`、`total_bytes` 以及每个传输项的 `transfer_id` 与传输状态（`"sent"` / `"failed"`）。
  2. **深度断点续传画像设计 (`ResumeMarkerDto` & `ResumeMarkersReport`)**:
     - 定义强类型画像 DTO：
       ```rust
       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
       pub struct ResumeMarkerDto {
           pub transfer_id: String,
           pub source: String,
           pub source_alias: Option<String>,
           pub total_bytes: u64,
           pub offset_bytes: u64,
           pub progress_percent: f64,
           pub chunk_aligned: bool,
           pub part_file_exists: bool,
           pub part_file_bytes: u64,
       }
       ```
     - 聚合回报结构体 `ResumeMarkersReport` 提供 `ok`、`count`、`total_partial_bytes` 与 `markers` 列表；并在 `StorageReport` 中并入 `resume_markers_count` 与 `resume_markers`，实现存储空间审计与续传标记深度联通。
  3. **运行时控制原语与死锁免疫治理**:
     - `send_files_batch(&self, peer: &DeviceId, paths: &[PathBuf]) -> Result<BatchSendReport, CoreError>`；
     - `send_batch_items(&self, peer: &DeviceId, items: Vec<BatchSendItem>) -> Result<BatchSendReport, CoreError>`；
     - `resume_markers_report(&self) -> ResumeMarkersReport`：双轨扫描内存 `ResumeTable` 与磁盘 `inbox/` 侧车文件 `.<transfer_id>.resume.json`（同时兼容 `.{id}.part` 与 `{id}.part`），自动校验并解析 `ResumeSidecar`（支持 `source`/`device_id` 与 `total`/`total_bytes` 别名互通），计算块对齐性（`offset % 65536 == 0`）与对端友好别名；
     - **互斥锁解耦**：彻底消除 `storage_report` 中因持有 `hubs` Mutex 再次调用 `resume_markers_report` 引发的线程不可重入自死锁（Self-Deadlock），将锁范围严格局限在局部扫描代码块内。
  4. **回环控制面端点群闭环 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `POST /api/file/send-batch` 与 `POST /api/file/batch`：接收 `BatchSendRequest`，通过 `resolve_peer` 智能解析目标对端，执行批次发送并返回结构化 `BatchSendReport`；
     - `GET /api/file/resume` 与 `GET /api/storage/resume`：轻量级输出当前全量断点续传标记画像 `ResumeMarkersReport`；
     - **严格安全红线**：100% 绑定 127.0.0.1，校验请求头 `Host: 127.0.0.1|localhost`，不匹配时坚决返回 403 Forbidden。
  5. **双端命令行 CLI 全同构升级 (`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - `send` 子命令：参数升级为 `paths: Vec<PathBuf>`（`num_args = 1..`），单次调用可传入多个文件路径；以 ASCII 表格展示批次概要（Batch ID、总数、成功/失败数、总字节）与各文件传输状态；
     - `resume` 子命令：查询 `/api/file/resume`，以 ASCII 表格展示传输 ID、来源设备、别名、总字节、续传偏移、百分比及分块对齐状态，支持 `--json` 输出。
  6. **Web 控制台多文件提议与续传徽标 (`ui/index.html`)**:
     - 文件选择控件加入 `multiple` 属性，选择多个文件后依次读取并通过 base64 批量提交至 `/api/file/send-batch`；
     - 在活动传输列表中识别 `is_resumed` 标记并渲染绿底 `Resumed` 状态徽标。
  7. **验证基线**:
     - `tests/phase1.rs` 增设 `batch_file_offer_pipeline_and_resume_observability_via_loopback_api` 集成测试；
     - 全工作区 341+ 项测试全绿，Linux aarch64 交叉编译通过，clippy 零告警，cargo deny 全检通过。

**1.58 相对 1.57(Linux BlueZ D-Bus GATT 传输栈架构与跨平台三端 ANCS 全闭环)**

- **动机(Linux 平台 GATT 传输栈缺失与三端全闭环最后一公里)**:
  在 Tetherly 陆续完成 ANCS 苹果通知服务全生命周期治理与事件总线闭环 (v1.56 / ADR-049)、macOS 原生 CoreBluetooth 传输栈与跨平台地址抽象 (v1.57 / ADR-050) 后，Tetherly 在 Windows（WinRT GATT）与 macOS（CoreBluetooth）均已具备原生硬件级 ANCS 接入能力。然而，在 **Linux 桌面与工作站平台（`#[cfg(all(unix, not(target_os = "macos")))]`）** 上，底层传输层仍存在显著断层：
  1. **Linux 平台传输栈缺失（Missing Linux GATT Transport）**：
     `crates/tetherly-node/src/ancs.rs` 中针对 Linux 依然硬编码返回 `Ok(None)`，守护进程在 Linux 上启动时若传入 `--ancs <addr>`，只能打印 `ANCS is not implemented on this platform; iPhone notifications stay off` 并直接丢弃通知接入能力；
  2. **三端架构异构统一的最后一公里**：
     - Windows 采用 WinRT MTA 异步驱动 (`win_gatt.rs`)；
     - macOS 采用 CoreBluetooth + GCD 串行调度队列驱动 (`macos.rs`)；
     - Linux 平台缺乏对应的标准 BlueZ D-Bus GATT 驱动；
     需要依托 Linux 标准 System D-Bus（`org.bluez`）接口，实现基于纯 Rust 异步 D-Bus 客户端（`zbus`）的高性能自包含传输栈，完成 Windows / macOS / Linux 三大桌面平台 ANCS GATT 驱动的 100% 同构全闭环。

- **架构与协议设计(ADR-051 Linux BlueZ D-Bus GATT 传输栈架构与跨平台三端 ANCS 全闭环)**:
  1. **零 Lockfile 扰动纯 Rust D-Bus 依赖架构 (`crates/tetherly-node/Cargo.toml`)**:
     - 在 `[target.'cfg(unix)'.dependencies]` 中配置 `zbus`：
       ```toml
       [target.'cfg(unix)'.dependencies]
       # D-Bus client for BlueZ GATT ANCS (src/ancs/bluez.rs). Reuses zbus 4.4.0 already in Cargo.lock.
       zbus = { version = "4", default-features = false, features = ["tokio"] }
       ```
     - 彻底复用 `secret-service` 锁定的 `zbus 4.4.0`，杜绝任何外部 C 依赖与 AGPL 协议违规，`deny.toml` 100% 检验通过。
  2. **前端契约与线程隔离架构 (`crates/tetherly-node/src/ancs/bluez.rs`)**:
     - 前端结构体 `BluezAncsTransport` 仅持有 `std::sync::mpsc::Sender<Cmd>`，天然满足 `Send + Sync` 契约；
     - 内部同步方法在 `ACK_TIMEOUT = Duration::from_secs(10)` 内等待后台响应，超时映射为 `CoreError::AncsSubscribe` 或 `CoreError::AncsWrite`；
     - 实现 `Drop` 自动向后台派发 `Cmd::Stop` 信号触发优雅退出；
     - 后台专用线程 `"tetherly-ancs-bluez"` 拥有独立的单线程 Tokio Runtime，避免与上层运行时冲突死锁。
  3. **BlueZ D-Bus 对象树拓扑与 48 位 MAC 寻址 (`dev_path_from_mac`)**:
     - 提供标准转换函数：
       ```rust
       pub fn dev_path_from_mac(mac: &[u8; 6]) -> String {
           format!(
               "/org/bluez/hci0/dev_{:02X}_{:02X}_{:02X}_{:02X}_{:02X}_{:02X}",
               mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
           )
       }
       ```
     - 连接系统 D-Bus (`Connection::system()`)，通过 `org.freedesktop.DBus.ObjectManager.GetManagedObjects` 检索设备路径及其子路径下的 ANCS 特征。
  4. **严格恪守 SSOT §9.4 硬约束**:
     - 严格按顺序订阅：首先订阅 Data Source (`StartNotify`)，再订阅 Notification Source (`StartNotify`)；
     - 监听 D-Bus `PropertiesChanged` 信号，解析 `ay`（字节数组）属性推入 `AncsInbound::NotificationSource` 与 `AncsInbound::DataSource`，严禁在信号监听线程反向写入 Control Point；
     - Control Point 写入命令通过 BlueZ `WriteValue(bytes, {"type": "request"})` 严格串行执行并等待 ATT Write Response 确认；
     - 监听设备 `Connected == false` 突变推入 `AncsInbound::Disconnected`，退出驱动循环触发指数退避重连 (`ancs_backoff_delay`)。
  5. **跨平台统一装配与路由 (`crates/tetherly-node/src/ancs.rs`)**:
     - Linux 平台 (`#[cfg(all(unix, not(target_os = "macos")))]`) 正式接入 `bluez::open(address).map(Some)`，达成三端原生 GATT 栈全闭环。

**1.57 相对 1.56(macOS ANCS CoreBluetooth 传输栈架构、跨平台外设地址抽象与守护进程链路修复)**

- **动机(守护进程启动链路缺陷、MAC vs UUID 寻址断层与 macOS 原生 GATT 栈缺失)**:
  在 Tetherly 陆续完成 ANCS 苹果通知服务全生命周期治理与事件总线闭环 (v1.56 / ADR-049)、网络拓扑透视 (v1.55 / ADR-048)、远程屏幕与键鼠治理 (v1.54 / ADR-047) 后，底层 **ANCS 真实物理传输栈**与**跨平台外设寻址**暴露出三个核心缺陷：
  1. **守护进程启动链路缺陷（Daemon Startup Bug）**：
     - 在 `crates/tetherly-node/src/bin/tetherly.rs` 的 `Cmd::Run` 中，用户指定 `--ancs <addr>` 时，调用 `tetherly_node::ancs::open(address)` 成功返回 `(transport, inbound)`，然而 `cfg.ancs` 从未被赋值（仍为 `None`），`cfg.ancs_peripheral` 从未记录该地址；`_transport` 被下划线前缀悄然遗弃。导致 `Node::start(cfg)` 初始化内部状态时 `self.shared.ancs` 恒为 `None`。随后调用 `node.spawn_ancs_pump(inbound)` 即使收到 `AncsInbound::Ready`，`self.ancs_connect()` 与所有后续 `ancs_feed` 调用均因 `shared.ancs == None` 而静默返回 `Ok(None)`，守护进程的 ANCS 实际链路在生产中完全无法工作；
  2. **跨平台外设寻址断层（Addressing Mismatch: MAC vs UUID）**：
     - Windows WinRT GATT 暴露 48 位物理 MAC 地址（形如 `11:22:33:44:55:66` 或 `11-22-33-44-55-66`）；
     - macOS CoreBluetooth 为了隐私与安全，操作系统底层严格隐藏硬件 MAC 地址，仅对外暴露 128 位 `NSUUID`（形如 `9F82A2E6-B9C0-4E90-B8E5-3A21F5A67C43`）；
     - 既有地址解析逻辑硬编码校验 12 个十六进制字符（48 位），在 macOS 传入 UUID 时会直接报错拒收；
  3. **macOS CoreBluetooth 传输栈缺失**：
     - 非 Windows 平台硬编码返回 `Ok(None)`；需要建立符合 macOS CoreBluetooth 规范的传输栈架构与原生驱动桥接，支持通过 128 位外设 UUID 检索与建立 GATT 连接。

- **架构与协议设计(ADR-050 macOS ANCS CoreBluetooth 传输栈架构、跨平台外设地址抽象与守护进程链路修复)**:
  1. **跨平台外设地址抽象与解析器 (`crates/tetherly-core/src/ancs.rs` & `lib.rs`)**:
     - 定义强类型 `BleAddress` 枚举：
       ```rust
       #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
       pub enum BleAddress {
           Mac([u8; 6]),
           Uuid([u8; 16]),
       }
       ```
     - 实现通用解析器 `parse_ble_address(text: &str) -> Result<BleAddress, CoreError>`：
       - 自动识别 48 位 MAC 地址格式（支持冒号、连字符或裸 12 位十六进制）；
       - 自动识别 128 位 UUID 格式（支持 RFC 4122 标准连字符格式或裸 32 位十六进制）；
       - 非法长度、非法字符或格式不符返回 `CoreError::AncsFrame("invalid bluetooth address format")`；
     - 在 `BleAddress` 上实现规范化字符串格式化方法 `to_canonical_string(&self) -> String` 与属性检索原语。
  2. **守护进程启动链路与状态机所有权修复 (`crates/tetherly-node/src/bin/tetherly.rs`)**:
     - 彻底修复 `Cmd::Run` 中丢弃 `transport` 的缺陷：
       ```rust
       let ancs_link = match ancs.as_deref() {
           Some(address) => match tetherly_node::ancs::open(address) {
               Ok(Some(link)) => Some((address.to_string(), link)),
               Ok(None) => {
                   info!("ANCS is not implemented on this platform; iPhone notifications stay off");
                   None
               }
               Err(e) => {
                   warn!(%e, "ANCS address {address} is malformed; iPhone notifications stay off");
                   None
               }
           },
           None => None,
       };

       let (ancs_addr, inbound) = match ancs_link {
           Some((addr, (transport, inbound))) => {
               cfg.ancs = Some(transport);
               cfg.ancs_peripheral = addr.clone();
               (Some(addr), Some(inbound))
           }
           None => (None, None),
       };

       let node = Node::start(cfg)?;
       if let Some(inbound) = inbound {
           node.spawn_ancs_pump(inbound);
           info!(peripheral = ?ancs_addr, "ANCS link up; pair the iPhone in system Bluetooth settings if you have not");
       }
       ```
  3. **macOS CoreBluetooth 传输栈构建 (`crates/tetherly-node/src/ancs/macos.rs` & `ancs.rs`)**:
     - 在 `crates/tetherly-node/Cargo.toml` 中配置 macOS 依赖项：
       ```toml
       [target.'cfg(target_os = "macos")'.dependencies]
       dispatch2 = "0.3"
       objc2-core-bluetooth = { version = "0.3", default-features = false, features = [
           "std",
           "CBCentralManager",
           "CBPeripheral",
           "CBService",
           "CBCharacteristic",
           "CBUUID",
       ] }
       ```
     - 在 `crates/tetherly-node/src/ancs/macos.rs` 中设计自包含传输栈：
       - 前端结构体 `MacAncsTransport` 实现 `AncsTransport`（`subscribe_data_source`, `subscribe_notification_source`, `write_control_point`），持有 `Sender<Cmd>`，带有 10 秒超时 `ACK_TIMEOUT`；
       - 后台专用线程 `"tetherly-ancs-cb"` 运行 GCD 串行队列 (`tetherly.ancs.queue`)，托管 `CBCentralManager` 与 `CBPeripheral`；
       - 连接已配对的 iPhone 外设（基于 128 位 UUID），解析 ANCS 服务与 3 个特征（Notification Source、Data Source、Control Point）；
       - `setNotifyValue(true)` 订阅通知源与数据源；值变更仅复制字节并推入 `AncsInbound`；
       - 控制点通过 `writeValue:forCharacteristic:type:CBCharacteristicWriteWithResponse` 严格串行写入并确认；
       - 在外设断开时推入 `AncsInbound::Disconnected` 并触发退避重连。
  4. **运行时可观测性与控制面透视 (`crates/tetherly-node/src/runtime.rs` & `bin/tetherly.rs`)**:
     - `AncsReport` 扩充 `pub peripheral: Option<String>` 字段；
     - 节点状态打印展示 `rep.ancs_peripheral`。
  5. **端到端集成与回归验证 (`tests/phase4.rs` & `tetherly-core`)**:
     - 增设 `test_ble_address_and_daemon_ancs_wiring_regression`：
       - 验证 48 位 MAC 地址解析（带冒号、带连字符、无分隔符）；
       - 验证 128 位 UUID 地址解析（带连字符、无连字符、大小写不敏感）；
       - 验证非法格式拦截；
       - 验证 `Node::start` 携带 `cfg.ancs` 时的状态机初始化与 `ancs_report()` 外设地址透传；
       - 验证 `node.spawn_ancs_pump` 驱动 `AncsInbound::Ready` 后状态机成功跃迁至 `ready` 状态。

**1.56 相对 1.55(ANCS 苹果通知服务全生命周期治理、白名单规则动态增删查、状态机事件总线闭环与双端 CLI 全同构)**

- **动机(ANCS 状态流转事件盲区、白名单不可变性缺陷、缺乏结构化强类型 DTO 与控制面闭环缺失)**:
  在 Tetherly 陆续完成网络拓扑透视 (v1.55 / ADR-048)、远程屏幕与键鼠治理 (v1.54 / ADR-047)、文件提议治理 (v1.53 / ADR-046)、剪贴板审计 (v1.52 / ADR-045) 后，底层 **ANCS 苹果通知中心服务 (Apple Notification Center Service, GATT Ingress)** 与 **本地打开应用白名单治理 (`OpenAllowlist`)** 暴露出一系列不可忽视的治理痛点：
  1. **事件总线静默脱节（Event Blackout on ANCS Transitions & Allowlist Mutations）**：
     - 当 ANCS 客户端在 `Idle`、`Ready`、`Backoff` 之间流转（如 BLE GATT 链路重连或退避等待），或者用户在控制面动态新增规则、移除规则、一键清空白名单时，底层从未向节点事件总线广播任何 `UiEvent`。导致 Web UI（SSE `EventSource`）和 CLI 事件监听器（`tetherly events`）处于完全失明状态，严重违背 Tetherly 全协议可观测性准则；
  2. **白名单单向只增不减与治理原语缺失（Allowlist Immutability Defect）**：
     - `crates/tetherly-core/src/allowlist.rs` 中的 `OpenAllowlist` 此前仅提供 `push` 追加操作，缺乏按 `app_id` 精准移除单条规则（`remove`）与全量清空规则（`clear`）的原语，导致用户配置错误规则后无法撤销或重置，造成潜在的恶意 URL scheme 驻留风险；
  3. **控制面缺乏结构化强类型 DTO 抽象（Missing Strongly-Typed DTOs）**：
     - `/api/ancs` 接口此前返回内联构造的松散 JSON，缺乏类型严谨的 `OpenRuleDto`、`AncsConnectReport`、`AncsDisconnectReport`、`AncsRuleReport`、`AncsClearRulesReport` 统一结构定义，未在 `lib.rs` 对外导出，导致 CLI 与外部集成客户端无法获得严格的类型校验保证；
  4. **终端全同构控制与局部渲染闭环缺失**：
     - CLI 缺乏 `ancs disconnect`、`ancs remove`、`ancs clear` 等核心治理子命令；Web UI 缺少主动断开、行内删除规则与一键清空按键，且未基于 SSE 事件总线实施局部就绪刷新。

- **架构与协议设计(ADR-049 ANCS 苹果通知服务全生命周期治理、白名单规则动态增删查、状态机事件总线闭环与双端 CLI 全同构)**:
  1. **核心白名单原语扩展与 URL 注入守卫 (`crates/tetherly-core/src/allowlist.rs` & `error.rs`)**:
     - `OpenRule` 派生 `Serialize, Deserialize, PartialEq, Eq`，便于直接序列化传输；
     - `OpenAllowlist` 扩展 `remove(&mut self, app_id: &str) -> Option<OpenRule>` 与 `clear(&mut self) -> usize` 原语；
     - 严格遵循 ADR-008「打开应用的 URL 只来自本机规则」，仅允许形如 `scheme://` 的本地自定义协议，拒绝 `http://`、`https://`、`file://` 等可能被外部恶意 payload 诱导打开的非本地 Scheme；
     - 在核心错误定义中确立 `CoreError::NoAncsTransport` 变体，保证在未附着 GATT 传输层时连接操作返回符合契约的标准错误响应。
  2. **事件总线扩展与安全红线守卫 (`crates/tetherly-node/src/events.rs`)**:
     - 新增结构化变体 `UiEvent::AncsStateChanged { state: String, attached: bool, retry_at_ms: Option<u64> }`；
     - 新增结构化变体 `UiEvent::AncsRuleChanged { action: String, app_id: Option<String>, url: Option<String>, rules_count: usize }`；
     - **严格遵守安全红线铁律（Zero-Plaintext Invariant）**：事件载荷绝对禁止携带任何苹果通知消息正文、标题或敏感属性，仅传递连接状态 (`state`)、附着标记 (`attached`)、退避重试时间戳 (`retry_at_ms`)、操作动作 (`action`)、目标应用标识 (`app_id`)、跳转 URL (`url`) 与当前总规则数 (`rules_count`)；
     - 提供 `ancs_state_changed` 与 `ancs_rule_changed` 构造辅助函数及关联方法。
  3. **结构化 DTO 抽象与运行时控制原语 (`crates/tetherly-node/src/runtime.rs`, `lib.rs`)**:
     - `OpenRuleDto`：封装 `app_id` 与 `url`；
     - `AncsConnectReport`：封装 `ok`、`state`、`retry_at_ms` 与 `message`；
     - `AncsDisconnectReport`：封装 `ok`、`state`、`retry_at_ms` 与 `message`；
     - `AncsRuleReport`：封装 `ok`、`action: "added" | "removed" | "not_found"`、`rule: Option<OpenRuleDto>` 与 `total_rules: usize`；
     - `AncsClearRulesReport`：封装 `ok`、`action: "cleared"`、`cleared_count: usize` 与 `total_rules: usize`；
     - `AncsReport` 升级：包含 `rules: Vec<OpenRuleDto>` 及兼容旧版的 `allowlist: Vec<String>`；
     - 新增核心控制原语 `ancs_connect_reported`、`ancs_disconnect_reported`、`add_open_rule_reported`、`remove_open_rule_reported`、`clear_open_rules_reported`，在发生状态流转或白名单变更时全生命周期广播 `UiEvent`。
  4. **轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/ancs` 重构为返回强类型 `AncsReport`；
     - `POST /api/ancs/connect` 返回强类型 `AncsConnectReport`；
     - `POST /api/ancs/disconnect` 返回强类型 `AncsDisconnectReport`；
     - `POST /api/ancs/allow` 及别名 `POST /api/ancs/rules/add` 支持添加白名单规则并返回强类型 `AncsRuleReport`；
     - `DELETE /api/ancs/allow` 及别名 `POST /api/ancs/rules/remove` 与 `DELETE /api/ancs/rules` 支持通过 Query 或 JSON 体删除指定规则并返回强类型 `AncsRuleReport`；
     - `POST /api/ancs/rules/clear` 及别名 `DELETE /api/ancs/rules/clear` 支持一键清空规则并返回强类型 `AncsClearRulesReport`；
     - 严格遵循 `127.0.0.1` 回环绑定与 `Host` 头守卫，拦截非回环请求并返回 HTTP 403 Forbidden (`loopback only`)。
  5. **CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs`, `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - `ancs` 命令族扩展 `status`、`connect`、`disconnect`、`allow`、`remove`、`clear` 子命令；
     - 保持与现有输出及 `--json` 管道的完美兼容性，清晰打印当前 ANCS 附着状态、连接阶段、退避时间戳与白名单规则表；
     - SSE `events` 监听器新增对 `[ancs_state_changed]` 与 `[ancs_rule_changed]` 的格式化着色打印。
  6. **Web UI 实时动态刷新与状态徽标 (`ui/index.html`)**:
     - 在「Apple 通知服务 (ANCS)」面板新增「断开 ANCS」操作按键、规则列表行内即时删除标签 (`×`) 与「清空白名单」按钮；
     - 实现 `updateAncsStatus()` 轻量级局部渲染，解耦全局状态轮询；
     - 在 SSE 事件流中捕获 `ancs_state_changed` 与 `ancs_rule_changed` 事件，驱动 ANCS 卡片毫秒级局部刷新。
  7. **端到端集成测试全覆盖 (`tests/phase4.rs`)**:
     - 增设 `ancs_lifecycle_governance_rule_management_and_events_via_loopback_api`：
       - 验证非回环 Host 请求（`Host: evil.attacker.com`）被强制掐断拦截并返回 403 Forbidden；
       - 验证 SSE `/api/events` 流式监听就绪；
       - 验证 `GET /api/ancs` 初始状态解析与规则表初态；
       - 验证 `POST /api/ancs/connect` 触发连接并捕获 `UiEvent::AncsStateChanged`；
       - 验证 `POST /api/ancs/allow` 与 `POST /api/ancs/rules/add` 添加规则并捕获 `UiEvent::AncsRuleChanged`；
       - 验证 `DELETE /api/ancs/allow` 移除指定规则并捕获 `UiEvent::AncsRuleChanged`；
       - 验证移除不存在规则返回 `action: "not_found"`；
       - 验证 `POST /api/ancs/rules/clear` 清空全部规则并捕获清空事件；
       - 验证 `POST /api/ancs/disconnect` 断开连接并捕获退避重试事件；
       - 验证节点优雅停机。

**1.55 相对 1.54(网络拓扑全景透视、网卡物理分类动态重探治理、EasyTier 侧车穿透可观测性与事件总线闭环)**

- **动机(网络拓扑事件盲区、网卡热插拔动态探测缺失、EasyTier 侧车状态突变脱节与控制面缺乏强类型 DTO)**:
  在 Tetherly 陆续完成远程屏幕与键鼠治理 (v1.54 / ADR-047)、文件提议治理 (v1.53 / ADR-046)、剪贴板审计 (v1.52 / ADR-045)、通知双向同步 (v1.51 / ADR-044) 后，底层**网络接口拓扑与 EasyTier 侧车穿透可观测性**暴露出一系列治理痛点：
  1. **事件总线静默脱节（Event Blackout on Network & Overlay Changes）**：
     - 当宿主机经历网卡热插拔、WiFi/以太网热切换、VPN 拨号建立/断开，或者 EasyTier 侧车守护进程完成启动、穿透建立、节点发现或异常离线时，底层从不向节点事件总线广播任何 `UiEvent`。导致 Web UI（SSE `EventSource`）和 CLI 事件监听器（`tetherly events`）完全处于失明状态，必须依赖高开销的主动轮询；
  2. **网卡物理分类静态锁定与动态重探能力缺失（Dynamic Rescan Primitive Missing）**：
     - 节点启动时虽然基于 `lan.rs` 进行了网卡分类，但缺乏在不重启节点的前提下主动触发「即时全量网卡重探与 EasyTier 侧车再连接」的控制原语；
  3. **控制面缺乏结构化强类型 DTO 抽象（Missing Strongly-Typed DTOs）**：
     - `/api/net` 接口此前返回内联构造的松散 JSON，缺乏类型严谨的 `NetworkReport`、`IfaceReportDto`、`OverlayStatusReport` 与 `NetworkRefreshReport` 统一结构定义，未在 `lib.rs` 对外导出，导致 CLI 与外部集成客户端无法获得严格的类型校验保证；
  4. **终端全同构控制与局部渲染闭环缺失**：
     - CLI 缺乏 `net --refresh` 显式重探管道；Web UI 缺乏网络重探控制按钮与解耦全局轮询的局部即时渲染支持。

- **架构与协议设计(ADR-048 网络拓扑全景透视、网卡物理分类动态重探治理、EasyTier 侧车穿透可观测性与事件总线闭环)**:
  1. **事件总线扩展与安全红线守卫 (`crates/tetherly-node/src/events.rs`)**:
     - 新增结构化变体 `UiEvent::NetworkRefreshed { iface_count: usize, listen_count: usize, overlay_present: bool, overlay_source: Option<String> }`；
     - 新增结构化变体 `UiEvent::OverlayStateChanged { present: bool, source: String, local_ips: Vec<String>, peer_count: usize }`；
     - **严格遵守安全红线铁律（Zero-Plaintext Invariant）**：事件载荷绝对禁止泄露私网敏感信息，仅传递网卡分类统计、监听计数、侧车状态及对端节点数量；
     - 提供 `network_refreshed` 与 `overlay_state_changed` 构造辅助函数及关联方法。
  2. **结构化 DTO 抽象与运行时控制原语 (`crates/tetherly-node/src/runtime.rs`, `lib.rs`)**:
     - `IfaceReportDto`：封装 `iface`、`ip`、`listen` 与 `reason`；
     - `OverlayStatusReport`：封装 `enabled`、`present`、`source`、`local_ips`、`peer_count` 与 `peers`；
     - `NetworkReport`：封装 `ok`、`interfaces` (兼容 `ifaces`) 与 `overlay: OverlayStatusReport`；
     - `NetworkRefreshReport`：封装 `ok`、`action: "refreshed"`、`ifaces_scanned`、`ifaces_listening`、`overlay_present`、`overlay_source`、`overlay_local_ips`、`overlay_peers_count` 与 `timestamp_ms`；
     - 新增核心控制原语 `node.network_report_dto() -> NetworkReport` 与 `node.refresh_network().await -> NetworkRefreshReport`；
     - 运行时主动执行物理网卡重新枚举与 EasyTier 侧车连接探测，更新 `Shared.overlay`，并在发生状态突变时广播 `UiEvent::OverlayStateChanged`，在刷新完成时广播 `UiEvent::NetworkRefreshed`；
     - 后台 `overlay_scan_loop` 同步在侧车状态演进时派发 `UiEvent::OverlayStateChanged`。
  3. **轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/net` 重构为返回强类型 `NetworkReport`；
     - 新增 `POST /api/net/refresh` 及别名 `POST /api/net/probe`，触发主动重探并返回强类型 `NetworkRefreshReport`；
     - 严格遵循 `127.0.0.1` 回环绑定与 `Host` 头守卫，拦截非回环请求并返回 HTTP 403 Forbidden (`loopback only`)。
  4. **CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs`, `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - `tetherly net` 与 `tetherly-cli net` 增设 `--refresh` (`-r`) 参数，支持主动触发网卡分类与 EasyTier 侧车重探；
     - 保持与现存输出及 `--json` 管道的完美后向兼容性，清晰打印分类网卡、监听状态、跳过原因、EasyTier 侧车状态、虚拟 IP 与 Peers；
     - SSE `events` 监听器新增对 `[network_refreshed]` 与 `[overlay_state_changed]` 的格式化着色打印。
  5. **Web UI 实时动态刷新与状态徽标 (`ui/index.html`)**:
     - 在「网络与接口诊断」面板新增「探测刷新」(`netrefreshbtn`) 按钮与异步操作状态反馈；
     - 实现 `updateNetworkStatus()` 轻量级局部渲染，解耦全局状态轮询；
     - 在 SSE 事件流中捕获 `network_refreshed` 与 `overlay_state_changed` 事件，驱动网络与侧车卡片毫秒级自动就绪刷新。
  6. **端到端集成测试全覆盖 (`tests/phase2.rs`)**:
     - 增设 `dynamic_network_rescan_and_overlay_events_via_loopback_api`：
       - 验证双端网卡分类与初始 EasyTier 虚拟网卡状态；
       - 验证 SSE `/api/events` 流式监听就绪；
       - 验证 `GET /api/net` 强类型 `NetworkReport` 反序列化与 loopback 监听判定；
       - 验证非回环 Host 请求（`Host: evil.attacker.com`）被强制掐断拦截并返回 403 Forbidden；
       - 验证调用 `POST /api/net/refresh` 与 `POST /api/net/probe` 触发主动重探并捕获 `UiEvent::NetworkRefreshed`；
       - 验证直接调用 Rust API `node.refresh_network().await` 正确性与事件派发闭环；
       - 验证节点优雅停机。

**1.54 相对 1.53(远程屏幕与键鼠共享全生命周期治理、实时遥测可观测性与事件总线闭环)**

- **动机(事件总线静默脱节、缺乏结构化遥测报告 DTO、活动对端追踪盲区与可观测性断层)**:
  在 Tetherly 陆续完成文件提议治理 (v1.53 / ADR-046)、剪贴板审计 (v1.52 / ADR-045)、通知双向同步 (v1.51 / ADR-044)、黑名单治理 (v1.50 / ADR-043) 后，核心多媒体与控制通道——**远程屏幕串流 (Phase 5 / 45720) 与键鼠输入共享 (Phase 3 / 45719)** 暴露出状态黑盒与事件盲区：
  1. **事件总线静默脱节（Event Blackout）**：
     - 当远程屏幕在 `Idle`、`Requested`、`Allowed`、`Streaming` 间流转，或者键鼠共享连线/断开/焦点在 `Local` 与 `Remote` 之间切换（`input_seat`）时，底层从不向节点事件总线广播任何 `UiEvent`。导致 Web UI（SSE `EventSource`）和 CLI 事件监听器（`tetherly events`）完全处于失明状态；
  2. **缺乏结构化遥测报告 DTO（Missing Typed Reports）**：
     - `/api/screen` 与 `/api/input` 接口目前仅返回松散的内联 JSON，缺乏类型严谨的 `ScreenStatusReport`、`ScreenControlReport`、`InputStatusReport`、`InputControlReport` 结构化定义，未在 `lib.rs` 中对外导出；
  3. **活动对端与双向遥测透视缺失（Peer Tracking & Telemetry Blindspots）**：
     - Host 端未显式记录当前连入的受信任访客（Visitor），访客端未显式记录当前查看的宿主（Host），控制器与受控端未记录焦点对端 ID；发送端与接收端度量指标未统一归一化；
  4. **终端全同构控制与可观测性闭环缺失**：
     - CLI 缺少对结构化报告的整洁呈现与统一 `--json` 管道输出，SSE 事件监听器缺少 `[screen_state_changed]` 与 `[input_seat_changed]` 格式化高亮。

- **架构与协议设计(ADR-047 远程屏幕与键鼠共享全生命周期治理、实时遥测可观测性与事件总线闭环)**:
  1. **事件总线扩展与安全红线守卫 (`crates/tetherly-node/src/events.rs`)**:
     - 新增结构化变体 `UiEvent::ScreenStateChanged { state: String, allowed: bool, peer: Option<String> }`；
     - 新增结构化变体 `UiEvent::InputSeatChanged { seat: String, channel_open: bool, peer: Option<String> }`；
     - **严格遵守安全红线铁律（Zero-Plaintext Invariant）**：事件载荷绝对禁止携带屏幕像素缓冲区、按键扫描码、字符文本或坐标敏感轨迹，仅携带状态标记、席位标识与对端 ID；
     - 提供 `screen_state_changed` 与 `input_seat_changed` 构造辅助函数及 Associated Functions。
  2. **结构化 DTO 抽象与运行时控制原语 (`crates/tetherly-node/src/runtime.rs`, `lib.rs`)**:
     - `ScreenStatusReport`：封装 `ok`、`state`、`channel_open`、`allowed`、`sent`、`received`、`dropped`、`refused`、`bytes`、`rx_bytes`、`p95_frame_interval_us` 与 `active_peer`；
     - `ScreenControlReport`：封装 `ok`、`action`、`state`、`allowed`、`peer` 与 `timestamp_ms`；
     - `InputStatusReport`：封装 `ok`、`channel_open`、`seat` 与 `active_peer`；
     - `InputControlReport`：封装 `ok`、`action`、`seat`、`channel_open`、`peer` 与 `timestamp_ms`；
     - 在 `Shared` 中维护活动对端互斥锁 `screen_peer: Mutex<Option<DeviceId>>` 与 `input_peer: Mutex<Option<DeviceId>>`；
     - 增设 `screen_status_report`、`screen_allow_reported`、`screen_revoke_reported`、`screen_start_reported`、`screen_stop_reported`、`input_status_report`、`input_start_reported` 与 `input_stop_reported` 原语；
     - 在屏幕连接、授权许可、授权撤销、推流启动与停止、键鼠连接断开与边缘跨越切换时全生命周期广播 `UiEvent`。
  3. **轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/screen` 返回强类型 `ScreenStatusReport`；
     - `POST /api/screen/allow` 与 `POST /api/screen/revoke` 返回强类型 `ScreenControlReport`；
     - `POST /api/screen/start` 与 `POST /api/screen/stop` 返回强类型 `ScreenControlReport`；
     - `GET /api/input` 返回强类型 `InputStatusReport`；
     - `POST /api/input/start` 与 `POST /api/input/stop` 返回强类型 `InputControlReport`；
     - 严格 127.0.0.1 回环绑定与 `Host` 校验（403 on mismatch）。
  4. **CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs`, `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - `screen status` 与 `input status` 支持富文本排版与 `--json` 原生管道输出；
     - `screen allow/revoke/start/stop` 与 `input start/stop` 打印规范操作确认信息与当前对端；
     - `events` 监听器新增对 `[screen_state_changed]` 与 `[input_seat_changed]` 的格式化着色打印。
  5. **Web UI 实时动态刷新与状态徽标 (`ui/index.html`)**:
     - 在 SSE 事件总线分发循环中挂载 `screen_state_changed` 与 `input_seat_changed` 监听器；
     - 解耦重量级全节点快照 `render(s)`，支持屏幕与键鼠卡片根据 SSE 事件驱动局部即时渲染。
  6. **端到端集成测试全覆盖 (`tests/phase5.rs`)**:
     - 增设 `screen_and_input_lifecycle_governance_and_events_via_loopback_api`：
       - 验证初始屏幕与键鼠状态透视；
       - 验证访客握手连入触发 `Requested` 状态及活动对端记录；
       - 验证宿主调用 `/api/screen/allow` 派发 `ScreenStateChanged` 事件；
       - 验证屏幕串流启动、通道激活与双向数据流转遥测；
       - 验证宿主调用 `/api/screen/revoke` 撤销授权与事件派发；
       - 验证键鼠通道连线、`InputSeatChanged` 事件派发与优雅断开；
       - 验证节点优雅停机。

- **动机(文件提议状态黑盒、重载全节点快照开销、缺乏轻量级提议透视、批量决策原语缺失、受信任对端自动接收空白及事件生命周期脱节)**:
  在 Tetherly v1.52 实现了跨端剪贴板生命周期治理、元数据遥测审计与历史可观测性闭环后，数据协同与安全体系得到大幅提升。但在核心应用层的**跨端文件传输系统（File Ingress & Offer Lifecycle）**中，暴露出**提议状态黑盒、轻量级提议可观测性断层、批量决策能力缺失与无人值守自动化空白**：
  1. **待确认文件提议缺乏轻量级透视端点（Heavy Snapshot vs Lightweight Offers Introspection）**：
     - 此前客户端或外部工具若要感知「当前有哪些待确认的接收提议」，必须调用重量级的全节点状态快照 `GET /api/state`。该接口会序列化整个节点的所有会话、候选通知、路由表、全局指标与剪贴板状态，在轮询与低资源场景下开销巨大；
     - 缺乏专用的 `GET /api/file/offers`，无法针对单一提议执行按 `transfer_id` 的 $O(1)$ 精准透视，亦无法按来源对端 `peer` 进行定向筛选；
  2. **提议决策动作缺乏零明文结构化报告（Zero-Plaintext Structured Decision Reports）**：
     - 现存接收（`POST /api/file/accept`）与拒绝（`POST /api/file/reject`）接口返回松散字典，未提供可追溯的 `FileDecisionReport` 与 `FileBatchDecisionReport`；控制端无法审计被确认或拒绝的文件名、字节大小、断点续传偏移与来源对端；
  3. **缺乏面向生产环境的批量提议治理原语（Batch Decision Governance）**：
     - 当对端批量投递多份文件，或多个协作设备同时发起传输时，用户或守护进程缺乏一键「全部接收」（`--all`）、「全部拒绝」或按特定设备定向批处理的能力；
  4. **无人值守与受信任对端自动接收策略空白（Auto-Accept Policy for Trusted Peers）**：
     - 在 NAS、无头运行（Headless Linux/macOS/Windows Daemon）或已严格通过 Noise 密钥配对的个人多设备协同场景下，每次对端发来文件都需要人工干预调用 API 确认，极大阻碍了无人值守工作流；
     - **严格遵守安全红线铁律（Verbatim Security Red Line）**：「信任库只在 Noise transport 成功后写」与「仅允许受信任对端触发自动接收」。因此，自动接收策略必须严格受限于 `is_trusted(peer) == true`，严禁未经鉴权的外部设备绕过确认机制；
  5. **事件总线生命周期缺失（Missing Event Bus Lifecycle for Rejection & Policy Change）**：
     - 当文件提议被拒绝（本地拒绝或对端拒绝）时，系统此前处于完全静默状态，从未派发任何 `UiEvent`；同时自动接收策略变更亦未在事件流中广播，导致 Web UI、SSE 订阅客户端与 CLI 监听器无法感知；
  6. **双端 CLI 与 Web UI 全同构控制闭环缺失**：
     - `tetherly` 与 `tetherly-cli` 缺少 `offers`、`accept`、`reject`、`auto-accept` 命令群；Web UI 缺少待确认文件治理面板与自动接收配置开关。

- **架构与协议设计(ADR-046 跨端文件传输提议生命周期治理、轻量级提议透视与终端全同构控制闭环)**:
  1. **结构化 DTO 报告设计 (`crates/tetherly-node/src/runtime.rs`, `lib.rs`)**:
     - `FileOfferDto`：封装 `transfer_id`、`filename`、`total_bytes`、`resume_from`、`source` 与 `timestamp_ms`；
     - `FileOffersReport`：封装 `ok`、`count`、`offers: Vec<FileOfferDto>` 与 `auto_accept_trusted: bool`；
     - `FileDecisionReport`：封装单项决策结果 `transfer_id`、`filename`、`total_bytes`、`resume_from`、`peer`、`accepted` 与 `timestamp_ms`；
     - `FileBatchDecisionReport`：封装批量决策结果 `accepted_count` / `rejected_count`、`total_evaluated` 与 `decisions` 列表；
     - `AutoAcceptReport`：封装自动接收策略状态 `enabled`、`auto_accepted_count` 与 `target: "trusted_peers_only"`。
  2. **事件总线扩展 (`crates/tetherly-node/src/events.rs`)**:
     - 新增 `UiEvent::FileRejected { transfer_id, source, reason }` 与 `UiEvent::FileAutoAcceptChanged { enabled }`。
  3. **运行时控制原语与受信对端自动接收策略 (`crates/tetherly-node/src/runtime.rs`)**:
     - 在 `Shared` 状态中增设原子控制位 `auto_accept_trusted: AtomicBool` 与原子计数器 `auto_accepted_count: AtomicU64`；
     - 实现 `Node::is_trusted(&self, peer: &str) -> bool`，基于 `trust_table` 判定 Noise 鉴权受信状态；
     - 在 `SessionEvent::FileOffer` 核心处理循环中集成自动接收判定：若 `auto_accept_trusted` 开启且 `is_trusted(peer)`，则自动触发异步 `user_accept_file` 并将 `auto_accepted_count` 递增；
     - 在 `SessionEvent::FileReject` 与本地主动拒绝时广播 `UiEvent::FileRejected`；
     - 增设 `file_offers`、`user_accept_file_reported`、`user_reject_file_reported`、`batch_accept_files`、`batch_reject_files`、`auto_accept_status` 与 `set_auto_accept` 原语。
  4. **轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/file/offers`：支持按 `?id=tfl_...`（404 on missing）与 `?peer=...` 查询过滤；
     - `POST /api/file/accept`：支持按 `id` 单项接收或 `{ "all": true, "peer": ... }` 批量接收；
     - `POST /api/file/reject`：支持按 `id` 单项拒绝或 `{ "all": true, "peer": ... }` 批量拒绝；
     - `GET /api/file/auto-accept` 与 `POST /api/file/auto-accept`：查询与设置受信对端自动接收策略；
     - 严格 127.0.0.1 回环绑定与 `Host: 127.0.0.1|localhost` 校验（403 on mismatch）。
  5. **CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - `offers [--peer <peer>] [--id <id>] [--json] [--ui-port <port>]`；
     - `accept [<id>] [--all] [--peer <peer>] [--json] [--ui-port <port>]`；
     - `reject [<id>] [--all] [--peer <peer>] [--json] [--ui-port <port>]`；
     - `auto-accept [--enable] [--disable] [--json] [--ui-port <port>]`；
     - `events` 监听器增设 `[file_rejected]` 与 `[file_auto_accept_changed]` 格式化打印。
  6. **Web UI 提议治理升级 (`ui/index.html`)**:
     - 待确认文件列表增设断点续传徽标与来源标识；
     - 增设「刷新提议」、「全部接收」、「全部拒绝」操作按钮；
     - 增设「自动接收受信任对端」复选框与累计自动接收次数徽标；
     - 对接 SSE 实时更新事件流。
  7. **端到端集成测试全覆盖 (`tests/phase1.rs`)**:
     - 增设 `file_offer_lifecycle_and_terminal_governance_via_loopback_api`：
       - 验证初始提议与自动接收状态为空；
       - 验证提议到达时 `GET /api/file/offers` 透视、按 ID 查询与按 peer 过滤；
       - 验证单项拒绝、派发 `UiEvent::FileRejected` 与提议移除；
       - 验证批量投递与批量拒绝（`FileBatchDecisionReport`）；
       - 验证开启受信对端自动接收策略、自动静默入站接收与计数器递增；
       - 验证关闭自动接收策略及非法请求边界校验；
       - 验证优雅停机。
  8. **网络层取消安全与多路复用健壮性强化 (`crates/tetherly-net/src/codec.rs`, `session.rs`)**:
     - 引入状态化 `TaggedReader` 环形缓冲状态机，彻底消除 Tokio `select!` 竞态丢弃分支时由栈内局部变量丢失导致的网络帧解包失步（partial-read frame desynchronization）；
     - `ActiveSession` 会话层全量接入 `TaggedReader`，确保高并发文件提议与拒绝交错时 TCP 帧边界零丢失；
     - 在 `tests/phase1.rs` 中增加了步骤 4 与步骤 5 对端拒绝观测同步，5 平台 CI 矩阵 100% 验证通过。
  9. **Windows 平台异步端到端测试与生命周期健壮性加固 (`crates/tetherly-android/tests/client_e2e.rs`)**:
     - 针对 Windows CI Runner 虚拟化 CPU 在多测试并行压力下的 Argon2id 密钥派生耗时与线程调度延迟，将 `android_client_pairs_and_delivers_notify` 连线等待截止时间从 10s 调整为 30s；
     - 针对 `android_client_receives_a_quick_reply_once`，移除了对端 mock 服务端脆弱的固定 500ms 休眠设计，引入 `tokio::sync::oneshot::channel()` 双向生命周期同步，确保服务端在客户端完全取走快速回复帧并完成校验前维持会话存活。
  10. **生产级代码审查修复、原子锁治理与多文件提议尺寸纠偏 (`crates/tetherly-node/src/runtime.rs`, `tests/phase1.rs`)**:
     - **修复 P1 多文件累积字节统计失真**：在 `file_offers` 检索中将 `total_bytes` 与 `size` 统一求和 `t.files.iter().map(|f| f.size).sum()`，终结了多文件传输时提议体积仅按首个文件计算的问题；
     - **修复 P2 拒绝状态流转清理竞态与单点原子锁收敛**：在 `user_accept_file_reported` 与 `user_reject_file_reported` 中，将决策状态转换与来源/文件名/总大小提取合并到持有 `hubs` 锁的单次临界区内，避免了 `hubs.files.reject` 后并发 prune 导致提取 source 失败抛出 `UnknownTransfer` 的竞态风险，同时将加锁次数由 3 次彻底降为 1 次；
     - **代码去重与接口收敛**：将 `user_accept_file` 与 `user_reject_file` 重构为直接包装 `*_reported` 变体，消除重复逻辑；
     - **端到端集成测试增强**：在 `tests/phase1.rs` 中增设包含 2 个子文件的多文件提议网络注入，验证回环 `GET /api/file/offers?id=...` 返回准确的累积字节数 4000，且通过 `user_reject_file_reported` 完成原子拒绝与报告校验；5 平台 CI（Ubuntu, macOS, Windows, Android NDK, Deny）100% 验证通过。

**1.52 相对 1.51(跨端剪贴板生命周期治理、元数据遥测审计与历史可观测性闭环)**

- **动机(剪贴板事件流黑盒、元数据透视断层、明文泄漏风险防范、缺乏历史审计与主动安全复位机制)**:
  在 Tetherly v1.51 实现了通知全生命周期治理与双向同步后，跨端数据协同网络愈发成熟。但在应用层**跨端安全剪贴板系统（Clipboard Lifecycle & Telemetry）**中，暴露出**事件黑盒、可观测性断层、元数据遥测缺失与历史审计空白**：
  1. **跨端剪贴板同步缺乏事件总线驱动与实时感知（Event Blackout）**：
     - 当手机端或外部对端向桌面节点推送剪贴板帧（`SessionEvent::Clip(clip)` -> `apply_clip`）并写入本地系统剪贴板时，底层从未向节点事件总线广播任何 `UiEvent`，导致 Web UI、SSE 订阅客户端与 CLI 事件监听器对对端剪贴板摄入完全处于「失明」状态；
     - 本地主动发送剪贴板（`send_clipboard_to` / `send_clipboard_text`）同样未派发任何结构化事件，外部集成工具与诊断控制台无法通过统一事件流审计剪贴板出入站动作；
  2. **缺乏安全的纯元数据透视端点（Metadata-Only Status Inspection）**：
     - 现存回环接口仅有 `GET /api/clip`，会直接返回明文文本（`text`）。在系统健康巡检、监控探针或状态展示场景下，调用方仅需要感知剪贴板状态（是否存在文本、字节大小、当前序号 `last_seq`、内容 SHA-256 哈希 `last_hash`、累计出入站次数），无需也不应将未经脱敏的明文文本暴露至日志或监控链路；
  3. **缺乏受控的轻量级历史审计环形缓冲区（Bounded History Ring Buffer）**：
     - 节点缺乏剪贴板同步历史的可观测性。用户或客户端无法追溯「最近同步的剪贴板是何时发生、流向为何、来自哪个设备、大小是多少、校验哈希是什么」；
     - **严格遵守安全红线铁律（Verbatim Security Red Line）**：「UiEvent 严禁携带短信正文、一次性码、配对码、剪贴板纯文本」与「公开 issue / 日志禁止贴短信正文、一次性码、配对码、剪贴板」。因此，历史审计记录必须是**纯元数据结构（Zero-Plaintext）**，仅记录 `(seq, source, direction, bytes, hash, timestamp_ms)`，绝对不存储任何明文字符；
  4. **缺乏剪贴板主动清空与安全复位原语（Clipboard Reset & Purge）**：
     - 用户处理完敏感信息（如一次性密码、凭证碎片）后，缺乏通过回环控制面一键清空本地系统剪贴板、重置内容哈希并销毁审计历史的机制；
  5. **回环端点与 CLI 缺乏规范化 RESTful 治理接口**：
     - 缺少 `GET /api/clip/status`（安全纯元数据查询）；
     - 缺少 `GET /api/clip/history`（支持 `?limit=N` 的历史查询）；
     - 缺少 `POST /api/clip/clear` 与 `DELETE /api/clip`（清空剪贴板与复位审计缓冲区）；
     - 双端 CLI（`tetherly` 与 `tetherly-cli`）缺少 `clip status`、`clip history`、`clip clear` 命令，并在 `events` 监听器中缺少 `[clipboard_synced]` 与 `[clipboard_cleared]` 格式化高亮展示；
  6. **Web UI 实时交互与历史可视化补全**：
     - Web UI（`ui/index.html`）剪贴板卡片增设当前状态徽标、最近 5 条同步记录列表，以及一键「清空剪贴板」确认按钮，联动 SSE 事件动态局部刷新。

- **架构与协议设计(ADR-045 跨端剪贴板生命周期治理、元数据遥测审计与历史可观测性闭环)**:
  1. **核心模型与元数据环形缓冲区 (`crates/tetherly-core/src/clip.rs`, `lib.rs`)**:
     - 定义 `ClipDirection`（`Inbound`, `Outbound`）与 `ClipMetaEntry` 纯元数据条目；
     - `ClipHub` 集成固定容量环形缓冲区 `MAX_CLIP_HISTORY: usize = 32`（`VecDeque<ClipMetaEntry>`）与 `inbound_count`, `outbound_count` 统计计数器；
     - 提供 `record_entry`（自动 FIFO 逐出，保证 $O(1)$ 内存界限）、`history(limit)`、`clear_history`、`reset_state` 与 `stats` 原语；
     - 增加核心单元测试 `history_and_bounded_ring_buffer` 验证容量约束与状态复位。
  2. **事件总线扩展与安全红线守卫 (`crates/tetherly-node/src/events.rs`, `runtime.rs`, `lib.rs`)**:
     - 新增 `UiEvent::ClipboardSynced { seq, source, direction, bytes, hash, timestamp_ms, sent_peers }` 与 `UiEvent::ClipboardCleared { timestamp_ms }`；严格遵守**零明文原则（Zero-Plaintext）**，严禁包含文本内容或 OTP；
     - 定义结构化 DTO 报告：`ClipStatusReport`、`ClipHistoryReport`、`ClipMetaEntryDto` 与 `ClipClearReport`。
  3. **运行时可观测性闭环与控制原语 (`crates/tetherly-node/src/runtime.rs`)**:
     - `apply_clip` 闭环：入站写入系统剪贴板后，记录元数据至 `ClipHub`，递增 `inbound_count`，向事件总线广播 `UiEvent::ClipboardSynced`（inbound）；
     - `send_clipboard_to` / `send_clipboard_text` 闭环：出站投递至对端会话后，记录元数据至 `ClipHub`，递增 `outbound_count`，向事件总线广播 `UiEvent::ClipboardSynced`（outbound）；
     - `Node::clipboard_status`：零泄漏状态查询原语；
     - `Node::clipboard_history`：受控历史记录快照查询原语；
     - `Node::clear_clipboard`：安全清空系统剪贴板（设为 `""`），复位 `ClipHub` 哈希与历史，广播 `UiEvent::ClipboardCleared`。
  4. **轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/clip/status`：返回 `ClipStatusReport`（零明文）；
     - `GET /api/clip/history`：支持 `?limit=N`，返回 `ClipHistoryReport`；
     - `POST /api/clip/clear` 与 `DELETE /api/clip`：清空剪贴板并返回 `ClipClearReport`；
     - 严格 127.0.0.1 回环绑定与 `Host: 127.0.0.1|localhost` 校验（403 on mismatch）。
  5. **CLI 双端全同构升级 (`bins/tetherly-cli/src/main.rs` 与 `crates/tetherly-node/src/bin/tetherly.rs`)**:
     - `clip status [--json] [--ui-port <port>]`：格式化呈现当前剪贴板状态（字节数、序号、SHA-256、流向累计）；
     - `clip history [--limit <n>] [--json] [--ui-port <port>]`：表格化呈现最近同步历史记录；
     - `clip clear [--json] [--ui-port <port>]`：清空系统剪贴板与复位确认；
     - `events` 监听器增设 `[clipboard_synced]` 与 `[clipboard_cleared]` 格式化打印。
  6. **Web UI 实时交互与历史可视化 (`ui/index.html`)**:
     - 剪贴板卡片增设状态徽标（有效文本/空、大小、哈希缩写、流向计数）；
     - 增设「清空剪贴板」确认按钮；
     - 增设最近 5 条同步元数据条目（流向徽标、来源设备、大小、时间、SHA-256 缩写）；
     - SSE 监听器对接 `clipboard_synced` 与 `clipboard_cleared`，动态局部重绘。
  7. **端到端集成测试全覆盖 (`tests/phase1.rs`)**:
     - 增设 `clipboard_lifecycle_history_and_observability_via_loopback_api`：
       - 验证初始状态查询为空；
       - 验证出站发送时派发 `UiEvent::ClipboardSynced`（outbound）、状态及历史递增；
       - 验证对端入站同步时派发 `UiEvent::ClipboardSynced`（inbound）、系统剪贴板写入与哈希校验；
       - 验证环形缓冲区容量上限为 32 条且超出时 FIFO 逐出；
       - 验证 `POST /api/clip/clear` 与 `DELETE /api/clip` 清空系统剪贴板、重置状态与派发 `UiEvent::ClipboardCleared`；
       - 验证优雅停机。

**1.51 相对 1.50(通知全生命周期治理、跨端双向消除同步与通知历史可观测性闭环)**

- **动机(通知单向消除断层、跨端消除不同步、移动端常驻通知悬挂、缺乏多维批量消除治理与历史可观测性)**:
  在 Tetherly v1.50 建立了完整的对端黑名单与接入鉴权体系后，网络层安全边界已高度健壮。但在应用层**跨设备通知流协同（Notification Lifecycle & Ingress）**中，暴露出**通知单向消除断层、跨端双向同步缺失与批量治理空白**：
  1. **跨端通知消除断层与双向同步缺失**：
     - 当桌面端用户或 Loopback 客户端消除通知候选时（`Node::dismiss_candidate` / `POST /api/dismiss`），此前仅在本地 `hubs.notify` 内存表中移除了 `Candidate`，从未通过活动 Noise 隧道向源对端（如 Android 手机）发送 `TYPE_NOTIFY_DISMISS` 协议帧（`NotifyDismiss { app_id, uid }`）。导致手机端状态栏中的通知依然挂载，无法达成真正的「桌面已读/清除，手机同步消除」；
     - 反向链路同样存在断层：当手机端主动消除通知并在 Noise 会话接收到 `SessionEvent::Dismiss(d)` 时，桌面节点虽然调用了 `hubs.notify.dismiss_wire(peer, &d)`，但并未向本地事件总线派发 `UiEvent::CandidateDismissed`，导致 Web UI、终端监听器与外部 Loopback 订阅者对消除事件毫无感知，界面依然残留已消除通知；
  2. **缺乏多维批量消除与筛选治理原语**：
     - 现存接口仅支持按单一 `id`（`candidate_id`）逐个消除，缺少面向生产环境的批量治理能力：按对端设备过滤并全量消除（`dismiss_by_source` / `peer`）、按应用包名/标识批量消除（`dismiss_by_app` / `app_id`）、一键清空全量待处理通知（`clear_candidates` / `all`）；缺少轻量级结构化 DTO 报告（`CandidateDismissReport` 与 `BatchDismissReport`）；
  3. **回环端点与 CLI 缺乏规范化 RESTful 治理接口**：
     - 现有回环路由仅有非标 `POST /api/dismiss`，缺失规范的 RESTful 路由（`POST /api/notify/dismiss` 与 `DELETE /api/notify/candidates`）；`GET /api/notify/candidates` 缺乏按 `peer`、`app_id`、`has_otp` 的维度过滤；CLI 客户端缺乏高级过滤与批量消除命令群（`--all`, `--peer`, `--app-id`）；
  4. **事件总线扩展与 Web UI 实时动态联动**：
     - 增设 `UiEvent::CandidateDismissed { id, source, app_id, uid }`；Web UI 通知列表支持消除某条通知时实时从 DOM 剔除，同时监听 SSE `candidate_dismissed` 事件实现多标签页无缝局部重绘，并增设「清空通知」一键消除按钮；
  5. **安全红线铁律（Verbatim Security Red Line）**：
     - 「公开 issue / 日志禁止贴短信正文、一次性码、配对码、剪贴板」；
     - 「信任库只在 Noise transport 成功后写」；
     - 「Loopback UI 45716 仅绑定 127.0.0.1，校验 Host: 127.0.0.1|localhost (403 on mismatch); 严禁绑定 0.0.0.0 / ::」；
     - **`UiEvent` 严禁携带短信正文、一次性码、配对码、剪贴板纯文本**，`CandidateDismissed` 事件严禁携带任何通知正文或 OTP 验证码内容，仅允许携带结构化元数据标识符（`id`, `source`, `app_id`, `uid`）。

- **架构与协议设计(ADR-044 通知全生命周期治理、跨端双向消除同步与通知历史可观测性闭环)**:
  1. **核心通知模型扩展 (`crates/tetherly-core/src/notify.rs`)**:
     - `Candidate` 结构体派生 `Clone`；
     - `take_candidate(&mut self, id: &str) -> Option<Candidate>`：原子提取并返回候选结构体，完整保留 `source` (DeviceId)、`app_id` 与 `uid` 元数据供网络层定向广播；
     - `dismiss_by_source(&mut self, source: &DeviceId) -> Vec<Candidate>`：批量提取并移除指定对端源的所有候选；
     - `dismiss_by_app(&mut self, app_id: &str) -> Vec<Candidate>`：批量提取并移除指定应用标识的所有候选；
     - `clear_candidates(&mut self) -> Vec<Candidate>`：清空并返回全部候选；
     - `dismiss_wire_candidates(&mut self, source: &DeviceId, d: &NotifyDismiss) -> Vec<Candidate>`：按对端与 wire 载荷定位移除，并返回逐出的候选列表以驱动事件总线；
  2. **事件总线与结构化 DTO 抽象 (`crates/tetherly-node/src/events.rs`, `runtime.rs`, `lib.rs`)**:
     - `UiEvent::CandidateDismissed { id, source, app_id, uid }`：闭环消除事件派发，严格不包含正文与 OTP；
     - `CandidateDismissReport { ok, id, source, app_id, uid, notified_remote }`；
     - `BatchDismissReport { ok, count, ids, notified_remote_count }`；
     - `DismissFilter { id, peer, app_id, all }`；
  3. **运行时跨端双向同步与批量消除控制原语 (`crates/tetherly-node/src/runtime.rs`)**:
     - `Node::dismiss_candidate_sync(&self, id: &str) -> Result<CandidateDismissReport, CoreError>`：
       - 原子提取候选并获取元数据；
       - 向对应对端会话投递 `NotifyDismiss { app_id, uid }` 帧；
       - 向本地事件总线派发 `UiEvent::CandidateDismissed`；
     - `Node::dismiss_candidates(&self, filter: &DismissFilter) -> Result<BatchDismissReport, CoreError>`：
       - 支持按 `all`、`peer`、`app_id` 或 `id` 批量提取匹配候选；
       - 分别向对应的源设备活动会话投递 `NotifyDismiss` 帧；
       - 逐个派发 `UiEvent::CandidateDismissed`，聚合统计消除总数与远端同步计数；
     - `SessionEvent::Dismiss(d)` 处理分支闭环：调用 `dismiss_wire_candidates` 并向本地派发 `UiEvent::CandidateDismissed`；
     - `AncsEvent::Removed` 分支闭环：派发 `UiEvent::CandidateDismissed`；
     - `Node::candidates_filtered(&self, peer, app_id, has_otp)`：多维条件过滤查询；
  4. **轻量级回环控制面端点升级 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/notify/candidates`：支持 Query 参数 `peer`, `app_id`, `has_otp`；
     - `POST /api/dismiss`：升级内部调用 `dismiss_candidate_sync` 并返回 `CandidateDismissReport`；
     - `POST /api/notify/dismiss` 与 `DELETE /api/notify/candidates`：统一规范端点，自动兼容单 ID 与批量（`peer`/`app_id`/`all`）消除；
  5. **CLI 双端全同构升级 (`tetherly` 与 `tetherly-cli`)**:
     - `dismiss` 命令群支持 `[id]`、`--peer <peer>`、`--app-id <app>`、`--all`、`--json`、`--ui-port <port>`；
     - `candidates` 命令群支持 `--peer <peer>`、`--app-id <app>`、`--has-otp <bool>`、`--json`、`--ui-port <port>`；
     - `events` 监听器高亮格式化输出 `[candidate_dismissed]`；
  6. **Web UI 实时交互与按键闭环 (`ui/index.html`)**:
     - 监听 `candidate_dismissed` SSE 事件，动态局部重绘；
     - 候选卡片「忽略」按钮乐观置灰并调用 `/api/notify/dismiss`；
     - 增设「清空通知」按键，一键触发全量清空；
  7. **端到端集成测试全覆盖 (`tests/phase1.rs`)**:
     - `notification_bidirectional_dismissal_and_lifecycle_governance_via_loopback_api` 完备验证。

**1.50 相对 1.49(对端黑名单访问控制、恶意连接防御与动态接入鉴权治理)**

- **动机(仅有受信任白名单、恶意连接与端口扫描无防御、缺乏持久化黑名单拦截、断联后无法阻止重拨)**:
  在 Tetherly v1.49 实现了对端连接会话全域治理与主动按需直连后，网络拓扑具备了高度灵活的受信任会话控制能力。但在跨网段多设备互联与开放局域网/虚网组网中，暴露出**仅有受信任白名单、缺乏黑名单防御与动态接入鉴权治理**的严重安全防护短板：
  1. **缺少对端黑名单拒绝与拦截机制**：此前系统仅有基于 `trusted.json` 的白名单信任判定；一旦某台设备发生异常、行为不受信、存在端口恶意探测或受到中间人骚扰，用户即使通过 `unpair` 解绑设备，对方依然可以不断发起 Noise 握手和 PIN 码配对探测，对节点控制通道造成 socket 资源耗尽与日志泛洪；
  2. **缺乏黑名单持久化存储与原子落盘**：节点缺乏独立的 `blocked.json` 黑名单持久化机制，无法记录被封禁的设备 ID（`device_id`）、封禁原因（`reason`）与时间戳（`blocked_at`），进程重启后防御状态完全丢失；
  3. **握手与拨号链路缺乏黑名单鉴权门禁**：
     - 在被动接收端（`accept_one`）：完成 Noise transport 握手提取对方真实认证身份后，缺少黑名单前置检查，无法在挂载会话前主动扼杀连接；
     - 在主动发起端（`dial` 与 `connect_peer`）：若对端已被拉黑，依然会尝试发起 TCP 连接并消耗网络带宽；后台自动重连定时器（`spawn_reconnect`）也会死循环尝试向被拉黑设备重连；
  4. **缺乏屏蔽正在通信设备的主动断连与清理机制**：拉黑某个当前在线的对端时，必须同步掐断活动 TCP 信道、清理会话映射、驱逐 `live` 列表、重置光标焦点，并向事件总线广播 `UiEvent::PeerDown`；
  5. **缺少轻量级回环控制面端点**：外部管理面板与桌面前端缺少 `GET /api/peers/blocked`、`POST /api/peers/block` 与 `POST /api/peers/unblock` 接口；
  6. **CLI 双端与 Web UI 缺失黑名单控制能力**：双端命令行缺少 `block`、`unblock` 与 `blocked` 子命令，Web UI 无法直观查看黑名单设备列表与一键拉黑/解封。

- **架构与协议设计(ADR-043 对端黑名单访问控制、恶意连接防御与动态接入鉴权治理)**:
  1. **错误语义与异常边界扩展 (`crates/tetherly-core/src/error.rs`)**:
     - 增设 `CoreError::PeerBlocked(String)` 变体，将黑名单拦截与鉴权失败明确暴露为结构化错误，避免与一般网络 IO 超时或密钥错误混淆；
  2. **黑名单持久化存储与原子落盘 (`crates/tetherly-node/src/store.rs`)**:
     - 定义结构体 `BlockedPeer { device_id, reason, blocked_at }` 与 `BlockedTable { peers }`；
     - 常量 `BLOCKED_FILE = "blocked.json"`；
     - 提供 `load_blocked`, `save_blocked`, `add_blocked_peer`, `remove_blocked_peer`, `is_peer_blocked` 原语；采用原子写入（写入临时文件后 `fs::rename`）并设置 POSIX 权限（`0600`）；
  3. **节点运行时动态黑名单同步与接入鉴权门禁 (`crates/tetherly-node/src/runtime.rs`)**:
     - 在 `Shared` 结构体增设 `blocked: Mutex<BlockedTable>` 内存缓存；
     - 定义 DTO 报告：`PeerBlockReport`, `PeerUnblockReport`, `BlockedPeersReport`；
     - `Node::is_blocked(&self, device_id: &str) -> bool`：快速查询内存黑名单；
     - `Node::block_peer(&self, query: &str, reason: Option<&str>) -> Result<PeerBlockReport, CoreError>`：
       - 支持设备 ID、别名或名称的智能解析（`resolve_peer`）；
       - 禁止屏蔽本机自身设备 ID（防误操作防御）；
       - 更新内存与持久化 `blocked.json`；
       - 若对端当前处于在线会话，瞬间从 `Shared.sessions` 移除关闭 TCP 信道、从 `Shared.live` 驱逐、复位 `input_seat`、并派发 `UiEvent::PeerDown`；
       - 派发 `UiEvent::PeerBlocked { device_id, reason }`；
     - `Node::unblock_peer(&self, query: &str) -> Result<PeerUnblockReport, CoreError>`：
       - 移除黑名单记录并原子落盘，派发 `UiEvent::PeerUnblocked { device_id }`；
     - 在主动端 `connect_peer`、`dial` 与后台 `spawn_reconnect` 处守卫：若 `is_blocked`，直接跳过并拒绝连接；
     - 在被动接收端 `accept_one` 完成 Noise 握手提取已认证对端 ID 后守卫：若 `is_blocked`，立即返回 `CoreError::PeerBlocked`，严禁写入信任库、严禁更新路由缓存、严禁附加在线会话；
  4. **轻量级回环控制面端点 (`crates/tetherly-node/src/uihttp.rs`)**:
     - `GET /api/peers/blocked`：返回 `BlockedPeersReport`；
     - `POST /api/peers/block`：接收 `PeerBlockBody { peer, reason }`，执行拉黑并返回 `PeerBlockReport`；
     - `POST /api/peers/unblock` 与 `DELETE /api/peers/block` / `DELETE /api/peers/blocked`：支持多维解封调用，返回 `PeerUnblockReport`；
     - 严格守卫安全红线：严格绑定 `127.0.0.1`，强校验 `Host: 127.0.0.1|localhost`；
  5. **CLI 双端全同构升级 (`crates/tetherly-node/src/bin/tetherly.rs`, `bins/tetherly-cli/src/main.rs`)**:
     - `block`: `--peer <PEER> [--reason <REASON>] [--json] [--ui-port <PORT>]`；
     - `unblock`: `--peer <PEER> [--json] [--ui-port <PORT>]`；
     - `blocked`: `[--json] [--ui-port <PORT>]`；
     - 在 `print_event` 格式化呈现 `[peer_blocked]` 与 `[peer_unblocked]` 事件；
  6. **Web UI 访问控制面板与按键闭环 (`ui/index.html`)**:
     - 在在线对端卡片与受信任设备卡片均增设「屏蔽」交互按键，弹出确认与原因输入框；
     - 独立增设「访问控制与黑名单治理」卡片，全景展示当前被拉黑设备 ID、原因与时间，并提供「+ 屏蔽设备」与「解除屏蔽」操作；
     - 接入 SSE 实时响应 `peer_blocked` 与 `peer_unblocked` 事件局部刷新视图；
  7. **端到端集成测试全绿覆盖 (`tests/phase1.rs`)**:
     - 增设 `peer_blacklist_and_access_control_governance_via_loopback_api`：完备验证配对连通、拉黑对端、活动 TCP 会话瞬间掐断与 SSE `PeerDown`/`PeerBlocked` 派发、`blocked.json` 持久化、主动外拨拦截、被动入站握手拒绝、拉黑幂等性、解除拉黑与会话重新连通全流程。

**1.49 相对 1.48(对端连接会话全域治理、主动按需直连与优雅断开控制闭环)**

- **动机(会话与信任生命周期混淆、主动重连能力缺失、按需断开无门、CLI 与 UI 控制缺失)**:
  在 Tetherly v1.48 实现了对端能力全景透视与特性协商后，多通道交互具备了动态边界治理能力。但在日常多节点拓扑运行中，暴露出**会话生命周期与信任关系强耦合、主动连接能力缺失**的关键运维痛点：
  1. **会话断连与信任解绑混淆**：此前仅存在 `POST /api/peers/unpair`（以及 CLI `unpair`），一旦调用便会彻底清除 `trusted.json` 中的对称密钥与信任关系，并抹除 `routes.json` 物理寻址缓存；若用户仅希望暂时断开某台设备的活动 TCP 会话（例如切换网络、开会节能或暂停多端同步），此前完全无法在保留配对信任的前提下优雅断开；
  2. **缺少按需主动发起直连原语**：已配对受信任设备断线后，系统完全依赖后台 30 秒轮询定时器（`spawn_reconnect`）尝试回拨；若后台重连被用户禁用（`--reconnect false`）或对端刚上线，用户无法主动下达指令要求节点立即与对端建立连接，必须等待长达 30 秒甚至重启守护进程；
  3. **缺少轻量级回环控制端点**：外部运维脚本与桌面前端缺少 `POST /api/peers/connect` 与 `POST /api/peers/disconnect` 原语，无法精准控制指定对端的会话在线状态；
  4. **双端命令行缺失连接与断开子命令**：`tetherly` 与 `tetherly-cli` 仅有初次配对用的 `dial`，缺少面向已受信任设备的 `connect` 与 `disconnect` 命令群；
  5. **Web 前端卡片缺少操作按键**：Web UI 对端卡片无法一键手动断开在线会话，离线受信任卡片无法一键触发重新连接。

- **架构与协议设计(ADR-042 对端连接会话全域治理、主动按需直连与优雅断开控制闭环)**:
  1. **对端会话生命周期模型解耦与 DTO 抽象 (`runtime.rs`, `lib.rs`)**:
     - `PeerConnectReport`: `{ ok, device_id, name, alias, addr, connected, already_connected }`；
     - `PeerDisconnectReport`: `{ ok, device_id, name, alias, disconnected, already_disconnected }`；
     - 严格区分「临时断开会话 (disconnect)」与「永久撤销信任 (unpair)」：disconnect 仅断开活动 TCP 传输信道，绝不修改 `trusted.json` 与 `routes.json`；
  2. **节点运行时主动连接与优雅断开控制原语 (`runtime.rs`)**:
     - `Node::connect_peer(&self, query: &str, direct_addr: Option<&str>) -> Result<PeerConnectReport, CoreError>`：
       - 支持基于设备 ID、别名或广播名的智能解析（`resolve_peer`）；
       - 若已在线，幂等返回 `already_connected: true`；
       - 若提供显式 `direct_addr`，校验套接字与非零端口并更新 `routes.json` 缓存；若未提供则从 `routes.json` 自动解析物理地址；
       - 异步触发 `dial(addr)` 握手建连，成功后返回完整报告；
     - `Node::disconnect_peer(&self, query: &str) -> Result<PeerDisconnectReport, CoreError>`：
       - 智能解析定位对端；
       - 若已处于离线状态，幂等返回 `already_disconnected: true`；
       - 否则从 `Shared.sessions` 移除发送端以关闭信道，从 `Shared.live` 驱逐，复位输入焦点（`input_seat`），并向事件总线广播 `UiEvent::PeerDown`；
  3. **轻量级回环控制面端点 (`uihttp.rs`)**:
     - `POST /api/peers/connect`：支持 JSON 体 `{"peer": "...", "addr": "..."}` 与 Query 参数 `?peer=...&addr=...`；未知对端返回 404，寻址失败或地址非法返回 400；
     - `POST /api/peers/disconnect`：支持 JSON 体 `{"peer": "..."}` 与 Query 参数 `?peer=...`；未知对端返回 404；
     - 严格守卫安全红线：仅绑定 `127.0.0.1`，强校验 `Host: 127.0.0.1|localhost`；
  4. **CLI 双端全同构升级 (`tetherly connect/disconnect`, `tetherly-cli connect/disconnect`)**:
     - `connect`: `--peer <PEER> [--addr <IP:PORT>] [--json] [--ui-port <PORT>]`；
     - `disconnect`: `--peer <PEER> [--json] [--ui-port <PORT>]`；
  5. **Web UI 会话治理按键与交互闭环 (`ui/index.html`)**:
     - 在在线设备卡片增设「断开」交互按键，点击调用 `POST /api/peers/disconnect`；
     - 在受信任设备卡片中，依据 `online` 状态动态呈现「断开」或「连接」按键，点击分别调用对应回环接口；
     - 接入 SSE `peer_up` 与 `peer_down` 事件驱动界面实时局部重绘；
  6. **端到端集成测试全绿覆盖 (`tests/phase1.rs`)**:
     - 增设 `peer_active_connect_and_disconnect_lifecycle_via_loopback_api`：覆盖已在线状态检测、断开会话、SSE `PeerDown` 捕获、信任保持、幂等二次断开、按需直连重连、显式地址连接、以及非法地址与未知对端防御全流程。

**1.48 相对 1.47(对端能力全景透视、多维特性协商与动态功能边界治理)**

- **动机(对端能力完全黑盒、特性协商可观测性断层、动态能力边界不可控、CLI 与 UI 无感知)**:
  在 Tetherly v1.47 实现了全协议网络流量透视与无锁原子遥测度量后，系统具备了微秒级网络吞吐可观测性。但在应用层多通道（通知同步、剪贴板共享、文件传输、键鼠控制、远程屏幕）交互演进中，暴露出**对端功能能力黑盒与特性协商可观测性断层**的严重架构缺陷：
  1. **对端能力完全丢弃与黑盒化**：协议虽然定义了 `TYPE_CAPS_UPDATE` 帧与 `CapsUpdate { caps: Vec<String> }`，但在 `runtime.rs` 接收到对端 `CapsUpdate` 时，仅提取了端口号（`fileport=`, `inputport=`, `screenport=`），将所有的核心功能特性标签（`notify`, `clip`, `file`, `input`, `screen` 等）完全抛弃；`LivePeer` 结构体缺少 `caps` 存储字段，系统在连接建立后对对端究竟支持哪些特性一无所知；
  2. **缺少能力全景透视端点**：`GET /api/peers` 与 `GET /api/state` 均未暴露对端能力列表，外部控制面无法获知对端已就绪的功能矩阵；缺少专用轻量级回环端点 `GET /api/caps`（透视本机支持并对外广播的能力清单）与 `GET /api/peers/caps`（查询所有对端或指定对端的能力矩阵）；
  3. **缺少动态能力热更新与即时广播机制**：本机能力此前静态写死，无法在运行时按需启闭某项功能（例如在网络受限或高安全要求场景动态关闭屏幕推流或剪贴板共享）；缺乏 `Node::broadcast_caps()` / `POST /api/caps` 动态热更新并向所有活动 Noise 会话广播最新能力帧的原语；
  4. **事件总线缺少能力变更事件**：对端更新能力或本机广播能力变更时，事件总线没有对应的 `UiEvent::PeerCapsChanged`，导致外部控制面与 UI 视图无法实时响应；
  5. **双端命令行与 Web UI 缺失能力透视与指示徽标**：`tetherly` 与 `tetherly-cli` 缺乏 `caps` 命令群，`peers` 列表缺少能力标签；Web UI（`ui/index.html`）对端卡片缺少能力徽标（`[通知]` `[剪贴板]` `[文件]` `[输入]` `[屏幕]`），用户无法直观了解对端具备何种功能。

- **架构与协议设计(ADR-041 对端能力全景透视、多维特性协商与动态功能边界治理)**:
  1. **能力数据结构与 DTO 模型设计 (`runtime.rs`, `events.rs`, `lib.rs`)**:
     - 扩展 `LivePeer` 结构体，增加 `caps: Vec<String>` 存储字段，全生命周期跟踪对端通告的能力列表；
     - 引入纯特性提取工具函数 `extract_features(caps: &[String]) -> Vec<String>`：严格过滤包含 `=` 的端口通告，保留唯一排序的纯特性标签；
     - 定义全景能力报告模型：
       - `LocalCapsReport`: `{ ok, device_id, caps, features, file_port, input_port, screen_port }`；
       - `PeerCapsDto`: `{ device_id, name, alias, caps, features, file_port, input_port, screen_port, online }`；
       - `PeersCapsReport`: `{ ok, count, peers }`；
     - 在 `events.rs` 增设 `UiEvent::PeerCapsChanged { device_id, caps, features }`，联动 SSE 实时推送；
  2. **运行时存储、动态广播与事件链路集成 (`runtime.rs`)**:
     - 在 `Shared` 维护 `local_caps: Mutex<HashSet<String>>`，默认包含 `"notify"`, `"clip"`, `"file"`, `"input"`, `"screen"`；
     - 实现 `Node::local_caps_report(&self) -> LocalCapsReport`；
     - 实现 `Node::peers_caps_report(&self, target_peer: Option<&str>) -> Result<PeersCapsReport, CoreError>`（支持基于设备 ID、别名与广播名的智能解析）；
     - 实现 `Node::update_local_caps(&self, enable: &[String], disable: &[String]) -> Result<LocalCapsReport, CoreError>`：运行时热更新本机特性集，组装最新 `CapsUpdate` 帧全量异步广播至所有活动会话，并派发本地 `UiEvent::PeerCapsChanged`；
     - 在 `SessionEvent::Caps(update)` 处理分支中保留完整 `caps` 列表并更新 `LivePeer.caps`，触发 `UiEvent::PeerCapsChanged`；
  3. **轻量级回环控制面端点 (`uihttp.rs`)**:
     - `GET /api/caps`：返回本机能力的 `LocalCapsReport`；
     - `POST /api/caps` 与 `PUT /api/caps`：支持 `{"enable": [...], "disable": [...]}` 动态启闭并热广播；
     - `GET /api/peers/caps`：支持 `?peer=<id_or_alias>` 查询对端能力矩阵 `PeersCapsReport`；
     - 升级 `GET /api/peers` 与 `GET /api/state`：在对端 DTO 中完整暴露 `caps` 与 `features` 数组；
     - 安全红线铁律：严格绑定 `127.0.0.1` 环回接口，校验 `Host: 127.0.0.1|localhost`（不匹配返回 403）；校验特性名称合法性（仅允许 1..=32 字节英文字母、数字、下划线及连字符，严禁 `=`、控制字符与敏感载荷）；
  4. **CLI 双端全同构升级 (`tetherly caps`, `tetherly-cli caps`)**:
     - 新增 `Cmd::Caps` 命令群，支持 `--peer <PEER>`, `--enable <CAP>`, `--disable <CAP>`, `--json`, `--ui-port <PORT>`；
     - 升级 `Cmd::Peers` 输出，直观附加展示对端能力标签（如 `caps: [notify, clip, file, screen]`）；
     - SSE 事件监听器接入 `peer_caps_changed` 实时流；
  5. **Web UI 能力看板与交互控制 (`ui/index.html`)**:
     - 增设「节点能力与特性边界」卡片，展示当前广播特性并提供按键动态启闭（交互调用 `/api/caps`）；
     - 对端卡片增设彩色特性徽标胶囊（`[通知]`, `[剪贴板]`, `[文件]`, `[输入]`, `[屏幕]`）；
     - 接入 SSE `peer_caps_changed` 事件实现无感局部重绘；
  6. **端到端集成测试全绿覆盖 (`tests/phase1.rs`)**:
     - 增设 `peer_capabilities_negotiation_and_introspection_via_loopback_api`：全面覆盖初态查询、对端能力提取、智能解析检索、动态启闭热广播、SSE 变更事件及非法输入边界防御。

**1.47 相对 1.46(全协议网络流量透视、多通道遥测度量与运行时可观测性闭环)**

- **动机(全局流量黑盒、四通道带宽盲区、协议帧遥测缺失、回环控制与运维看板断层)**:
  在 Tetherly v1.46 实现了静态路由注入与断线重连治理后，节点具备了强大的物理拓扑穿透能力。然而在长期生产运行中，系统面临着**全局网络流量与协议消息遥测的重大可观测性盲区**：
  1. **全局网络流量黑盒**：除了文件通道内部的会话级 `TransferStats` 外，守护进程缺乏对整机网络出入站总吞吐（TX/RX 字节数、数据帧包数）的统一度量与聚合；
  2. **四维多通道带宽分布不可知**：控制 (45717)、文件 (45718)、输入 (45719)、屏幕 (45720) 各自消耗了多少网络带宽完全缺乏细分度量，无法定位由于大文件传输或屏幕高频推流导致的网络拥堵瓶颈；
  3. **协议消息遥测计数缺失**：Ping/Pong 探活、Notification 通知、Clipboard 剪贴板、FileOffer 文件报价等核心协议帧缺乏全局调用计数，会话建连数与握手失败率处于黑盒状态；
  4. **缺少轻量级遥测回环查询与基准重置端点**：外部监控或自动化运维脚本无法一键获取全局网络指标，也无法在基准性能测试中原子清零历史数据；
  5. **双端命令行与 Web UI 缺乏遥测面板**：`tetherly` 与 `tetherly-cli` 缺失 `metrics`/`traffic` 命令，Web 前端缺少流量透视卡片。

- **架构与协议设计(ADR-040 全协议网络流量透视、多通道遥测度量与运行时可观测性闭环)**:
  1. **无锁原子遥测引擎与数据传输模型 (`TrafficTelemetry`, `ChannelTrafficDto`, `ProtocolMessagesDto`, `TrafficReport`)**:
     - 在 `telemetry.rs` 与 `lib.rs` 引入无锁原子结构体 `TrafficTelemetry`：
       - 基于 `AtomicU64` 与 `Ordering::Relaxed`，以 $O(1)$ 极低开销在各通道数据传输热路径上原子累加计数；
       - `ChannelTrafficDto`：聚合 `tx_bytes`, `rx_bytes`, `tx_frames`, `rx_frames`；
       - `ProtocolMessagesDto`：记录 `pings_sent/recv`, `pongs_sent/recv`, `notifies_sent/recv`, `clips_sent/recv`, `file_offers_sent/recv`；
       - `TrafficReport`：包含 `ok`, `uptime_secs`, `total_tx_bytes`, `total_rx_bytes`, `total_tx_frames`, `total_rx_frames`, 四通道明细、协议消息明细及会话统计（`sessions_established`, `sessions_closed`, `handshakes_failed`）；
     - 提供原子 `snapshot(uptime_secs)` 与 `reset()` 原语；
  2. **全通道 I/O 链路埋点集成 (`runtime.rs`)**:
     - 控制通道 (45717)：在 `attach` 的发送与接收循环中记录出入站帧字节与协议类型；
     - 文件通道 (45718)：在流式分块传输与内存分块发送/接收中记录出入站字节；
     - 键鼠通道 (45719)：在输入事件帧收发循环中原子累加字节与帧数；
     - 屏幕通道 (45720)：在屏幕压缩帧推流与接收循环中原子累加字节与帧数；
     - 会话生命周期：在会话建立成功、连接关闭及握手失败处精准递增对应计数器；
  3. **轻量级回环控制面端点 (`uihttp.rs`)**:
     - `GET /api/metrics` 与 `GET /api/traffic`：毫秒级响应 `TrafficReport` 快照；
     - `POST /api/metrics/reset` 与 `POST /api/traffic/reset`：调用 `node.reset_traffic_metrics()` 并向事件总线广播 `UiEvent::MetricsReset`；
     - 严格受限于安全红线：仅绑定 `127.0.0.1`，严格校验 `Host: 127.0.0.1|localhost`（不匹配返回 403）；数据仅包含纯数字度量，绝不泄露敏感载荷；
  4. **CLI 双端全同构命令 (`tetherly metrics` / `traffic`, `tetherly-cli metrics` / `traffic`)**:
     - 支持 `--reset`、`--json` 与 `--ui-port`；
     - 格式化终端输出：自动换算 B/KiB/MiB/GiB，绘制四通道带宽分布柱状占比，清晰呈现协议消息与会话健康度；
  5. **Web UI 遥测仪表盘与交互闭环 (`ui/index.html`)**:
     - 增设「网络流量与全协议遥测」面板，集成出入站吞吐徽标、四通道带宽分布进度指示与协议核心消息芯片；
     - 提供一键重置按键与 SSE `metrics_reset` 实时局部刷新；
  6. **端到端集成测试全绿覆盖 (`tests/phase1.rs`)**:
     - 增设 `traffic_metrics_inspection_and_telemetry_reporting_via_loopback_api`：完整断言初态查询、控制帧交互流量递增、文件流式传输度量、POST 重置及 SSE 事件响应与清零验证。

**1.46 相对 1.45(物理寻址路由表全景透视、静态路由注入与断线自动重连治理)**

- **动机(物理寻址路由黑盒、静态路由注入缺失、跨子网/NAT穿越重连盲区、CLI 与 Web 端同构断层)**:
  在 v1.18 至 v1.45 的演进中，Tetherly 建立了基于 mDNS 组播广播、Noise 握手及自动重连机制的点对点安全网格。然而在实际多宿主物理网络（跨物理子网、隔离 VLAN、禁止组播的公共 Wi-Fi、或云主机/NAT 穿越场景）中，暴露出以下显著缺陷：
  1. **物理寻址路由黑盒与可观测性断层**：节点虽在后台维护 `routes.json` 物理地址映射，但此前 `GET /api/routes` 仅粗糙暴露无元数据的裸字符串对，外部无法探知该物理地址对应的对端是否在线、是否已被信任、友好别名为何，调用方难以评估路由的有效性与实时健康度；
  2. **静态路由注入与动态修改能力缺失**：当两台设备处于不同子网（无法收到 mDNS 广播），若未预先配置或路由因 DHCP 变化失效，用户没有任何 API 或命令行途径向正在运行的守护进程注入对端 IP:Port 物理路由；此前必须杀掉守护进程手动编辑 JSON 文件，严重破坏运维连续性；
  3. **缺少智能对端解析联动**：用户希望指定对端名称或别名添加路由时，此前只能费力拷贝 21 位的 Base32 加密 ID；
  4. **CLI 双端缺少路由管理命令群**：此前仅有只读的 `routes` 命令，缺乏 `add` 与 `del` 子命令；
  5. **Web UI 缺乏路由治理与实时事件感知**：Web 界面完全缺失物理路由卡片，路由变更时无法通过 SSE 实时更新。

- **架构与协议设计(ADR-039 物理寻址路由表全景透视、静态路由注入与断线自动重连治理)**:
  1. **物理路由透视模型与报告 DTO (`RouteEntryDto`, `RoutesReport`, `RouteMutateReport`)**:
     - 在 `runtime.rs` 与 `lib.rs` 引入结构化模型：
       ```rust
       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct RouteEntryDto {
           pub device_id: String,
           pub addr: String,
           pub alias: Option<String>,
           pub name: Option<String>,
           pub online: bool,
           pub trusted: bool,
       }

       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct RoutesReport {
           pub ok: bool,
           pub count: usize,
           pub routes: Vec<RouteEntryDto>,
       }

       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct RouteMutateReport {
           pub ok: bool,
           pub device_id: String,
           pub addr: Option<String>,
           pub action: String, // "added", "updated", "removed"
       }
       ```
     - 严格守卫安全红线：绝不输出私钥、对称传输密钥或配对 PIN。
  2. **存储层路由变更高阶原语 (`store.rs`)**:
     - `remember_route(dir, device_id, addr) -> Result<bool, NodeStoreError>`：原子读写 `routes.json`，存在则更新并返回 `true`，不存在则新增并返回 `false`；
     - `forget_route(dir, device_id) -> Result<bool, NodeStoreError>`：原子从 `routes.json` 剔除指定对端并返回是否实际移除了条目；
  3. **智能对端解析赋能与路由变更运行时原语 (`Node`)**:
     - `Node::routes_report(&self) -> RoutesReport`：聚合 `routes.json` 与 `live_peers`、`trusted` 列表，为每条路由注入实时在线状态与别名信息；
     - `Node::add_route(&self, peer_spec: &str, addr: &str) -> Result<RouteMutateReport, CoreError>`：通过 `resolve_peer` 智能解析 `peer_spec`，严格校验 `SocketAddr` 合法性（禁止端口 0），持久化至 `routes.json` 并派发 `UiEvent::RouteChanged { action: "added"|"updated" }`；
     - `Node::remove_route(&self, peer_spec: &str) -> Result<RouteMutateReport, CoreError>`：智能解析后从 `routes.json` 移除条目，派发 `UiEvent::RouteChanged { action: "removed" }`；
  4. **轻量级回环控制面端点 (`POST /api/routes`, `DELETE /api/routes`)**:
     - `GET /api/routes`：低开销返回全景 `RoutesReport`；
     - `POST /api/routes` 与 `PUT /api/routes`：支持 `{ peer, addr }` 或 `{ device_id, addr }` 静态路由注入与热修改；
     - `DELETE /api/routes`、`POST /api/routes/delete` 与 `POST /api/routes/remove`：支持 `?peer=...` Query 参数或 JSON 请求体快速安全删除；
     - 错误路径处理：非法套接字返回 400，端口 0 返回 400，未匹配或歧义对端返回 400/404，非合法 JSON 返回 400；
     - 安全红线：严格绑定 `127.0.0.1` 环回接口并校验 `Host: 127.0.0.1|localhost`。
  5. **CLI 双端全同构升级 (`tetherly route` 与 `tetherly-cli route`)**:
     - 新增 `Cmd::Route { action: RouteCmd }`，支持 `RouteCmd::List`, `RouteCmd::Add { peer, addr }`, `RouteCmd::Del { peer }` / `Remove`；
     - 保持向后兼容：`tetherly routes` 与 `tetherly-cli routes` 映射为 `route list`；
     - 支持 `--json` 格式化机器可读输出；
  6. **Web UI 物理路由治理面板与 SSE 实时同步 (`ui/index.html`)**:
     - 新增「物理寻址路由与静态注入」卡片；
     - 实时展示物理 IP:Port、对端 ID、别名与在线状态徽标；
     - 提供「+ 注入路由」交互式弹窗，支持对端别名/ID 与套接字输入；
     - 每条路由项提供「删除」确认按钮；
     - 监听 SSE `route_changed` 事件，零延迟刷新多端视图；
  7. **端到端集成测试全绿覆盖 (`tests/phase1.rs`)**:
     - 增设 `route_table_governance_and_static_injection_via_loopback_api`：
       1. 验证 `GET /api/routes` 初始状态（会话对端自动缓存）；
       2. 验证 `POST /api/routes` 静态注入与 `UiEvent::RouteChanged { action: "added" }` SSE 捕获；
       3. 验证落盘持久化与全景查询数据正确性；
       4. 验证 `PUT /api/routes` 地址变更与 `action: "updated"` SSE 捕获；
       5. 验证基于对端友好广播名/别名的智能解析注入与更新；
       6. 验证 `DELETE /api/routes?peer=...` 查询参数删除与 SSE 广播；
       7. 验证 `POST /api/routes/delete` 请求体智能别名删除与磁盘文件清空；
       8. 验证删除不存在路由返回 404，非法 IP/端口 0/空别名/歧义解析返回 400 防御；
       9. 优雅停机。
- **验证与全量测试全绿**：
  - 本地代码质量与静态规范严格遵循 `cargo fmt --check` 与 `clippy --workspace --all-targets -- -D warnings`；
  - GitHub Actions CI 跨平台 5 平台矩阵全绿（Run ID: `39081141144`）：
    - `android` (arm64-v8a NDK): ✓ in 1m41s
    - `deny` (advisories, bans, licenses, sources): ✓ in 1m7s
    - `check (macos-latest)`: ✓ in 2m10s
    - `check (ubuntu-latest)`: ✓ in 3m32s
    - `check (windows-latest)`: ✓ in 5m1s
  - 核心安全红线守卫：绝不输出私钥、对称会话密钥或配对 PIN 等敏感数据；Loopback 45716 仅绑定 127.0.0.1 且严格校验 Host；零 LGPL 链接；`UiEvent` 严禁携带明文敏感内容。

**1.45 相对 1.44(节点运行配置全景透视、动态运行时特性开关与轻量级配置回环治理)**

- **动机(节点配置黑盒、运行时特性静态冻结、动态重命名缺失与运维管控盲区)**:
  在 v1.18 至 v1.44 的演进中，Tetherly 完善了点对点通信、配对认证、文件传输及对端别名解析等控制面与数据面能力。然而在长期运行守护进程及多宿主、混合网络环境运维中，暴露出以下显著缺陷：
  1. **节点运行配置黑盒与快照轮询开销**：此前外部 CLI、监控脚本或 Web UI 若要探知当前节点的基础运行参数（如当前广播名称、底层平台、存储数据目录、回环限制状态、端口矩阵与扩展模块挂载状态），唯一途径是拉取并全量反序列化庞大的整机快照 `GET /api/state`。在包含大量拓扑对端与路由缓存时，造成不必要的 CPU 与内存开销；
  2. **运行时特性静态冻结（热重载缺失）**：节点启动后，节点广播名 `name`、mDNS 广播发现 `advertise` 开关、以及后台自动重连对端 `reconnect` 循环处于静态冻结状态；若用户进入移动/受限计费网络希望暂停重连循环，或在公共 Wi-Fi 下临时隐身本机（停止 mDNS 广播以规避网络探测与节能），或在不同工作空间切换广播显示名，此前必须杀掉守护进程修改配置文件并重启，直接导致所有正在进行的传输会话与活动链路强行中断；
  3. **CLI 缺乏配置检视与控制子命令**：双端命令行（`tetherly` 与 `tetherly-cli`）缺乏对节点自身配置的一键快速查询与热修改机制；
  4. **Web UI 缺乏配置治理交互与实时事件同步**：Web 界面没有暴露节点基础配置，无法就地修改本机名称或切换广播/重连策略。

- **架构与协议设计(ADR-038 节点运行配置全景透视、动态运行时特性开关与轻量级配置回环治理)**:
  1. **节点配置安全透视模型 (`NodeConfigReport`)**:
     - 在 `runtime.rs` 与 `lib.rs` 引入 `NodeConfigReport` DTO：
       ```rust
       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct NodeConfigReport {
           pub ok: bool,
           pub device_id: String,
           pub name: String,
           pub platform: String,
           pub data_dir: String,
           pub loopback_only: bool,
           pub advertise: bool,
           pub reconnect: bool,
           pub memory_clip: bool,
           pub ui_port: u16,
           pub control_port: u16,
           pub file_port: u16,
           pub input_port: u16,
           pub screen_port: u16,
           pub overlay_enabled: bool,
           pub ancs_attached: bool,
           pub ancs_peripheral: String,
       }
       ```
     - 严格守卫安全红线：绝不输出私钥、对称传输密钥或配对 PIN 等敏感凭据。
  2. **原子动态运行时状态与热重载治理 (`Node` / `Shared`)**:
     - `node_name: Mutex<String>`：启动时由 `config.name` 初始化，动态修改严格校验非空（1..=64 字节）且无控制字符；
     - `advertise: AtomicBool` 与 `reconnect: AtomicBool`：提供无锁原子读取与即时切换；
     - `advertise_stop: Mutex<Option<watch::Sender<bool>>>`：mDNS 广播任务接入取消信号通道，在动态开启/关闭或改名时平滑停止或重新拉起广播循环；
     - `spawn_reconnect`：后台重连循环每轮轮询 `node.is_reconnect_enabled()`，关闭时挂起等待，开启时即时唤醒恢复重连尝试；
     - 每次变更触发 `Node::emit_config_changed`，广播 `UiEvent::ConfigChanged { name, advertise, reconnect }`。
  3. **轻量级回环控制面端点 (`GET /api/config`, `POST /api/config`, `PUT /api/config`)**:
     - `GET /api/config`：极低开销返回 `NodeConfigReport`；
     - `POST /api/config` 与 `PUT /api/config`：接收 `ConfigUpdateBody { name, advertise, reconnect }`，支持单字段或多字段联合更新；
     - 错误路径处理：空请求体返回 400（"no configuration fields to update"），非法名称返回 400，非合法 JSON 返回 400；
     - 安全红线：严格绑定 `127.0.0.1` 环回接口并校验 `Host: 127.0.0.1|localhost`。
  4. **CLI 双端全同构 (`tetherly config` 与 `tetherly-cli config`)**:
     - 新增 `Cmd::Config { name, advertise, reconnect, json, ui_port }`；
     - 无参数时输出整洁的节点运行全景配置卡片（设备 ID、名称、平台、数据目录、网络发现状态、端口列表等）；
     - 支持 `--name <NAME>` 动态重命名本机；
     - 支持 `--advertise <true|false>` 动态启闭 mDNS 广播；
     - 支持 `--reconnect <true|false>` 动态启闭后台重连；
     - 支持 `--json` 输出机器可读格式。
  5. **Web UI 动态配置面板与 SSE 实时同步 (`ui/index.html`)**:
     - 新增「节点配置与运行特性」卡片；
     - 展示节点设备 ID、当前名称、平台、数据目录及端口列表；
     - 提供「改名」按钮触发即时提示框，提供广播开关与自动重连开关；
     - 监听 SSE `config_changed` 事件，零延迟刷新多端视图。
  6. **端到端集成测试全绿覆盖 (`tests/phase1.rs`)**:
     - 增设 `node_config_inspection_and_dynamic_flags_via_loopback_api`：
       1. 验证 `GET /api/config` 初始状态字段完备性；
       2. 验证 `POST /api/config` 动态改名与 `UiEvent::ConfigChanged` SSE 事件捕获；
       3. 验证 `GET /api/health` 同步反映新名称；
       4. 验证 `PUT /api/config` 动态特性开关开启与 SSE 事件通知；
       5. 验证单字段局部更新（`reconnect: false`）；
       6. 验证空载荷、空名称、超长名称、控制字符及非法 JSON 的 400 校验防御；
       7. 优雅关闭节点。

**1.44 相对 1.43(对端设备别名治理、友好命名持久化与对端标识智能解析)**

- **动机(对端别名静态冻结、加密 DeviceId 输入摩擦高与多端识别割裂)**:
  在 v1.18 至 v1.43 的演进中，Tetherly 建立了基于 Noise 握手与 Ed25519 的点对点安全网格拓扑。然而随着多设备生态（多台笔记本、工作站、手机、云机）日常交互的深化，暴露出以下严重的控制面与用户体验缺陷：
  1. **受信任别名静态冻结**：初次配对成功后，`TrustedPeer.alias` 被一次性赋予握手帧中的 `hello.name`，之后没有任何途径修改。若对端系统改名、或者主机名默认为枯燥的系统型号（如 `DESKTOP-8K39LF2`、`localhost`），本地用户只能永久面对无意义的标识；
  2. **加密 `DeviceId` 交互摩擦巨大**：此前所有对端定向操作（CLI 的 `--peer`、API 的 `peer` 参数，包括 `send`、`ping`、`notify`、`clip`、`screen start`、`input start`、`unpair`）均强制要求输入完整的 21 字符 Base32 编码加密标识（如 `tdev_u2z4x6m8...`）。用户在终端或脚本中几乎无法盲敲，必须反复运行 `tetherly peers` 复制粘贴；
  3. **缺少别名原子持久化与变更通知**：此前 `TrustStore` 特征仅提供 `get`, `put`, `forget`, `remove`，缺乏就地更新别名的机制；即便重新 `put` 也无法向活跃会话通知别名变更，UI 无法动态响应。

- **架构与协议设计(ADR-037 对端设备别名治理、友好命名持久化与对端标识智能解析)**:
  1. **TrustStore 别名原子变更与文件持久化**:
     - 在 `tetherly-core` 的 `TrustStore` trait 及 `MemoryTrustStore` 增设 `update_alias(&mut self, id: &DeviceId, alias: &str) -> Result<bool, CoreError>`；
     - 在 `tetherly-node::store` 的 `save_trust` 与 `load_trust` 保障原子写盘（`trusted.json`），增设单测验证别名更新落盘生命周期；
  2. **智能对端解析系统 (`Node::resolve_peer`)**:
     - 引入自适应三级解析流水线：
       1. 优先尝试直接反序列化为加密 `DeviceId`（以 `tdev_` 开头且 Base32 合法）；
       2. 尝试受信任对端 `alias` 或在线对端广播名 `name` 的全量精确匹配（大小写不敏感）；
       3. 尝试在 `alias`、`name` 或 `device_id` 字符串中的不区分大小写包含/前缀匹配；
     - 歧义与冲突严格防御：若未找到匹配项返回 `CoreError::PeerNotFound("no peer matching '...' found")`；若多个设备命中则返回 `CoreError::PeerNotFound("multiple peers match '...': id1, id2")`，杜绝误操作；
  3. **轻量级回环控制面端点 (`POST /api/peers/alias`)**:
     - 路由接入 `POST /api/peers/alias` 与 `PUT /api/peers/alias`；
     - 别名校验安全防御：非空（1..=64 字节），严禁包含控制字符；
     - 成功更新后自动持久化并向事件总线广播 `UiEvent::PeerAliasChanged`；
     - 全域端点赋能：将 `node.resolve_peer()` 贯穿 `POST /api/peers/unpair`, `POST /api/peers/ping`, `POST /api/file/send`, `POST /api/notify`, `POST /api/clip/sync`, `POST /api/screen/start`, `POST /api/input/start`；
  4. **CLI 双端全同构升级 (`tetherly` 与 `tetherly-cli`)**:
     - 增设 `alias <peer> <alias>` 命令，支持根据设备 ID 或当前名字修改友好别名；
     - `peers` 命令升级：在线列表智能显式呈现 `name (alias: "...")`，受信任列表呈现别名与在线状态；
     - 所有子命令的 `--peer` 选项全面无缝支持友好别名和模糊匹配；
  5. **Web UI 交互闭环与安全 SSE 事件流**:
     - `ui/index.html` 在活动会话与受信任设备卡片增设「别名」按钮，点击弹出原生轻量提示框完成快速修改；
     - 监听 SSE 事件 `peer_alias_changed`，安全红线保障：载荷仅包含 `device_id` 与合规 `alias`，严禁敏感数据泄露；
  6. **端到端集成测试覆盖 (`tests/phase1.rs`)**:
     - 增设 `peer_alias_governance_and_smart_resolution_via_loopback_api` 完备验证别名查询、更新、持久化、SSE 广播、智能解析跨端调用与解绑。

**1.43 相对 1.42(配对控制面治理、PIN 码安全生命周期管理与 CLI 拨号配对全同构)**

- **动机(PIN 码生命周期黑盒、无法主动撤销、CLI 双端异构与 Web 状态断层)**:
  在 v1.18 至 v1.42 的演进中，Tetherly 建立了基于 SPAKE2 与 8 位数字 PIN 的安全初次配对流程，并在 `tetherly` 二进制中提供了向正在运行的守护进程请求生成 PIN 的功能。然而在实际使用与多端同构治理中，暴露出以下缺陷：
  1. **PIN 码状态单向黑盒**：此前只有 `POST /api/pin` 单向生成或通过 `POST /api/dial` 注入 PIN，缺乏 `GET /api/pin` 查询端点。调用方与 Web UI 无法获取当前是否存在活跃 PIN 以及剩余有效期倒计时；
  2. **缺少主动撤销机制**：PIN 具有 180 秒（`PIN_TTL`）超时机制。若用户误触生成 PIN 或中途取消配对，此前无法通过任何 API 主动撤销武装状态，必须被动等待超时，存在安全暴露时间窗口；
  3. **独立 CLI (`tetherly-cli`) 拨号配对异构与失效**：`bins/tetherly-cli` 中的 `Cmd::Pin` 仅在本地生成 throwaway 随机数，从未通知守护进程；其 `Cmd::Dial` 则直接启动 Phase 0 临时进程直连，与正在运行的守护进程完全脱节；
  4. **Web UI 状态恢复缺失**：网页刷新后 PIN 显示框立即重置为 `•••• ••••`，用户无法查验刚生成的 PIN 是否依然处于有效期内。

- **架构与协议设计(ADR-036 配对控制面治理、PIN 码安全生命周期管理与 CLI 拨号配对全同构)**:
  1. **PIN 报告模型 (`PinReport`)**:
     - 在 `runtime.rs` 与 `lib.rs` 引入 `PinReport` DTO：包含 `ok: bool`, `active: bool`, `pin: Option<String>`, `expires_in_secs: u64`, `ttl_secs: u64`, `created_at_ms: u64`；
     - `Node::pin_report(&self) -> PinReport`：检查 `self.shared.pin`，计算当前与生成时刻的时间差 `age`，在 `PIN_TTL` 内返回格式化数字与剩余秒数，超时或无 PIN 返回 `active: false`；
     - `Node::clear_pin(&self) -> bool`：原子清除 `Shared.pin`，若此前处于活跃状态则广播 `UiEvent::PinStateChanged { active: false }` 并返回 `true`；
     - `Node::generate_pin(&self)`：生成后广播 `UiEvent::PinStateChanged { active: true }`；
  2. **轻量级回环控制端点 (`GET /api/pin` 与 `POST /api/pin/clear`)**:
     - `GET /api/pin`：无快照序列化开销，直接返回 `PinReport`；
     - `POST /api/pin/clear` 与 `DELETE /api/pin`：主动撤销并清除武装 PIN，返回 `{"ok": true, "cleared": bool}`；
     - `POST /api/pin`：增强返回体，包含 `expires_in_secs` 与 `ttl_secs`；
  3. **双端 CLI 全量同构 (`tetherly` 与 `tetherly-cli`)**:
     - `pin` 子命令扩充 `--status`（查询当前 PIN 状态与剩余时间）、`--clear`（主动撤销 armed PIN）、`--json`（机器可读输出）与 `--ui-port`；
     - `dial` 子命令统一改造为回环客户端，将目标地址与可选对端 PIN 统一递送至运行中的主守护进程；
  4. **Web UI 体验升级 (`ui/index.html`)**:
     - 增设「撤销 PIN」按钮与动态有效倒计时展示；
     - 页面载入和定时轮询时通过 `GET /api/pin` 自动恢复当前有效 PIN；
     - 接入 SSE `pin_state_changed` 事件，感知 PIN 武装与撤销状态；
  5. **安全红线铁律（Verbatim Security Red Line）**:
     - `UiEvent` 与对外日志绝对禁止包含任何明文 PIN 数字或敏感认证信息（`PinStateChanged` 仅携带 `active: bool`）；
     - 回环 API 严格绑定 `127.0.0.1` 并强校验 `Host: 127.0.0.1|localhost`（不匹配返回 403）。

**1.42 相对 1.41(文件传输通道全景可观测性、轻量级传输查询与命令行同构治理)**

- **动机(大文件传输可观测性盲区、快照轮询性能损耗与专用传输管理能力缺失)**:
  在 v1.28 至 v1.41 的演进中，Tetherly 支持最高 64 GiB 流式大文件传输并建立了任务取消与会话累计度量机制。然而在生产高并发与长周期大文件传输实践中，暴露出以下架构痛点：
  1. **状态快照轮询高开销**：此前外部 CLI、无头脚本或 Web UI 若要感知传输实时进度与速度，必须频繁轮询 `GET /api/state`。在有数十个活跃对端或包含复杂路由缓存与 ANCS 白名单时，整机快照的全量 JSON 序列化造成不必要的 CPU 与内存开销；
  2. **缺少独立精准查询与多维过滤端点**：无法直接根据特定的 `transfer_id` 查询单个任务的精确详情（包含耗时、当前相位、错误信息等），也无法仅检索正在在途传输（`active`）或已终态（`terminal`）的任务；
  3. **CLI 缺少专属传输治理命令**：此前 CLI 仅有 `send`、`cancel` 与 `clear-transfers`，缺乏直观查看传输列表、进度与速率的 `transfers` 命令；
  4. **Web 传输交互与统计割裂**：Web 界面缺少轻量级传输独立刷新按键，传输度量与局部刷新机制有待加强。

- **架构与协议设计(ADR-035 文件传输通道全景可观测性、轻量级传输查询与命令行同构治理)**:
  1. **全景传输汇总数据契约 (`TransfersReport`)**:
     - 在 `runtime.rs` 与 `lib.rs` 引入 `TransfersReport` DTO：包含 `ok: bool`, `active_count: usize`, `terminal_count: usize`, `total_count: usize`, `transfers: Vec<ActiveTransfer>`, `stats: TransferStats`；
     - `Node::active_transfers(&self) -> Vec<ActiveTransfer>` 按任务启动时间倒序输出；
     - `Node::transfer_by_id(&self, id: &str) -> Option<ActiveTransfer>` 针对单任务执行 $O(1)$ 哈希表查找；
     - `Node::transfers_report(&self) -> TransfersReport` 聚合生成全景视图；
  2. **轻量级回环查询端点 (`GET /api/file/transfers` & `GET /api/transfers`)**:
     - 严格绑定 `127.0.0.1` 并强校验 `Host`（非法请求返回 403）；
     - 双模参数支持：同时支持 URI Query String (`?id=...&active=true...`) 与 POST JSON 请求体；
     - 单任务精准查询：当提供 `id` 时，命中返回 200 及 `ActiveTransfer`，未找到返回 HTTP 404；
     - 列表动态过滤：支持 `active=true`（仅保留 `Transferring` 与 `Verifying`）、`terminal=true`（仅保留 `Completed` 与 `Failed`）以及 `direction=inbound|outbound` 过滤；
  3. **双端同构命令行 (`tetherly` 与 `tetherly-cli`)**:
     - 增设 `Cmd::Transfers`，支持 `--id <tfl_...>`, `--active`, `--terminal`, `--direction <in|out>`, `--json`, `--ui-port <port>`；
     - 格式化输出任务传输百分比、已传输量/总量 (`format_bytes`)、实时速率 (`format_speed`) 及累计会话指标；
  4. **Web UI 局部刷新与全景交互 (`ui/index.html`)**:
     - 增设「刷新传输」按键，调用轻量级 `/api/file/transfers` 实时渲染；
     - 清除历史记录后无缝触发轻量级刷新；
  5. **安全红线铁律（Verbatim Security Red Line）**:
     - 回环接口仅监听 `127.0.0.1`，严格校验 `Host`；
     - 正在进行的大文件传输任务分块文件严防误删；
     - 传输查询接口严禁输出或泄漏任何超出传输控制元数据之外的宿主隐私内容。

**1.41 相对 1.40(对端加密通道往返时延 (RTT) 实时测速、网络质量度量与主动链路探测)**

- **动机(网络质量与链路状态黑盒化、对端时延不可见与按需主动探测缺失)**:
  在 v1.0 至 v1.40 的演进中，节点各核心通信通道均已建立在 Noise 加密隧道之上，并在后台由心跳循环定时发送 `TYPE_PING` (0x0001) 与 `TYPE_PONG` (0x0002) 帧。然而在实际多设备组网与网络质量运维中暴露出以下痛点：
  1. **链路时延黑盒化**：此前节点虽然在后台相互回应 Ping/Pong 帧，但在收到 Pong 时直接在事件匹配分支 `SessionEvent::Pong(_) => {}` 中丢弃了时间戳，从未记录或计算网络往返时延 (RTT)；
  2. **网络瓶颈无法定位**：当大文件传输降速、远程屏幕卡顿或跨局域网/EasyTier 虚网链路发生拥塞抖动时，运维人员与用户无法得知物理链路真实时延，无法判断是网络丢包、延迟激增还是系统负载高；
  3. **缺乏按需链路主动探测与度量能力**：CLI 与 Web UI 均无法主动向特定在线对端发送探测包并获取实时响应时延；
  4. **事件总线与控制面状态缺失时延度量**：`LivePeer` 缺少 RTT 字段，`GET /api/peers` 与 `GET /api/state` 无法呈现链路健康度。

- **架构与协议设计(ADR-034 对端加密通道往返时延 (RTT) 实时测速、网络质量度量与主动链路探测)**:
  1. **Noise 加密通道内微秒级往返时延 (RTT) 闭环度量**:
     - 充分复用已有的内层帧线协议 `TYPE_PING` (0x0001) 与 `TYPE_PONG` (0x0002)，其负载为大端序 8 字节毫秒时间戳 `unix_ms: u64`；
     - 接收端在收到 Ping 帧时立即原样返回 Pong 帧；
     - 发送端在收到 Pong 帧时，计算 `rtt_ms = now_ms.saturating_sub(ts)`，即刻获得经过 Noise 端到端加密隧道的精确往返时延；
  2. **异步等待派发器与按需探测端点 (`POST /api/peers/ping`)**:
     - 在运行时 `Shared` 中维护 `ping_waiters: Mutex<HashMap<DeviceId, Vec<oneshot::Sender<u64>>>>`；
     - `POST /api/peers/ping` 支持入参 `{"peer": "<tdev_...>"}`：若未指定且仅有一个在线对端则自动解析；
     - 调用 `Node::ping_peer(&self, peer_id: &DeviceId) -> Result<PeerPingReport, CoreError>`；
     - 立即向对端发送 Ping 帧并在 oneshot channel 上异步等待，设 5 秒超时保护；
     - 收到 Pong 时唤醒所有等待者并返回结构化 `PeerPingReport { ok: true, device_id, rtt_ms, path, addr }`；
  3. **LivePeer 状态暴露与拓扑可观测性**:
     - `LivePeer` 结构增加 `rtt_ms: Option<u64>` 与 `last_seen_ms: u64`；
     - 收到 Pong 响应时自动更新对应 peer 的 `rtt_ms` 与 `last_seen_ms`；
     - `GET /api/peers` 与 `GET /api/state` 快照全量暴露该度量数据；
  4. **实时时延流式广播 (`UiEvent::PeerLatency`)**:
     - 每次收到 Pong 响应时，通过节点内部事件总线广播 `UiEvent::PeerLatency { device_id: String, rtt_ms: u64 }`，经由 SSE 连接流式推送至所有前端监听者；
  5. **双端 CLI 同构测速命令 (`tetherly` 与 `tetherly-cli`)**:
     - `tetherly peers --ping [--peer <id>] [--count <n>] [--json]`：对在线对端进行加密通道延迟探测；
     - `tetherly ping --peer <id> [--count <n>] [--json]`：定向探测对端 Noise 通道时延；无 `--peer` 参数时保持原有对本地守护进程的回环健康探针行为；
     - 支持 `--count <n>` 多包探测并统计 `min / avg / max` 毫秒时延，支持 `--json` 管道输出；
  6. **Web UI 实时时延徽标与一键测速 (`ui/index.html`)**:
     - 对端卡片增设动态时延徽标（<50ms 绿，<150ms 蓝，>=150ms 黄）；
     - 增设一键「测速」交互按钮，完成后动态更新时延数值与徽标状态；
  7. **严格安全红线**:
     - Ping 探测帧全程运行在已建立的 Noise 加密隧道内部，严禁明文传输任何探测包，绝对不泄露私钥或凭据信息；
  8. **端到端集成测试全覆盖**:
     - 在 `tests/phase1.rs` 增设 `active_peer_rtt_probing_and_latency_reporting_via_loopback_api`，全面验证直接 Rust API、HTTP 回环 API（单对端自解析与定向探测）、错误参数拒绝、`GET /api/peers` RTT 更新、`GET /api/state` 镜像与 SSE `PeerLatency` 事件流分发。

**1.40 相对 1.39(节点事件总线实时流式推送 (SSE)、实时事件可观测性与 CLI 事件监听闭环)**

- **动机(高频轮询开销治理、微秒级事件响应性与无头事件流水线需求)**:
  在 v1.0 至 v1.39 的演进中，节点各核心子系统（对端上线/离线、文件传输到达与完成、通知候选与回复、配对请求与网络状态）均会在节点内部事件总线派发 `UiEvent`。然而由于此前回环接口仅提供请求-响应模型，客户端只能通过高频（700ms 间隔）轮询 `GET /api/state` 获取状态变化。这导致：
  1. **高频轮询的资源浪费与延迟瓶颈**：全量快照包含对端拓扑、路由、剪贴板、多通道传输与状态，即使无事件发生也反复序列化与解析；且最多存在 700ms 的感知延迟；
  2. **无头自动化流水线无法高效等待特定事件**：外部脚本或 CI 测试需要一种类似于 `tail -f` 的命令行工具来实时捕获节点事件；
  3. **标准 SSE 协议缺失**：浏览器原生具备极度高效的 `EventSource`，基于 HTTP/1.1 纯文本流，相比复杂的 WebSocket 更加轻量，与现有轻量 HTTP 回环架构天然契合。

- **架构与协议设计(ADR-033 节点事件总线实时流式推送 (SSE)、实时事件可观测性与 CLI 事件监听闭环)**:
  1. **标准 Server-Sent Events 端点 (`GET /api/events` 与 `GET /api/events/stream`)**:
     - 严格遵循 127.0.0.1 绑核与 Host 头校验；
     - 响应头：
       ```http
       HTTP/1.1 200 OK\r\n
       Content-Type: text/event-stream; charset=utf-8\r\n
       Cache-Control: no-cache, no-transform\r\n
       Connection: keep-alive\r\n
       Access-Control-Allow-Origin: *\r\n\r\n
       ```
     - 初始连接握手发送 `: connected\n\n`；
     - 每 15s 发送一次 `: ping\n\n` 保持连接活性；
     - 每次事件推送：`data: <UiEvent JSON>\n\n`；
     - 节点停止时：收到 `node.shutdown_signal()` 信号平滑跳出并关闭 TCP 连接。
  2. **双端 CLI 同构事件流监听命令 (`tetherly events` 与 `tetherly-cli events`)**:
     - `events [--count <n>] [--timeout-secs <s>] [--json] [--ui-port <port>]`；
     - 支持最大接收事件数截断退出与总超时退出；
     - 支持标准格式化终端高亮打印与 raw JSON 管道流输出。
  3. **Web UI 事件响应化改造 (`ui/index.html`)**:
     - 创建 `EventSource("/api/events")`，自动重连；
     - 事件到达时即时触发 `tick()` 刷新界面，背景兜底轮询由 700ms 降至 2000ms+；
     - 增设 SSE 状态徽标与最近事件跑马灯。
  4. **严格数据脱敏安全红线**:
     - `UiEvent` 严格禁止携带短信正文、OTP 验证码、配对码与剪贴板文本；
     - 仅包含状态变化、传输 ID、设备 ID、对端名称等系统级元数据。

**1.39 相对 1.38(节点健康检查探针、轻量级守护进程可观测性与优雅停止机制)**

- **动机(高频探针与全量状态解耦、进程级核心可观测性与受控平滑退出机制)**:
  在 v1.0 至 v1.38 的演进中，节点控制面功能已涵盖配对、路由、传输、通知、剪贴板、键鼠、屏幕、网卡与存储等众多领域。然而在实际系统集成、高可用部署与守护进程管理中暴露出以下痛点：
  1. **高频探活与沉重全量状态耦合**：外部系统（如 systemd、Docker、K8s 探针或无头监控脚本）此前只能请求 `GET /api/state` 检查节点存活性。该接口必须锁定并序列化全量活动传输、设备列表、候选队列与路由信息，随着系统运行与负载升高，开销剧增，极不适应 1s~2s 级别的健康巡检；
  2. **守护进程指标黑盒**：系统缺少对进程 PID、系统持续运行时间（Uptime）、操作系统平台与各核心网络端口（control 45717, file 45718, input 45719, screen 45720, ui 45716）绑定状态的单一零开销透视源；
  3. **缺乏受控平滑的优雅停止机制**：节点在 `Node::run(&self)` 中此前依赖 `std::future::pending()` 无限阻塞，终止进程只能依赖外部硬性 `SIGTERM` / `SIGKILL`，容易造成正在写入的连接或临时缓冲异常中断；
  4. **缺乏开箱即用的轻量 CLI ping/health/stop 工具**：无头终端缺少轻量级延迟检测与守护进程管理命令。

- **架构与协议设计(ADR-032 节点健康检查探针、轻量级守护进程可观测性与优雅停止机制)**:
  1. **轻量级健康探针端点 (`GET /api/health`)**:
     - 严格遵循 127.0.0.1 绑核与 Host 头校验；
     - 响应结构：
       ```json
       {
         "ok": true,
         "status": "healthy",
         "version": "1.39.0",
         "device_id": "c0ffee...",
         "name": "desktop-node",
         "platform": "macos",
         "pid": 12345,
         "uptime_secs": 3600,
         "ports": {
           "control": 45717,
           "file": 45718,
           "input": 45719,
           "screen": 45720,
           "ui": 45716
         },
         "live_peers": 2,
         "trusted_peers": 3
       }
       ```
  2. **优雅停止控制面 (`POST /api/daemon/shutdown` 与 `POST /api/daemon/stop`)**:
     - 异步处理请求：先向客户端发送 HTTP 200 `{"ok": true, "status": "shutting_down", "message": "node shutdown initiated"}`，延时 50ms 触发 `node.shutdown()`；
     - `Node::shutdown()` 广播 `watch::Sender<bool>`；
     - `Node::run(&self)` 接收到通知后退出等待循环，返回 `Ok(())`；
     - `uihttp::accept_loop` 与所有 `node_accept_loop` 接收到停止信号后终止监听。
  3. **CLI 全量同构命令群 (`health`, `ping`, `stop`)**:
     - `health [--json] [--ui-port <port>]`：格式化输出节点运行状态或 JSON；
     - `ping [--count <n>] [--timeout-ms <ms>] [--ui-port <port>]`：连续向回环健康端点发送请求并测算 RTT 往返时延；
     - `stop [--ui-port <port>]`：触发受控优雅停止。
  4. **Web UI 补全 (`ui/index.html`)**:
     - 增设「守护进程与系统健康」卡片，集成存活状态、PID、运行时长、端口绑定、实时 RTT 测速与受控优雅停止。
  5. **端到端集成测试 (`tests/phase1.rs`)**:
     - 增设 `node_health_probe_ping_and_graceful_shutdown_via_loopback_api`。

**1.38 相对 1.37(节点数据目录全景可观测性、存储配额分析与孤儿分块垃圾回收)**

- **动机(存储状态黑盒、大文件传输孤儿分块堆积隐患与控制面治理空白)**:
  在 Phase 1（v1.26 至 v1.28）落地流式大文件传输（最高 64 GiB）与断点续传（`ResumeTable` / `.resume.json` 侧车）后，文件传输引擎运行高度稳健。然而在长周期的系统运维与无头环境下暴露了存储治理盲区：
  1. **存储状态黑盒**：外部监控、无头命令行与 Web UI 无法获取当前节点数据目录（`data_dir`）实际路径、接收收件箱（`inbox`）空间占用、已接收文件总数、元数据占用与临时分块文件状况；
  2. **大文件传输孤儿分块堆积隐患**：当传输发生网络异常、对端设备永久离线、传输被用户取消或节点进程异常中断时，临时分块文件 `.{transfer_id}.part` 与 `.resume.json` 侧车可能长期滞留磁盘，缺乏统一可观测透视与安全垃圾回收机制；
  3. **命令行与回环控制面缺口**：CLI 与 Web UI 缺少专属的 `storage` 命令群与轻量级回环端点，无法在不借助外部 OS shell 的前提下直接排查与清理存储空间。

- **架构与协议设计(ADR-031 节点数据目录全景可观测性、存储配额分析与孤儿分块垃圾回收)**:
  1. **轻量级回环 HTTP 控制面 (`GET /api/storage` 与 `POST /api/storage/clean`)**:
     - 严格遵循 127.0.0.1 绑核与 Host 头校验；
     - `GET /api/storage` 返回模型：
       ```json
       {
         "ok": true,
         "data_dir": "/path/to/data",
         "inbox_dir": "/path/to/data/inbox",
         "inbox_total_bytes": 1048576,
         "inbox_file_count": 5,
         "partial_bytes": 2097152,
         "partial_count": 2,
         "orphaned_partial_count": 1,
         "orphaned_partial_bytes": 1048576,
         "meta_bytes": 4096
       }
       ```
     - `POST /api/storage/clean` 支持接收可选 JSON `{"orphaned_only": bool}`（默认 `true`），返回清理报告：
       ```json
       {
         "ok": true,
         "freed_bytes": 1048576,
         "deleted_files_count": 2,
         "cleared_resume_entries": 0
       }
       ```
  2. **节点运行时透视与垃圾回收 (`Node::storage_report` 与 `Node::clean_storage`)**:
     - 单遍扫描 `data_dir` 与 `inbox`，精准分类已完成文件、活跃分块、可续传分块与孤儿分块；
     - 垃圾回收严格实行安全隔离：处于 `Transferring` 或 `Verifying` 阶段的在途任务严禁删除；`orphaned_only: true` 仅清理失配孤儿文件，`orphaned_only: false` 清理所有非在途未决分块并注销内存断点续传表；
  3. **CLI 命令行全量同构 (`tetherly` 与 `tetherly-cli`)**:
     - `storage status [--json] [--ui-port <port>]`：展示数据目录、收件箱用量、分块用量与孤儿统计；
     - `storage clean [--all] [--ui-port <port>]`：触发垃圾回收，打印释放空间与删除文件数；
  4. **Web UI 补全 (`ui/index.html`)**:
     - 在 Web UI 中增设「存储与数据目录」面板，提供存储用量多维透视与一键清理孤儿分块交互；
  5. **端到端集成测试全覆盖**:
     - 在 `tests/phase1.rs` 增设 `storage_inspection_and_partial_cleanup_via_loopback_api`，全面验证初始指标、收件箱文件计数、孤儿分块识别、安全清理执行与正常文件保护。

**1.37 相对 1.36(ANCS 苹果通知服务深层可观测性、白名单规则治理与命令行回环全同构)**

- **动机(ANCS 状态黑盒、缺乏轻量级诊断 API 与 CLI 控制子命令空白)**:
  在 Phase 4（v1.23）落地 Windows WinRT BLE GATT 驱动后，电脑端已支持通过蓝牙低功耗接入 iPhone 的 Apple Notification Center Service (ANCS)。然而，在无头运维、脚本自动化与日常桌面交互中仍存在关键断层：
  1. **缺少轻量级专用状态接口**：此前仅能通过全量快照 `GET /api/state` 获取 `ancs_state: String`（如 `"off"` 或状态名）以及 `open_apps: Vec<String>`。每次查询都要承受遍历全量信任库、活跃传输、网卡分类与屏幕帧统计的性能惩罚，且丢失了传输栈底层连接态（`attached: bool`）与退避重试倒计时（`retry_at_ms: Option<u64>`）；
  2. **CLI 缺乏 ANCS 控制与治理子命令**：`tetherly` 与 `tetherly-cli` 均无 `ancs` 子命令群。运维人员在终端无法查看当前 ANCS 连接状态，无法通过命令行触发与 iPhone 的重连订阅握手，也无法动态向安全白名单（`OpenAllowlist`）注入允许跳转的 App 规则；
  3. **Web UI 白名单管理不完整**：Web UI 缺乏统一的 ANCS 状态视图与白名单规则追加交互，用户无法直观查阅已授权的桌面外部应用跳转规则。

- **架构与协议设计(ADR-030 ANCS 苹果通知服务深层可观测性与规则治理)**:
  1. **回环 HTTP API 扩充 (`GET /api/ancs`)**:
     - 严格遵循 127.0.0.1 绑核与 Host 头校验；
     - 返回结构：
       ```json
       {
         "ok": true,
         "attached": true,
         "state": "ready",
         "retry_at_ms": null,
         "allowlist": ["com.apple.MobileSMS", "com.tencent.xin"]
       }
       ```
  2. **节点运行时扩展 (`Node::ancs_report`)**:
     - 聚合底层传输栈是否存在 (`attached`)、当前 Ingress 状态名 (`state`)、退避重试时间戳 (`retry_at_ms`) 与当前生效的应用白名单 (`allowlist`)；
  3. **CLI 命令行全量同构 (`tetherly` 与 `tetherly-cli`)**:
     - `ancs status [--json] [--ui-port <port>]`：展示 ANCS 传输栈、状态、重试时间与允许跳转的 App 清单；
     - `ancs connect [--ui-port <port>]`：向已连接的 iPhone 发起 ANCS 订阅通道握手；
     - `ancs allow <app_id> [--url <template>] [--ui-port <port>]`：动态向白名单注入允许跳转的应用规则；
  4. **Web UI 补全 (`ui/index.html`)**:
     - 在 Web UI 中增设「ANCS 苹果通知服务」卡片，提供状态指示徽标、重连控制按钮、白名单规则展现与输入添加表单；
  5. **端到端集成测试全覆盖**:
     - 在 `tests/phase4.rs` 增设 `ancs_status_inspection_connection_and_allowlist_via_loopback_api`，全面验证无 transport 时返回 `off`、动态追加白名单实时生效、mock transport 触发握手与状态跃迁，以及 400 校验防御。

**1.36 相对 1.35(网卡物理分类透视、EasyTier 侧车穿透诊断与网络探测回环闭环)**

- **动机(网络分类黑盒、EasyTier 侧车穿透诊断缺失与轻量级网络 API 空白)**:
  在 v1.0 至 v1.35 架构中，系统通过 `tetherly_core::bind::classify`（ADR-024/ADR-018）对本机网络接口进行严格物理分类并给出跳过判定（如 LoopbackAlias、LinkLocal、CGNAT Overlay 等），并通过 `sidecar.rs` 探测外部自建的 EasyTier 虚拟组网。但在现场诊断与无头运维中存在明显痛点：
  1. **缺少轻量级网络诊断接口**：此前仅能通过 `GET /api/state` 的全量快照拉取 `lan: Vec<IfaceReport>`，伴随大量无关字段（候选通知、屏幕统计、活跃传输等），外部诊断探针与监控无法以毫秒级进行轻量网卡轮询；
  2. **EasyTier 侧车虚拟 IP 拓扑黑盒**：此前 `Snapshot` 仅粗略包含 `overlay_present`、`overlay_source` 与 `overlay_peers`（数量），而 EasyTier 本机虚拟 IP（`local_ips`）与已探测到的虚拟对端（`peers`）被隐藏在内存中，运维人员无法得知当前分配到的虚拟 IP 是什么，也无法定位虚拟直连目标；
  3. **CLI 缺乏网络与接口诊断工具**：`tetherly` 与 `tetherly-cli` 均无网络诊断子命令，在纯终端环境下无法直接列出物理网卡监听与跳过原因，亦无法确认 EasyTier 侧车连通性；
  4. **Web UI 网卡跳过原因不可见**：此前 UI 仅展示一行“仅局域网”或“探测 EasyTier 侧车…”，用户无法在界面上直观查看哪些网卡被识别并监听、哪些因何种原因（如 `127.0.0.53` 属于系统别名，`100.64.0.0/10` 属于 CGNAT）被安全跳过。

- **架构与协议设计(ADR-029 网卡物理分类透视与侧车穿透诊断)**:
  1. **回环 HTTP API 扩充 (`GET /api/net`)**:
     - 严格遵循 127.0.0.1 绑核与 Host 头校验；
     - 返回结构：
       ```json
       {
         "ok": true,
         "interfaces": [
           {
             "iface": "en0",
             "ip": "192.168.1.100",
             "listen": true,
             "reason": null
           },
           {
             "iface": "lo0",
             "ip": "127.0.0.1",
             "listen": true,
             "reason": null
           }
         ],
         "overlay": {
           "enabled": true,
           "present": false,
           "source": "Absent",
           "local_ips": [],
           "peer_count": 0,
           "peers": []
         }
       }
       ```
  2. **节点运行时状态暴露 (`Node::network_report`)**:
     - 返回 `(Vec<IfaceReport>, OverlayStatus)`，由 `lan::interface_report()` 与 `sidecar::discover()` 聚合产生；
  3. **CLI 命令行全量同构 (`tetherly` 与 `tetherly-cli`)**:
     - `net [--interfaces] [--json] [--ui-port <port>]`：默认打印 EasyTier 侧车状态概览与监听中的局域网地址；指定 `--interfaces` 时完整打印包含跳过网卡及其理由的格式化诊断表格；支持 `--json` 输出结构化数据；
  4. **Web UI 补全 (`ui/index.html`)**:
     - 在网络配置卡片增加网卡分类诊断详情表格与 EasyTier 拓扑展开视图；
  5. **端到端集成测试全覆盖**:
     - 在 `tests/phase2.rs` 增设 `network_interface_classification_and_sidecar_inspection_via_loopback_api`。

**1.35 相对 1.34(对端拓扑全景可观测性、物理路由缓存透视、轻量级对端与网络检索及命令行同构)**

- **动机(对端拓扑与物理路由黑盒、离线信任对端不可见与轻量级检索缺失)**:
  在 v1.0 至 v1.34 架构中，节点虽然在内存 `Shared.live` 维持在线会话，在 `Shared.trust` (`trusted.json`) 维护配对设备信任库，在 `routes.json` 维护物理寻址缓存，但在生产使用与多设备组网中暴露出以下三类严重痛点：
  1. **离线受信任对端完全“隐形”**：此前无论是在 Web UI 还是 `GET /api/state` 导出的快照中，仅包含当前在线建立 TCP 会话的 `live` 列表。一旦受信任对端（如手机或离线电脑）关机或断网，用户便无法查看已信任设备列表，不仅无法确认自身信任圈的资产全貌，更导致离线解绑 (`unpair`) 无法在 UI 操作（只能凭记忆手动输入 device_id）；
  2. **物理寻址路由缓存黑盒化**：`routes.json` 记录了节点与各对端建立会话的最后物理 IP:Port，但系统缺乏针对路由缓存的直接透视接口，无法确认当前物理网络通道是走局域网还是 EasyTier 虚拟网；
  3. **缺少轻量级对端与路由轮询端点**：外部监控、状态探针或脚本若想得知对端连接状态，不得不拉取庞大的全量快照 `GET /api/state`，带来多余的传输与序列化损耗；
  4. **CLI 缺乏独立的对端与路由检索命令**：用户在终端无法通过 `tetherly peers` 或 `tetherly routes` 快速获取对端拓扑与物理路由，只能通过混合式的 `status` 命令查看。

- **架构与协议设计(ADR-028 对端拓扑全景可观测性与物理路由缓存透视)**:
  1. **回环 HTTP API 扩充 (`GET /api/peers` 与 `GET /api/routes`)**:
     - `GET /api/peers`：轻量级返回当前在线对端与全部受信任设备列表：
       ```json
       {
         "ok": true,
         "self_id": "tdev_...",
         "live_count": 1,
         "trusted_count": 2,
         "live": [
           {
             "device_id": "tdev_...",
             "name": "peer_node",
             "platform": "macos",
             "addr": "127.0.0.1:45717",
             "rtt_ms": 15,
             "caps": ["notify", "clip", "file"],
             "state": "connected"
           }
         ],
         "trusted": [
           {
             "device_id": "tdev_...",
             "alias": "peer_node",
             "paired_at": 1700000000,
             "revoked": false,
             "online": true
           }
         ]
       }
       ```
     - `GET /api/routes`：轻量级返回物理寻址路由缓存列表：
       ```json
       {
         "ok": true,
         "count": 1,
         "routes": [
           {
             "device_id": "tdev_...",
             "addr": "192.168.1.100:45717",
             "last_seen_ms": 1700000000000
           }
         ]
       }
       ```
  2. **双 CLI 二进制 (`tetherly` 与 `tetherly-cli`) 命令全量同构**:
     - `tetherly peers [--json] [--trusted] [--ui-port <port>]`：格式化输出在线对端列表，支持 `--trusted` 展现受信任资产全景（标记在线/离线），支持 `--json` 输出结构化数据；
     - `tetherly routes [--json] [--ui-port <port>]`：格式化展示物理路由缓存表；
  3. **Web UI 补全**:
     - 在 `ui/index.html` 的对端卡片区域增设「受信任设备」视图，显示绿色的「在线」与灰色的「离线」徽标，并对离线对端提供直观的「解除配对」交互支持。
  4. **端到端集成测试与 CI 全量验证**:
     - 在 `tests/phase1.rs` 增设 `peers_and_routes_inspection_via_loopback_api`：
       - 验证未配对时 `peers` 与 `routes` 均为空；
       - 验证配对后 `GET /api/peers` 呈现 `live` 与 `trusted` 均包含对端且 `online: true`；
       - 验证配对后 `GET /api/routes` 包含物理寻址记录；
       - 验证对端节点离线后，`GET /api/peers` 中对端从 `live` 移除但在 `trusted` 中保留且 `online: false`；
       - 验证调用 `POST /api/peers/unpair` 后，`trusted` 与 `routes` 对应项同步清除。

**1.34 相对 1.33(远程屏幕与键鼠通道全域控制、定向拨号与无头命令行交互闭环)**

- **动机(无头环境屏幕/KVM控制断层、多对端定向盲区与撤销许可缺失)**:
  在 v1.0 至 v1.33 架构中，系统已在 45720 端口建立了远程桌面串流通道，在 45719 端口建立了远程键鼠控制通道。但在生产应用与工程落地中暴露出以下三类严重断层：
  1. **无头/终端 CLI 交互完全缺失**：用户在纯终端、SSH 会话或无图形桌面环境下，完全无法通过命令行管理远程屏幕与键鼠通道，双端 CLI (`tetherly` 与 `tetherly-cli`) 均缺少 `screen` 与 `input` 子命令群；
  2. **多端在线时的定向拨号死局**：此前 `uihttp.rs` 中的 `POST /api/screen/start` 与 `POST /api/input/start` 在多于一个对端在线时直接硬编码报错 `"multiple peers connected; ... target selection not implemented"`，且不接受任何请求参数，导致在多设备组网场景下屏幕与键鼠通道彻底瘫痪；
  3. **屏幕许可撤销接口未暴露**：核心运行时 `Node::screen_revoke()` 已经实现，但回环 HTTP API 仅提供了 `POST /api/screen/allow`，却未暴露撤销端点（`POST /api/screen/revoke`），Web UI 亦无撤销按钮，一旦误点允许只能强杀进程或等待超时断开；
  4. **缺少轻量级状态轮询 API**：外部状态监控或 CLI 必须请求庞大的 `GET /api/state` 才能知晓当前的屏幕帧率、流状态与输入座位，造成不必要的系统资源浪费。

- **架构与协议设计(ADR-027 远程屏幕与键鼠通道全域控制与定向闭环)**:
  1. **回环 HTTP API 扩充 (`GET /api/screen`、`GET /api/input`、`POST /api/screen/revoke`)**:
     - `GET /api/screen`：轻量级返回 `{ "ok": true, "state": "idle"|"requested"|"allowed"|"streaming", "channel_open": bool, "sent": u64, "received": u64, "dropped": u64, "bytes": u64, "refused": u64, "p95_frame_interval_us": Option<u64> }`；
     - `GET /api/input`：轻量级返回 `{ "ok": true, "channel_open": bool, "seat": "local"|"remote" }`；
     - `POST /api/screen/revoke`：调用 `node.screen_revoke()`，重置流状态回 `Idle`，清空发送通道并返回 `{ "ok": true, "state": "idle" }`。
  2. **定向设备目标拨号与鲁棒通道打开**:
     - 升级 `POST /api/screen/start` 与 `POST /api/input/start`，支持解析 JSON `{ "peer": "tdev_..." }`；
     - 单设备在线时若未传 `peer` 则自动匹配；多设备在线时若未传 `peer` 则诚实返回 `400 multiple peers connected; pass peer (tdev_…)`；若传入目标则校验合法性与在线状态，有效打破此前未实现的盲区。
  3. **双 CLI 二进制 (`tetherly` 与 `tetherly-cli`) 命令全量同构**:
     - `tetherly screen allow [--ui-port <port>]`：授权本机屏幕被观看；
     - `tetherly screen revoke [--ui-port <port>]`：撤销本机屏幕观看许可；
     - `tetherly screen start [--peer <id>] [--ui-port <port>]`：发起屏幕推流（可选定向设备）；
     - `tetherly screen stop [--ui-port <port>]`：停止屏幕推流；
     - `tetherly screen status [--json] [--ui-port <port>]`：展示屏幕通道状态、帧统计与 p95 间隔；
     - `tetherly input start [--peer <id>] [--ui-port <port>]`：建立并激活远程键鼠控制通道；
     - `tetherly input stop [--ui-port <port>]`：停止远程键鼠控制；
     - `tetherly input status [--json] [--ui-port <port>]`：展示键鼠通道状态与光标座位。
  4. **Web UI 补全**:
     - 在 `ui/index.html` 的「远程屏幕（实验）」卡片增设「撤销允许」按钮，对接 `POST /api/screen/revoke`。
  5. **端到端集成测试与 CI 全量验证**:
     - 在 `tests/phase5.rs` 增设 `screen_and_input_lifecycle_and_peer_targeting_via_loopback_api`：
       - 验证 `GET /api/screen` 与 `GET /api/input` 初始状态；
       - 验证 `POST /api/screen/allow` 激活许可与 `POST /api/screen/revoke` 成功撤销；
       - 验证未获许可时 `POST /api/screen/start` 被宿主拒绝；
       - 验证通过 `POST /api/screen/start`（带 `--peer` 参数）跨节点成功启动屏幕串流与帧递增；
       - 验证通过 `POST /api/input/start` 建立键鼠通道并成功转移光标座位，断开后安全归还。

**1.33 相对 1.32(通知候选生命周期全域治理、OTP 命令行安全提取与双向快速回复闭环)**

- **动机(终端无头环境通知交互断层与 OTP 安全消费原语缺失)**:
  在 v1.0 至 v1.32 架构中，系统通过 `NotifyHub` 实现了短信/通知接收、OTP 正则提取与 120s 暂态候选存储。但在无头或终端环境下存在明显断层：
  1. **CLI 缺乏通知与验证码交互原语**：用户在服务器、SSH 远程终端或纯 CLI 环境下，完全无法查询当前待处理通知候选 (`candidates`)，无法在终端提取或管道传递验证码 (`tetherly otp <id> | pbcopy`)，无法手动忽略失效通知 (`dismiss`)，亦无法向对端手机快捷回复短信 (`reply`)；
  2. **缺少轻量级轮询接口**：原先查询候选只能调用庞大的 `GET /api/state` 获取全量快照，对高频监控脚本造成不必要的序列化开销；
  3. **OTP 提取缺少纯净返回通道**：原有的 `POST /api/copy` 强制将验证码写入本机剪贴板，无法以独立纯文本方式返回给终端管道或自动化程序；
  4. **严格遵循安全红线**：验证码与短信正文严禁在非 127.0.0.1 泄露，严禁打印至守护进程日志（ADR-010）。

- **架构与协议设计(ADR-026 通知候选生命周期全域治理与双向回复闭环)**:
  1. **回环 API 扩充 (`GET /api/notify/candidates`、`GET /api/notify/replies` 与 `POST /api/otp/get`)**:
     - `GET /api/notify/candidates`：轻量级返回未过期的 `Vec<CandidateViewDto>`，包含 ID、app_id、title、has_otp、created_ms、expires_ms 及 actions；
     - `GET /api/notify/replies`：返回当前待交付至宿主系统的 `Vec<ReplyViewDto>`；
     - `POST /api/otp/get`：接收 `{"id": "..."}`，调用 `node.get_candidate_otp(&d.id)`，仅在 HTTP 响应体中返回 `{"ok": true, "id": "...", "otp": "..."}`，**守护进程日志严禁打印**；
     - 升级 `POST /api/reply/take`：在一次性消费回复时返回 `{"ok": true, "uid": "...", "app_id": "...", "text": "..."}`，提供单次安全的回复内容消费保障。
  2. **节点运行时 OTP 提取原语 (`Node::get_candidate_otp`)**:
     - 在 `crates/tetherly-node/src/runtime.rs` 实现 `get_candidate_otp(&self, id: &str) -> Result<String, CoreError>`；
     - 安全获取 `Shared.hubs` 锁并调用 `hubs.notify.copy_otp(id, &*clock)`，精准处理过期 (`CandidateExpired`) 与无验证码 (`NoOtp`) 错误。
  3. **双 CLI 二进制 (`tetherly` 与 `tetherly-cli`) 命令全量对齐**:
     - `candidates [--json]`：美化表格或 JSON 输出当前活跃通知候选与剩余有效期；
     - `otp <id> [--copy]`：直接打印验证码，支持 `--copy` 同时写入系统剪贴板；
     - `dismiss <id>`：显式注销并丢弃指定通知候选；
     - `reply <id> <text>`：针对指定候选向对端设备发送快捷回复；
     - `replies [--json]`：查看待处理入站快速回复队列；
     - `take-reply <id>`：单次消费并打印入站回复文本（二次消费返回 400）。
  4. **端到端集成测试与 CI 全量验证**:
     - 在 `tests/phase1.rs` 增加 `notify_candidate_lifecycle_and_quick_reply_via_loopback_api`：
       - 验证通知推送后候选自动生成与 `has_otp: true` 识别；
       - 验证 `POST /api/otp/get` 提取与 `POST /api/copy` 写入剪贴板；
       - 验证通过 `POST /api/reply` 发送回复并在对端安全入队；
       - 验证 `POST /api/reply/take` 成功提取并遵循单次消费不变量；
       - 验证 `POST /api/dismiss` 销毁候选与非法参数 400 保护。

**1.32 相对 1.31(跨端安全剪贴板主动同步、回环控制与全端闭环)**

- **动机(剪贴板生产控制入口缺失与平台互通断层修复)**:
  在此前版本中，剪贴板（`ClipHub` 与 `ClipSet`）的核心编解码与去重逻辑已在 `tetherly-core` 建立，但生产链路面临关键痛点：
  1. **生产控制与交互入口缺失**：回环 HTTP API（45716）完全没有剪贴板路由，Web UI 与 CLI 终端无法发起剪贴板主动同步，亦无法将指定文本推送给对端；
  2. **平台过滤人为隔断移动端互通**：原 `Node::send_clipboard` 内部通过 `filter(|p| is_desktop_platform(&p.platform))` 仅向桌面平台广播，导致具有 `clip` 能力的 Android 设备无法接收来自桌面的剪贴板更新，与 §7.5 全平台信任圈初衷冲突；
  3. **缺乏显式广播与序号步进机制**：原 `prepare_local` 在相同哈希时严格返回 `None`（用于后台高频轮询防重复推送），但当用户显式点击「发送」或终端重发文本时，缺乏强行广播并递增 `clip_seq` 的原语；
  4. **必须严守脱敏红线**：依据安全红线「公开 issue / 日志禁止贴短信正文、一次性码、配对码、剪贴板」，剪贴板文本必须严格限制在 127.0.0.1 回环内流转，任何守护进程日志与指标接口均严禁打印或泄露明文剪贴板内容。

- **架构与协议设计(ADR-025 跨端安全剪贴板主动同步与回环控制全闭环)**:
  1. **核心状态机显式广播原语 (`ClipHub::force_prepare_local`)**:
     - 在 `crates/tetherly-core/src/clip.rs` 增加 `pub fn force_prepare_local(&mut self, text: &str) -> Result<ClipSet, CoreError>`；
     - 显式递增 `last_seq`，记录 `last_hash`，强制生成新 `ClipSet`，同时保留 `prepare_local` 的静默防回环能力。
  2. **节点运行时剪贴板控制原语全链路打通**:
     - 在 `crates/tetherly-node/src/runtime.rs` 实现 `send_clipboard_to(&self, target_peer: Option<&DeviceId>) -> Result<usize, CoreError>`、`send_clipboard_text(&self, text: &str, target_peer: Option<&DeviceId>) -> Result<usize, CoreError>` 与 `get_clipboard_text(&self) -> Result<Option<String>, CoreError>`；
     - 彻底移除 `is_desktop_platform` 过滤，只要对端处于活跃连接表且通过 Noise 认证即可同步；
     - 支持指定 `--peer` 定向投递与全域广播，校验 `CLIP_TEXT_MAX = 1 MiB` 上限。
  3. **回环 API 扩充 (`POST /api/clip/sync` 与 `GET /api/clip`)**:
     - 在 `crates/tetherly-node/src/uihttp.rs` 增加：
       - `POST /api/clip/sync`：支持载荷 `{"text": "...", "peer": "tdev_..."}` 或空载荷读取本地系统剪贴板并广播，响应 `{"ok": true, "bytes": N, "hash": "...", "sent_peers": M}`；
       - `GET /api/clip`：安全读取当前剪贴板状态，响应 `{"ok": true, "has_text": bool, "bytes": N, "hash": "...", "text": "..."}`；
       - 严格 127.0.0.1 绑核与 Host 头校验，**守护进程日志严禁打印剪贴板文本**。
  4. **全端 CLI 与 Web UI 对接**:
     - `tetherly` 与 `tetherly-cli` 引入 `clip` 子命令群：
       - `tetherly clip sync [--peer <device_id>]`：同步本机系统剪贴板至对端；
       - `tetherly clip send [text] [--peer <device_id>]`：发送指定文本或从标准输入管道读取；
       - `tetherly clip get`：获取当前剪贴板内容（支持管道输出）；
     - Web UI（`ui/index.html`）增设「剪贴板同步」卡片：包含「同步本机剪贴板至所有对端」快捷按钮、文本输入框与「发送文本」按钮，并实时展示脱敏状态反馈。
  5. **端到端集成测试与 CI 全量验证**:
     - 在 `tests/phase1.rs` 增加 `clip_sync_and_send_via_live_node_api_reaches_peers`：
       - 验证初始状态查询；
       - 验证设置本地剪贴板后经 `POST /api/clip/sync` 成功投递至对端并落地；
       - 验证相同内容的重复同步被本地防回环机制静默抑制（`sent_peers: 0`）；
       - 验证显式文本广播与本地系统剪贴板同步写入；
       - 验证定向设备投递与非法设备 400 校验；
       - 验证超过 1 MiB（`CLIP_TEXT_MAX`）文本被严格 400 拦截拒绝。

**1.31 相对 1.30(信任关系主动解绑与设备安全生命周期治理)**

- **动机(配对设备永久驻留风险与主动安全解除机制缺失)**:
  在此前版本中，设备经 SPAKE2 验证及 Noise IK 握手成功后，其公钥和设备凭据将持久化保存在 `trusted.json` 中。
  1. **无法撤销配对关系**：系统缺乏主动解绑（Unpair）或撤销信任的控制原语。当用户设备淘汰、遗失、转借或不再需要同步时，无法通过系统命令或控制台移除对端信任，唯一途径是手动停止服务并手动编辑底层 JSON 文件；
  2. **活跃会话未能即时熔断**：即使通过外部手段修改信任文件，正在运行中的守护进程仍维系着 45717 会话通道与 45718/45719/45720 流通道，被移除的设备仍能继续触发通知、传输文件或操控键鼠；
  3. **寻址路由未同步清理**：`routes.json` 中残留的过往寻址信息会导致守护进程后台仍周期性重拨已失信的对端设备。

- **架构与协议设计(ADR-024 信任关系主动解绑与设备生命周期治理)**:
  1. **存储层路由遗忘原语 (`forget_route`)**:
     - 在 `crates/tetherly-node/src/store.rs` 增加 `pub fn forget_route(dir: &Path, device_id: &str) -> Result<(), NodeStoreError>`；
     - 依据 `device_id` 原子过滤并更新 `routes.json`，防止后续自动重连尝试；
     - 增加单元测试 `route_remember_and_forget_roundtrip` 验证路由增删的一致性。
  2. **节点运行时解绑控制原语 (`Node::unpair`)**:
     - 在 `crates/tetherly-node/src/runtime.rs` 实现 `pub async fn unpair(&self, device_id: &DeviceId) -> bool`；
     - **原子清理内存信任库**：获取 `Shared.trust: Mutex<MemoryTrustStore>` 锁，调用 `store.peers.remove(device_id)` 彻底销毁凭据；
     - **原子落盘**：调用 `self.persist_trust()` 将更新后的信任表原子同步至磁盘上的 `trusted.json`；
     - **路由清除**：调用 `forget_route` 移除物理寻址缓存；
     - **即时会话断开**：异步锁定 `Shared.sessions` 并移除对应的 `mpsc::Sender<InnerFrame>`，丢弃发送端以关闭 channel，底层连接监听循环侦测到关闭后立即终止 TCP 链路；
     - **状态驱逐与事件广播**：从 `Shared.live` 移除对端记录，若该对端持有输入座位则重置回本地模式，最后广播 `UiEvent::PeerDown`。
  3. **回环 API 扩充 (`POST /api/peers/unpair`)**:
     - 在 `crates/tetherly-node/src/uihttp.rs` 增加 `POST /api/peers/unpair` 路由；
     - 接收 JSON 载荷 `{"device_id": "tdev_..."}`，校验 RFC 4648 base32 格式合法性（非法格式返回 400 Bad Request）；
     - 响应 `{"ok": true, "device_id": "...", "unpaired": bool}`，幂等支持重复解绑查询。
  4. **全端 CLI 与 Web UI 对接**:
     - `tetherly` 与 `tetherly-cli` 引入 `unpair <device_id>` 子命令；
     - Web UI（`ui/index.html`）在对端卡片上增设 `<button class="bad ghost">解除配对</button>` 按钮，点击经确认后调用解绑 API 并自动刷新对端列表。
  5. **端到端集成测试与 CI 全量验证**:
     - 在 `tests/phase1.rs` 增加端到端测试 `peer_unpair_and_trust_revocation`：
       - 验证初始信任与活跃连接；
       - 验证调用 `POST /api/peers/unpair` 成功解绑；
       - 验证内存 `trusted()` 与磁盘 `trusted.json` 均彻底无对端记录；
       - 验证活跃网络会话被立刻切断且 `live_peers()` 降为 0；
       - 验证幂等解绑返回 `unpaired: false` 与非法设备 ID 校验拒绝。

**1.30 相对 1.29(传输生命周期全域管控、会话级传输度量与有限留存治理)**

- **动机(长期运行守护进程内存膨胀隐患与生命周期管理缺失)**:
  在 v1.27 引入 `ActiveTransfer` 与 v1.29 完善取消控制后，所有入站和出站的文件传输任务（无论处于 `Completed` 还是 `Failed` 终态）均长期滞留在 `Shared.active_transfers: Mutex<HashMap<String, ActiveTransfer>>` 中。
  1. **内存泄漏与快照膨胀**：在长周期运行的节点（如桌面常驻服务、连续同步数百个文件的场景）中，已完成的历史传输记录无限累积，造成堆内存单调增长，且使回环接口 `GET /api/state` 响应体中的 `Snapshot.transfers` 无限膨胀，拖慢前端与 CLI 轮询性能；
  2. **全局传输吞吐度量缺失**：系统缺乏会话生命周期内的累加传输度量（总发送字节、总接收字节、成功计数、失败计数），若直接清空传输列表，用户将丢失长期传输统计；
  3. **历史记录缺乏原子安全清理机制**：前端与终端缺少清除已完成任务的入口，且清理必须在原子锁保护下严格过滤终态项，决不能误删正在网络中分块传输（`Transferring`）或 SHA-256 校验中（`Verifying`）的在途任务。

- **架构与协议设计(ADR-023 传输生命周期全域管控与会话级度量)**:
  1. **终态有限留存治理 (`MAX_TERMINAL_TRANSFERS = 64`)**:
     - 在 `tetherly-node::runtime` 定义上限常量 `pub const MAX_TERMINAL_TRANSFERS: usize = 64`；
     - 实现 `prune_terminal_transfers_locked(transfers: &mut HashMap<String, ActiveTransfer>)`：仅当终态记录（`Completed` 或 `Failed`）数量超过 64 条时，提取对应的时间戳（`updated_ms`）按升序排列，逐一淘汰最久远的终态项；
     - **强隔离不变量**：处于 `Transferring` 或 `Verifying` 的在途活跃传输绝不被标记为候选淘汰项，无论累积多少条终态记录，在途传输永远免于被淘汰。
  2. **会话级持久化传输度量 (`TransferStats`)**:
     - 引入结构体 `TransferStats { pub bytes_sent: u64, pub bytes_received: u64, pub completed_count: u64, pub failed_count: u64 }`；
     - 挂载于 `Shared.transfer_stats: Mutex<TransferStats>`，与易失的传输卡片彻底解耦；
     - 在传输状态流转（`set_transfer_phase`）跃迁至 `Completed` 时，依据传输方向分别累加 `bytes_sent` 或 `bytes_received` 并递增 `completed_count`；
     - 在传输跃迁至 `Failed` 或调用 `cancel_transfer` 时原子递增 `failed_count`，设置防重计数守卫（仅在非终态跃迁时累加一次）；
     - 即使清空或淘汰历史卡片，节点运行期间的全局度量数据依然准确持久存在。
  3. **原子安全历史清理控制原语 (`clear_terminal_transfers`)**:
     - `Node::clear_terminal_transfers(&self) -> usize`：在单次获取 `active_transfers` 锁的过程中，原子收集所有处于 `Completed` 或 `Failed` 的键并将其移除，返回清理数量；
     - 正在进行的传输任务不受任何影响，避免并发网络流读写异常或状态混乱。
  4. **回环 API 扩充 (`POST /api/file/clear`)**:
     - 在 `crates/tetherly-node/src/uihttp.rs` 增加 `POST /api/file/clear` 路由，执行 `clear_terminal_transfers` 并响应 `{"ok": true, "cleared": N}`；
     - 将 `transfer_stats` 聚合度量挂入 `GET /api/state` 的 `Snapshot` 模型中向客户端持续广播。
  5. **CLI 与 Web UI 全端呈现**:
     - `tetherly` 与 `tetherly-cli` 引入 `clear-transfers` 子命令，一键清理已完成历史；
     - `tetherly status` 命令在概览中呈现会话累计度量：`Transfer Metrics: sent: ..., received: ..., completed: ..., failed: ...`；
     - Web UI（`ui/index.html`）在卡片栏添加快捷操作按钮 `<button id="cleartransfers">清除完成记录</button>`，并在传输区顶部渲染累计吞吐与成功/失败总计状态条。
  6. **端到端集成测试与 CI 全量验证**:
     - 单元测试：`bounded_retention_prunes_oldest_terminal_transfers` 验证自动淘汰逻辑与在途保护；
     - 集成测试：`transfer_lifecycle_governance_and_stats` 验证端到端成功/失败传输度量累加、API 清理终态记录以及在途传输不受干扰的不变量。

**1.29 相对 1.28(全双工传输任务取消机制、控制面非阻塞解耦与回环控制全闭环)**

- **动机(超大文件长时间占线风险与控制会话阻塞消除)**:
  在 v1.28 升级至 64 GiB 传输上限后，超大文件传输在常规网络下可持续数十分钟甚至数小时。若用户误选文件或网络环境突变，系统缺乏主动取消控制能力，用户只能强杀守护进程，导致破坏已建立的设备信任与会话连接；同时，原实现中发送端在 `on_peer_accepted_file` 中直接在 45717 控制会话的 `handle_event` 主循环中同步执行 `send_stream_from_with_progress`，导致发送大文件期间 45717 控制通道无法读取对端心跳、无法转发通知或同步剪贴板，造成控制面被大文件占线饥饿（Head-of-Line Blocking）。

- **架构与协议设计(ADR-022 全双工传输取消与控制面解耦)**:
  1. **控制通道非阻塞解耦 (`tokio::spawn`)**:
     - 将文件发送任务从 45717 轮询循环中解耦至独立的后台 `tokio::spawn` 异步任务，使得控制通道的 `recv_event()`、心跳 keepalive 与事件响应完全不受 45718 数据通道传输时长的影响，保持毫秒级响应。
  2. **精确任务取消控制原语 (`transfer_cancels` 与 `cancel_transfer`)**:
     - 在节点运行时 `Shared` 注册 `transfer_cancels: Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>`；
     - 接收端（`accept_file`）与发送端（`on_peer_accepted_file`）分别注册 oneshot 取消信道，在传输循环中通过 `tokio::select!` 监听取消信号；
     - `Node::cancel_transfer(transfer_id)`：
       - 原子拔除 `pending_out`（防止并发接受竞态）；
       - 标记 `phase = TransferPhase::Failed`，设置 `error = "cancelled by user"`，将瞬态 `speed_bps` 复位为 0；
       - 触发 oneshot 发送端，通过 RAII 即时关闭底层 TCP 传输流并回收文件描述符；
       - 接收端断开时自动保留断点续传 marker（`.part` 字节对齐）。
  3. **回环 API 与控制闭环**:
     - 回环控制端（45716）增加 `POST /api/file/cancel` 路由，仅接受 `{"id": "tfl_..."}` 参数；
     - 非活跃传输调用返回 400，活跃传输返回 `{"ok": true, "transfer_id": ...}`。
  4. **CLI 与 Web UI 操作联动**:
     - `tetherly` 与 `tetherly-cli` 引入 `cancel <id>` 子命令；
     - `tetherly-cli` 补齐 `notify` 子命令，与守护进程能力完全对齐；
     - Web UI（`ui/index.html`）在处于 `transferring` 或 `verifying` 阶段的活跃传输卡片右上角增加动态 `[取消]` 按钮，点击即刻调用 `/api/file/cancel`。
  5. **测试与 CI 全量验证**:
     - 新增集成测试 `in_flight_streaming_transfer_can_be_cancelled_via_loopback_api`：验证回环取消 API 触发、`speed_bps` 复位、阶段置为 `Failed`、重复取消 400 幂等保护、控制会话持续健康（后续通知秒级送达）以及后续新文件传输 100% 成功。

**1.28 相对 1.27(64 GiB 企业级文件传输上限、EWMA 速率平滑、CLI 状态监控与 Web UI 速率/ETA 全景可观测性)**

- **动机(突破历史内存束缚，打通全链路速度与控制面动态交互)**:
  在 v1.26 引入流式传输引擎与 v1.27 引入活跃传输状态机后，传输体系在性能与结构上已高度成熟，但生产落地依然存在核心诉求：
  1. **历史上限解绑**：原 `FILE_MAX = 512 MiB` 诞生于全量 `Vec<u8>` 内存驻留时代。在拥有 $O(1)$ 常数界后，该限制成为阻碍企业级超大文件（如 OS 磁盘镜像、RAW 视频素材）传输的无谓枷锁，需彻底提升至 64 GiB；
  2. **传输速率感知**：网络波动、磁盘 I/O 抖动使得用户无法准确获知当前的吞吐表现，需要引入经平滑处理的实时速率计量；
  3. **终端交互体验**：CLI 缺少对节点全局状态和对端连接情况的查询能力（`status`），且 `tetherly send` 发送大文件时缺乏如 `rsync`/`curl` 般流畅的终端行内进度刷新；
  4. **Web 界面动态预估**：Web UI 虽有进度条，但缺乏人性化的单位换算（KiB/MiB/GiB/TiB）、速率展示及剩余时间（ETA）预测。

- **架构与协议设计(ADR-021 企业级文件上限与全景可观测性)**:
  1. **64 GiB 传输上限拓展 (`FILE_MAX`)**:
     - `tetherly-core::filexfer::FILE_MAX` 正式升级为 `64 * 1024 * 1024 * 1024`（64 GiB）；
     - `FileHub::offer_resumable`（报价解析门）、45718 接收头校验（`recv_chunks_after_header`）以及发送端入口（`offer_and_send_file`/`offer_and_send_bytes` 与 `POST /api/file/send`）三处严格保持一致，确保在分配 0 额外内存前拒绝超限请求；
     - 清除各处 32 位转 64 位平台溢出隐患，直接断言精确到字节的 64 GiB 边界拒收。
  2. **EWMA 传输速率平滑计算 (`speed_bps`)**:
     - `ActiveTransfer` 结构体新增 `pub speed_bps: u64` 字段，并在 5 处状态机创建点规范初始化为 0；
     - 在 `update_transfer_progress` 触发时，计算 `delta_bytes` 与 `delta_ms`。采用经典指数加权移动平均算法（70% 历史权重 + 30% 瞬态权重）消除瞬态网络波动带来的速率跳跃；
     - 当传输跃迁至终态（`Completed` 或 `Failed`）时，实时速率显式清零。
  3. **控制面异步回环与 CLI 状态监控 (`tetherly status`)**:
     - 在 `tetherly-node::uihttp` 实现异步 `loopback_get(port, path) -> io::Result<(u16, Vec<u8>)>`；
     - 在 `tetherly` 和 `tetherly-cli` 引入 `status` 子命令，提供清晰的文本概览（设备 ID、节点名称、PIN 活跃状态、LAN 监听与已连接对端列表），支持 `--watch`（配合 ANSI 清屏代码实时轮询）、`--interval` 与 `--json` 输出。
  4. **终端发送交互式进度跟随 (`tetherly send --watch`)**:
     - 为 `tetherly send` 扩充 `--watch` 参数。发起发送后，客户端持续轮询回环状态机，在终端使用单行回车符（`\r`）实时输出：
       `[ 45.2%] 1.20 GiB / 2.65 GiB (48.50 MiB/s) [transferring]`；
     - 传输完成输出绿标确认并退出，失败即刻报错退出。
  5. **Web UI 人性化渲染 (`ui/index.html`)**:
     - 落地 `formatBytes`（自动适配 B、KiB、MiB、GiB、TiB）、`formatSpeed` 与 `formatEta` 辅助函数；
     - 卡片内实时动态展示当前吞吐速率（如 `52.40 MiB/s`）与动态剩余倒计时（如 `剩余 1m25s`）。

**1.27 相对 1.26(大文件传输实时追踪、双向进度观测与 Web UI 可视化进度条全闭环)**

- **动机(消除传输黑盒，实现传输过程全透明与双向可观测性)**:
  在 v1.26 引入流式大文件传输引擎后，系统成功消除了内存暴涨并达成了 $O(1)$ 内存常数界。然而，流式传输在运行时对上层系统呈现为「黑盒」：
  1. 发送端在网络通道推送大文件（耗时数秒至数分钟）时，UI 与 CLI 无法感知当前已发送字节数与传输速率；
  2. 接收端在分块落盘 `.part` 阶段，缺少向回环控制面汇报当前进度的统一状态机；
  3. 最终落盘校验（`IncrementalSha256` 匹配）与原子重命名阶段缺乏明确的阶段区分；
  必须建立一套从底层网络通道、节点运行时状态机、回环 API 到前端界面的端到端可观测性机制。

- **架构与协议设计(ADR-020 实时传输追踪)**:
  1. **数据通道分块进度回调 (`send_stream_from_with_progress`)**:
     - 在 `tetherly-net::filechan` 扩充发送流原语，接收 `FnMut(u64, usize) -> Result<(), NetError>` 闭包，每次 64 KiB 加密块写出并刷新到 Socket 后精确触发，汇报 `(chan_offset, chunk_len)`。
  2. **节点运行时状态机 (`ActiveTransfer`)**:
     - 在 `tetherly-node::runtime` 定义传输方向 `TransferDirection`（`Inbound`, `Outbound`）与生命周期阶段 `TransferPhase`（`Transferring`, `Verifying`, `Completed`, `Failed`）。
     - `Shared` 注册 `active_transfers: Mutex<HashMap<String, ActiveTransfer>>`。在发起发送、对端接受、数据写入、增量校验与落盘完成各个阶段精准流转状态，支持按开始时间倒序输出。
  3. **回环 UI 控制平面集成 (`Snapshot.transfers`)**:
     - 在 `tetherly-node::uihttp` 中将节点活跃传输列表暴露至 `GET /api/state` 的 `Snapshot.transfers` 字段。
  4. **Web UI 实时进度条与卡片渲染 (`ui/index.html`)**:
     - 增加现代风格的进度条容器与动态颜色填充，展示传输方向（上传/下载）、文件名、已传字节/总字节、实时百分比以及当前阶段徽章。

**1.26 相对 1.25(流式大文件传输引擎全面落地: $O(1)$ 内存常数界、零整文件驻留、增量 SHA-256 与跨平台生产闭环)**

- **动机(突破全量内存驻留对大文件传输的物理束缚)**:
  在 v1.24/v1.25 中，虽然将 `FILE_MAX = 512 MiB` 确立为内存保护边界并打通了生产发送入口，但底层架构依旧采用「整文件缓冲」模式：
  1. 发送端：`offer_and_send_bytes` 会在 `pending_out` 中持久化整份 `Vec<u8>`，同时调用方还持有原始 buffer；
  2. 接收端：`accept_file` 在内存中用 `Vec<u8>` 接收全部解密块，直至全部收齐后方才执行一次性原子刷盘 `atomic_write`。
  当网络传输数百兆乃至数吉字节大文件时，双端内存暴涨、GC/分配器压力激增，直接限制了低配设备（如天翼云手机、老旧轻薄本）的稳定运行。必须重构传输管线为真正的 $O(1)$ 内存流式架构。

- **架构与协议设计(ADR-019 流式数据管线)**:
  1. **纯净增量哈希状态机 (`IncrementalSha256`)**:
     - 在 `tetherly-core::frame` 增加 `IncrementalSha256` 结构体，无 OS 依赖、纯 Rust 实现，内部维护 `sha2::Sha256` 状态。
     - 支持任意粒度的流式数据块 `update(&[u8])`，并支持输出 32 字节原生指纹或 64 位小写十六进制字符串。单元测试证明其与一次性 `hex_sha256` 计算结果 100% 逐位一致。
  2. **流式加密发送器 (`send_stream_from`)**:
     - 在 `tetherly-net::filechan` 引入 `send_stream_from<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>`。
     - 严格按 64 KiB（`CHUNK`）对齐读取与封包，维持 `TFL2` 魔数规范；
     - 每次仅在栈上分配单一 64 KiB 缓冲，使用 `counter_nonce(chan_offset / CHUNK + 1)` 与 `aad_for(transfer_id, chan_offset)` 进行 ChaCha20-Poly1305 密封，直接通过流写入 Socket。
  3. **流式落盘接收器 (`recv_chunks_to_sink`)**:
     - 在 `tetherly-net::filechan` 引入 `recv_chunks_to_sink<R, W, F>`。
     - 边接收密文边进行 AEAD 校验，解密后的明文数据实时喂给 `IncrementalSha256`，并即刻异步写进目标 `W: AsyncWrite + Unpin`（即 `.part` 文件句柄）；
     - 触发每块进度的回调通知 `on_chunk(chan_offset, chunk_len)`，在传输过程中内存上限恒为单个块大小（64 KiB）。
  4. **节点运行时流式集成 (`tetherly-node::runtime`)**:
     - 发送源抽象：`OutboundFile` 抽象为 `OutboundSource::Path(PathBuf)` 与 `OutboundSource::Bytes(Vec<u8>)`。
     - `offer_and_send_file(&self, peer, path, name)`：基于分块迭代器快速增量计算大文件 SHA-256，无需整文件读入内存；
     - `on_peer_accepted_file`：对端同意接收后，直接异步打开文件流（`tokio::fs::File`），按块对齐精确 seek 到 `resume_from` 位置，直接将文件流连通至 TCP 传输管道；
     - `accept_file`：恢复续传时仅需增量读取磁盘已有的有效前缀并注入 `hasher`，随后直接将网络块流追加至 `.part` 临时文件，最后核对增量 SHA-256 签名并原子重命名为最终文件。
  5. **生产 API 对接**:
     - `POST /api/file/send` 识别到 `path` 入参时，直接走 `offer_and_send_file` 流式路径，杜绝了 `std::fs::read` 导致的整文件内存拷贝。

- **新增回归与集成覆盖**:
  - `tetherly-core`: `incremental_sha256_matches_one_shot_hex_sha256`（增量哈希与全量哈希一致性）；
  - `tetherly-net`: `streaming_roundtrip_with_sink_and_incremental_hasher`（多块端到端解密、落盘与摘要校验）；
  - `tetherly-net`: `streaming_resume_from_chunk_boundary`（断点续传块对齐流式校验）；
  - `tests/phase1.rs`: `multi_chunk_file_streamed_via_live_node_api_reaches_inbox`（跨多个 64 KiB 块的大文件通过真实回环 HTTP API 发送、对端接受、落盘与完整性校验）。

**1.25 相对 1.24(全面接手、CI-First 开发与构建体系确立、文件发送生产闭环与流式化蓝图)**

- **动机(全面接手开发与生产缺口攻坚)**:接手后经过全套 290 项测试验证与代码拓扑审查,确定核心传输与安全架构已高度稳固,但在工程实践与生产可用性上面临两个关键断层:
  1. **构建与验证体系必须 CI-First**:Tetherly 涉及 Windows(WinRT GATT/DXGI/SendInput)、Linux(bullseye glibc 2.31/X11)、macOS(CoreGraphics)以及 Android(NDK arm64-v8a + Gradle JNI)等多端复杂编译链。若依赖本地环境多端拼装,极易重蹈旧 MinGW 32/64 位混淆或 glibc 符号过高的覆辙。遵循用户指示:「构建什么的能用 github ci 的就用 ci」,正式将 GitHub Actions 确立为构建、多平台矩阵回归与安全审计的第一责任面。
  2. **生产文件「只收不发」缺口(v1.24 诚实报告的待办)**:自 v1.12 引入断点续传以来,文件发送功能 `offer_and_send_bytes` 仅在集成测试 `tests/phase1.rs` 中被调用;Node 服务的生产 Web UI 只有接收确认交互,CLI 亦缺少 `tetherly send` 指令。生产节点处于「接收链路健全、发送入口缺失」的半闭环状态。
  3. **大文件流式化架构演进**:当前 `FILE_MAX = 512 MiB` 是受制于两端整文件 `Vec<u8>` 内存驻留的妥协上限。必须设计基于 64 KiB 块对齐的边读/边收/边写与增量 SHA-256 流式协议,将内存占用降至 $O(1)$,为解除 512 MiB 上限铺平道路。
- **CI-First 开发与交付规约**:
  - **多平台矩阵集成 (`ci.yml`)**:每次功能提交与 PR 必须由 GitHub Actions 在 `ubuntu-latest`、`windows-latest`、`macos-latest` 上原生执行 `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo test --workspace --locked`;同步触发 `android` 容器化任务跑 `cargo ndk` 编译 `libtetherly_android.so` 并跑通 Gradle 单测与打包;通过 `cargo-deny-action` 执行许可、漏洞（advisories/licenses/bans/sources）全量守卫。
  - **Linux 发版安全容器 (`release.yml`)**:发布构建严格在 `debian:bullseye`(glibc 2.31)容器中执行,并以 `objdump` 强校验拒绝 `GLIBC_2.32+` 动态符号,确保二进制向下兼容 Debian 11 / Ubuntu 20.04。发版时自动合并 Windows 与 macOS 产物并推送到 GitHub Releases。
  - **本地开发机准则**:本地开发机只负责代码编写、局部语法检查与单测调优,禁止在本地维护脆弱的多端打包交叉编译器,发版制品一律以 GitHub Actions 生成并由 `gh release` / `actions/upload-artifact` 托管为准。
- **v1.25 实施方案(生产文件发送入口闭环)**:
  - **回环 HTTP API**:在 `crates/tetherly-node/src/uihttp.rs` 增设 `POST /api/file/send` 路由,接收 JSON `{ "path": "/path/to/file", "peer": "tdev_..." }`。调用方验证文件可读性、存在性与不超过 `FILE_MAX`,解析目标 `peer` 在 `live_peers` 集合内,调用 `node.send_file(path, peer)`。
  - **CLI 客户端命令**:在 `bins/tetherly-cli/src/main.rs` 与 `crates/tetherly-node/src/bin/tetherly.rs` 增设 `tetherly send <file_path> [--peer <device_id>] [--ui-port <port>]` 子命令。作为 127.0.0.1 活节点回环客户端,与 `tetherly pin`/`dial`/`notify` 保持完全一致的 loopback client 模式（ADR-013）。
  - **流式架构蓝图**:规划基于既有 `TFL2`（魔数 + 块对齐起始偏移）协议的零拷贝流式发送引擎（`FileStreamSource`/`FileStreamSink`）,分块读取（64 KiB）+ `ring::digest::Context` 增量 SHA-256 哈希 + 增量落盘 `.part`,彻底解耦内存大小与文件大小。
- **新增回归与验证目标**:
  - `tests/phase1.rs` 增补 `file_sent_via_live_node_api_reaches_the_paired_peer` 端到端集成回归；
  - UI 自动化测试覆盖 `/api/file/send` 成功与超限/离线错误码处理；
  - 维持 290+ 全绿与零破坏性变更。

**1.24 相对 1.23(文件单文件上限成为真门;网卡分类可解释且单一真理源)**

- **动机(补齐 §9.4 只缺的一段)**:§9.4 明写「Windows:`windows` crate GATT」,但仓库里只有 `tetherly-core::ancs`(协议编解码 + `AncsIngress` 状态机 + `AncsTransport` 端口 + `MemoryAncsTransport`)。真实栈、乃至**入站的接线**都不存在:`NodeConfig.ancs` 在生产路径里恒为 `None`,`tetherly-core` 也刻意不知道「值变了」这件事。于是 ANCS 在真机上**根本无从启动**——端口有了,插头没接。
- **实现**:新增 `crates/tetherly-node/src/ancs.rs`(装配点:`AncsInbound` 四态通道 `Ready`/`NotificationSource`/`DataSource`/`Disconnected`;`open(address) -> Result<Option<AncsLink>, CoreError>`,非 Windows 目标诚实 `Ok(None)`)与 `crates/tetherly-node/src/ancs/win_gatt.rs`(自包含,只 import 绝对路径,照 `screen/macos.rs`、`screen/x11.rs` 先例)。
  - **线程模型**:专属 `tetherly-ancs` 线程持有全部 WinRT 接口并自建 **MTA** 公寓(`CoInitializeEx(COINIT_MULTITHREADED)`;`windows` 0.59 无 `initialize_mta` 封装)。`AncsTransport` 前端只持有一个 `Sender`,故端口**按构造** `Send + Sync`——node 永不直接触碰 COM 接口。
  - **同步端口的落点**:`windows` 0.59 的 async 扩展为 `IAsyncOperation<T>` 提供**阻塞 `get()`**(内部 OS `Waiter` 信号,不空转),因此同步的 `AncsTransport` 三个方法可直接实现,**不需要自造 executor、不需要新依赖**。
  - **建链**:`FromBluetoothAddressAsync` → `GetGattServicesForUuidAsync(ANCS)`(要求 `Status == Success` 且服务非空)→ `GetCharacteristicsAsync` → 按 UUID 找三特征,**并校验属性**(Notification Source / Data Source 必须含 `Notify`,Control Point 必须含 `Write`;缺一即点名报错,不静默降级)。
  - **订阅与写**:`subscribe_*` 写 CCCD 为 `Notify` 并要求 `Success`;`write_control_point` 经 `DataWriter` 打包 `IBuffer` 后 `WriteValueWithResultAsync` 并核对 `GattWriteResult::Status`,**阻塞到结果为真**——这正是「串行、且只有一条在途」成为事实而非假设的原因。前端对每次命令的 ack 等待设 **10s 上界**,超时/线程消失一律报 `AncsSubscribe`/`AncsWrite`,与 v1.21 的 `WriteStalled` 同一态度(不谎报成功)。
  - **入站**:两个特征的 `ValueChanged` 回调**只把字节拷到无界通道**(§9.4 禁止在回调线程写 Control Point);`ConnectionStatusChanged` 置原子标志,驱动线程据此收尾并发 `Disconnected`。`Session::drop` 显式 `RemoveValueChanged`/`RemoveConnectionStatusChanged`,不留注册悬挂。
  - **重连**:建链失败或断开后按 `tetherly_core::ancs::ancs_backoff_delay(attempt)` 指数退避(1s 起、30s 封顶),成功即清零。
  - **node 侧接线**:`Node::spawn_ancs_pump(inbound)` 在**独占线程 + current-thread 运行时**里循环——`Ready` → `ancs_connect()`(由 `AncsIngress::connect` 固定「先 Data Source 后 Notification Source」的顺序);`NotificationSource`/`DataSource` → 对应 feed 点;`Disconnected` → `ancs_disconnected()`;每 500ms `ancs_tick()` 驱动串行 Control Point 队列。CLI `tetherly run --ancs <11:22:33:44:55:66>` 打开链路(先在系统蓝牙里配对 iPhone)。
- **新增回归(全绿,280 → 285)**:`win_gatt` 纯逻辑单测 4 条(Windows 地址记法 `:`/`-`/裸十六进制解析且首字节为高位、6 类畸形输入拒绝、四个 ANCS UUID 的 **LE 解码 ↔ 规范 GUID 逐字段一致**、畸形 UUID 报错——映射错会静默查错服务,故此断言保值);`tests/phase4.rs::m4_5` 端到端 1 条,用 `MemoryAncsTransport` 喂 `AncsInbound` 通道,断言**订阅顺序**(Data Source 先)、**回调只入队而 tick 才写 Control Point**、DataSource 响应成为候选、`Disconnected` 落入 `Backoff`。
- **验证与诚实边界**:`cargo fmt --check` / `clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace --locked`(**285 全绿**)/ `cargo deny`(advisories/bans/licenses/sources)全绿;`Cargo.lock` **零改动**(WinRT 蓝牙特性同属既有 `windows` crate,无新包)。**诚实边界**:WinRT 代码在**原生 Windows x64** 上真编译(比 v1.19 的跨目标 type-check 更强),但 `FromBluetoothAddressAsync`/配对/真机通知的**运行时行为需物理 iPhone**,故仍是 **Manual-required**;macOS(CoreBluetooth)与 Linux(BlueZ)栈**未实现**,`open()` 诚实返回 `Ok(None)` 而非假装可用。

**1.24 相对 1.23(文件单文件上限成为真门;网卡分类可解释且单一真理源)**

- **动机(Tetherly 自查 + 参考项目对照)**:对照 Lancat(README 里「可设单文件大小上限」)回看本仓库的文件接收路径时,发现 `crates/tetherly-node/src/runtime.rs` 那句 `if total > size.max(32 * 1024 * 1024)` 是**死代码**——紧邻上一行的 `if total != size { .. }` 已经把两者钉成相等,于是 `total > size.max(k)` 恒等于 `size > max(size, k)`,永远为假。也就是说「拒绝荒谬的声明大小」这道**读起来存在的保护从未生效**。同族的第二个缺口:§14「默认 LAN 监听」只有 IP 段判据,既无法区分真网卡与第三方 VPN TUN,也不给任何解释——用户对着一个 `100.64.x.x` 的 Tailscale 地址只会看到「什么都没发现」。
- **修复(1):`FILE_MAX` 成为真门,并说明它为什么是内存边界**。v1 的整条文件路径是**整文件驻留内存**的:接收端把正文累积进 `Vec<u8>` 再 `atomic_write`,发送端 `pending_out` 还要在调用方缓冲之上**再存一份**(`data.to_vec()`)。因此上限的意义是**内存边界**而非磁盘策略:`tetherly-core::FILE_MAX = 512 MiB`。
  - **报价即拒(最外层)**:`FileHub::offer_resumable` 在解析时按**声明大小**拒绝 `size > FILE_MAX`,返回 `CoreError::FileTooLarge(FILE_MAX)`——**零字节分配,用户从未看到候选**。
  - **通道再钉一次**:`accept_file` 把 `FILE_MAX` 作为真实 `max_size` 交给 `recv_chunks_after_header`(此前传 `u64::MAX`,使通道自带的 `total > max_size` 检查形同虚设),并在本地再断言一次(通道头是**未认证**的,不可单独信任)。
  - **发送端同向**:`offer_and_send_bytes` 拒绝超过上限的 `data`,不向对端报一个对方必须拒绝的价。
  - **拒绝 ≠ 断链**:`SessionEvent::FileOffer` 分支此前用 `?`,任何 `CoreError`(不安全文件名、坏摘要、超限、候选表满)都会冒泡**拆掉整个会话**;现改为记录即丢(warn + 丢弃该报价),会话继续。
- **修复(2):网卡分类单一真理源 + 可解释**。`tetherly-core::bind` 新增 `BindClass`(`Listen`/`LoopbackAlias`/`Unspecified`/`Broadcast`/`Multicast`/`Overlay`/`CgnatOverlay`/`Ipv6`/`NotLan`)与 `classify`/`classify_v4`/`is_cgnat`;`should_listen`/`should_listen_v4` 改为**就是** `classify(..).listens()`,故「诊断」与「实际绑定」在结构上**不可能分叉**。`tetherly-node::lan` 新增 `IfaceReport` + `interface_report()`(每次 `/api/state` 快照里的 `lan` 字段)与 `log_skipped_interfaces()`(启动时每个被跳过的地址各一行 `reason`)。要点:CGNAT 段 `100.64.0.0/10` 不再与「普通公网地址」混为一谈,而是 `cgnat-overlay-needs-explicit-cidr`——这正是教用户去传 `--overlay-cidr 100.64.0.0/10` 的那句话;`127/8` 的非规范别名(如解析器 stub 的 `127.0.0.53`)显式归为 `LoopbackAlias`,`lan_ips` 因此不再需要「跳过 loopback 接口」这条隐式规则,规则只剩**一处**。
- **新增回归(全绿,285 → 290)**:`tetherly-core/src/bind.rs`(1 条:`classify` 逐类原因 + 「只有 `Listen` 会 listen、非 `Listen` 必给 reason」的不变量);`tetherly-core/src/filexfer.rs`(1 条:超限报价得 `FileTooLarge` 且**不入表**,恰好等于上限则接受);`crates/tetherly-net/src/filechan.rs`(1 条:通道头声明 1025 而上限 1024 → `TooLarge`,且**未缓冲任何字节**);`tetherly-node/src/lan.rs`(1 条:报告与 `lan_ips()` 必须点名**同一批地址**);`tests/phase1.rs::an_oversized_offer_is_refused_without_dropping_the_session`(端到端:伪造 `size = FILE_MAX + 1` 的报价 → 对端候选**保持为空**、会话未断,**随后一笔正常传输仍然成功**)。
- **验证与诚实边界**:`cargo fmt --check` / `clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace --locked`(**290 全绿**)/ `cargo deny`(advisories/bans/licenses/sources)全绿;`Cargo.lock` **零改动**(未新增依赖)。**诚实边界**:`FILE_MAX` 是「当前整文件驻留内存」这一实现下的边界,**不是产品策略**——真正的流式(接收端边收边写 + 增量 sha256、发送端分块读)是后续项,做完即可抬高该值;网卡分类仍是 **IP 段 + CGNAT 判据**,**并未**采用 Lancat 那套 OS 标记分类(Windows `GetIfTable2`/Linux sysfs),故「一个带 RFC1918 地址的第三方 TUN 会被当作 LAN」这一点**尚在**;v1 的跨网仍只走用户自装的 EasyTier 侧车(§2.2),CGNAT fabric 需用户显式 `--overlay-cidr`。

**1.22 相对 1.21(修正内层帧预算:分片屏幕帧此前无法加密)**

- **动机(为 v1.21 缺陷补回归时暴露)**:编写「写停滞」回归时用满额 payload 的内部帧,竟先撞上 `Crypto(Noise("input error"))`。顺藤摸瓜发现这不是测试的问题,而是**生产路径的潜伏缺陷**:任何**需要分片的屏幕帧**都发不出去。zstd 把真实帧压得远小于 64 KiB,故已跑的端到端链路从未触及,属「成功但不可证」的同类风险(v1.18 `pin` 缺陷族)。
- **根因**:`INNER_PAYLOAD_MAX` 原为写死的 `64 * 1024 = 65536`,**未计入** `InnerFrame::encode()` 前置的 **12 字节内层帧头**与 **16 字节 AEAD tag**——而这两者都在 Noise 单条消息 **65535 字节**上限之内。实测(见下 `limits` 回归)可加密明文上限恰为 **`65535 − 16 = 65519`**,故内层 payload 其实最多只能到 `65519 − 12 = 65507`,原常量**多算了 29 字节**。屏幕分片公式 `SCREEN_FRAG_CHUNK_MAX = INNER_PAYLOAD_MAX − SCREEN_FRAG_HEADER` 因此产出 `24 + 65512 = 65536` 字节的分片 payload,编码后 **65548 字节明文**,传输层直接拒加密。同一根因也波及 `to_frame()` 的单帧路径(payload 允许到 65536,同样加不了密)。**文件通道不受影响**:它走第二条 TCP 上自己的 ChaCha20-Poly1305 + 长度前缀(`aead_seal`),不经 Noise transport。
- **修复(以传输上限推导,而非写死)**:`crates/tetherly-core/src/frame.rs` 新增 `INNER_HEADER = 12` 与 `NOISE_MESSAGE_MAX = 65_535`,`INNER_PAYLOAD_MAX = NOISE_MESSAGE_MAX − 16 − INNER_HEADER`(= **65507**);`encode`/`decode` 改用 `INNER_HEADER`,不再写死 12。于是「帧层不能把传输层加密不了的明文交上去」成为**结构性事实**:分片 payload `24 + (65507 − 24) = 65507` 恰好等于上限,编码后 65519 = 传输上限,恰好可加密。向后兼容:任何版本都**从未**成功发出过 payload ∈ (65507, 65536] 的帧(加密必失败于写之前),故收紧解码上限对线上格式无损。
- **新增回归(全绿,275 → 280)**:`crates/tetherly-crypto/tests/limits.rs`(2 条,真实 IK 握手后量出明文上限 65519 通过/65520 被拒,并断言满额内层帧明文 ≤ 上限且可加密、再多一字节被拒);`crates/tetherly-net/tests/screen_wire.rs`(1 条,4 MiB 1024×1024 BGRA 帧经 `to_frames()` 分片、**真实会话**逐片 `send_bounded`、对端 `recv_inner` + `ScreenAssembler` 重组后逐字节相等);`crates/tetherly-net/tests/backpressure.rs`(2 条,v1.21「写停滞」缺陷的会话级回归:不读的对端在 `SESSION_WRITE_TIMEOUT` 后得 `WriteStalled` 而非永久挂起,持续读的对端绝不误报停滞)。
- **验证与诚实边界**:本机 `cargo fmt --check` / `clippy --workspace --all-targets -D warnings` / `cargo test --workspace --locked`(**280 全绿**)/ `cargo deny`(advisories/bans/licenses/sources)全绿。**诚实边界**:窗口预算的修正为端到端**新回归**所证,但真机满屏帧(未压缩/大分辨率)的 45720 线上实测仍在 Phase 5「真机闭环」的既有 Manual-required 范围内。

**1.21 相对 1.20(会话存活检测与背压;Linux 发版锚点由 glibc 2.35 下移到 2.31)**

- **动机(跨 VPS 实测暴露)**:两台 VPS 配对成功、双向 `path lan` 正常,但**只有第一条 `notify` 到达对端**;此后每次 `notify` 都返回 `notified tdev_...`,对端却毫无日志,双向皆然。两端 `ss` 显示连接仍 `ESTAB`,但 **Send-Q 堆积未确认字节**(一侧 422 B、另一侧 194 B)——路径已被中间设备**静默黑洞化**,而**没有任何一层会发现**。
- **根因**:控制会话任务的 `select!` 里 **`await` 了写操作**,而 `write_tagged` 末尾 `flush()` 在发送缓冲填满、对端零窗口时会**无限期挂起**——整个任务就此卡死(既不再 `recv_event`,又因队列**无界**而继续吞下后续帧);`send_to` 用 `mpsc::unbounded_channel`,只要队列还在就返回 `Ok`。桌面节点**从不主动发 ping**(只在收到时回 pong),故 TCP 的 `ESTABLISHED` 被一路当作「活着」。
- **修复三层**:
  - `tetherly-core::liveness`(**抽离于时钟之外**,无 `now()`/随机源):`SESSION_HEARTBEAT=15s`、`SESSION_LIVENESS_DEADLINE=45s`;`Liveness::is_expired` 判据是「45s 内**无任何入站帧**即判死」,减法饱和故**时钟回拨不会误判**;4 条确定性单测(边界、重装、时钟回拨)。
  - `tetherly-net`:`SESSION_WRITE_TIMEOUT=15s` + `NetError::WriteStalled` + `ActiveSession::send_bounded()`——把写包进超时,把「静默挂起」变成**硬错误**;超时即视为致命(写可能停在中途),调用方据此拆会话。
  - `tetherly-node`:控制/输入/屏幕/屏幕控制四条通道**全部改为有界**(`mpsc::channel`;控制与输入 256、屏幕 32),入队改 `try_send`——满即**诚实报错**(`peer busy; send queue full`)而非静默缓冲,落实背压;控制会话加**心跳**(每 15s 经同一通道发 `ping`,顺带保活 NAT 映射)与存活判据(拆会话 → 发 `PeerDown` → 后续 `notify` 快速真失败);**全部 8 处会话写**改用 `send_bounded`;屏幕帧队列满时**整帧丢弃**(直播语义,绝不排队部分帧——`try_send` 前先看 `capacity()`),避免慢链路下无界增长。
- **效果**:路径死掉后**至多 15s** 会话被拆,`notify` 从「谎报成功」变为**可操作的失败**(`peer gone`/`not connected`);对活跃流则表现为**丢帧而非阻塞**。
- **Linux 发版锚点下移到 glibc 2.31(`.github/workflows/release.yml`)**:v1.20 的 `ubuntu-22.04` 锚点只保证「不高于 2.35」,产物实测要求 **`GLIBC_2.34`**——可达主机都能跑,却**跑不了 glibc 2.31 的老发行版**,与 §14「覆盖 glibc 2.31」的承诺不符。改为在 **`debian:bullseye`(glibc 2.31)容器**内构建:动态符号地板随之 **≤ 2.31**,一个产物覆盖 Debian 11 / Ubuntu 20.04 及更新主机;守卫改为拒绝 `GLIBC_2.32+` 并**打印实际地板**(v0.1.3 实测地板 **`GLIBC_2.30`**)。桌面 build 拆为独立 job(`build-linux` 容器 + `build-desktop` 矩阵),`publish` 依赖两者。两条容器细节是**必须的**:①容器默认 `/bin/sh` 是 dash、拒绝 `set -o pipefail`,故 job 的 `run` shell 固定为 `bash`;②bullseye 已过 LTS,`deb.debian.org` 不再提供其 pool(`apt-get update` 全部 404),故 `sources.list` 指向 **`archive.debian.org`** 并加 `Acquire::Check-Valid-Until=false` 以接受归档的过期 `Release`。
- **验证与诚实边界**:本机 `cargo fmt --check` / `clippy --workspace --all-targets -D warnings` / `cargo test --workspace --locked`(**275 全绿**,=271 + `liveness` 4 条)/ `cargo deny`(advisories/licenses/bans/sources)全绿。CI(`ci.yml`)三平台 + android + deny 五个 job 全绿。跨 VPS 重连后连发两条通知**均被对端入库**(OTP 正文 `has_otp:true`、第二条 `has_otp:false`)。发版(`release.yml`)在 `v0.1.3` tag 上全绿:`build-linux` 容器内 `objdump` 地板为 `GLIBC_2.30`,六个产物(linux/windows/macos 各 tetherly 与 tetherly-cli)已发布。**诚实边界**:容器构建路径只在 CI 验证(本机无 Linux 链接器);「glibc 2.31 真机启动」以 CI 守卫 + 地板打印为证据,仍需在一台真实 glibc 2.31 主机上复验一次。

**1.20 相对 1.19(CLI 控制面全部改为「活节点回环客户端」;新增 `/api/notify`;Linux 发版锚定 glibc 2.35)**

- **动机**:v1.18 只修了 `pin` 这一处「CLI 自起节点」缺陷,但同一类错误在 `dial` 上仍然存在,且**跨节点投递通知根本没有 CLI 路径**。跨 VPS 实测暴露:`tetherly dial` 在守护进程已在跑时执行 `Node::start(cfg)?` 去**再绑一遍**同一批 LAN 端口,直接死于 `no LAN address bound`;而 `tetherly inject-notify` 在**自己的**进程里构造 `NotifyPush` 并 `ingest_local_push`,只把候选喂给本进程——从不经 Noise 会话,**永远到不了对端**。根因与 ADR-013 同出一辙:能接受 PIN、能建立/持有会话、能投递消息的,永远是**持有那份进程内状态的那个进程**。
- **`Cmd::Dial` → 回环 `POST /api/dial`**:与 `pin` 同一形状——附带 `--ui-port`(默认 45716),经 `tetherly_node::loopback_post` 把 `{addr, pin}` 交给**运行中**的节点;`dial` 的 PIN 装置、拨号、`persist_trust`、`remember_route` 全部发生在活节点里,CLI 只读回结果。无守护进程监听时**诚实非零退出**并提示 `tetherly --data-dir <dir> run`。**红线保持**:`/api/dial` 仅在 127.0.0.1 可达(复用既有回环 UI 的 `Host` 校验),非回环来源一律 403。
- **`Cmd::InjectNotify` → `Cmd::Notify` + 新路由 `POST /api/notify`**:`inject-notify` 只在本进程灌假推送、无法投递,予以**删除**并替换为真正的投递命令——`tetherly notify --title … --body … [--app-id …] [--peer tdev_…] [--ui-port …]`,经回环 `POST /api/notify` 由活节点 `send_notify()` **经已配对的 Noise 会话**发出。目标选择**诚实**:恰好一个 live peer 时隐含之;零个直接 400 `no connected peer`;**多个**则要求显式 `--peer`,不静默挑一个。`--peer` 先 `DeviceId::parse` 再校验它在 live 集合内,未知/不在线一律 400。正文与 v1.18 一致:**只投递、不执行、不入日志**;`uid` 取本机时刻、`app_name` 固定 `tetherly`、动作集裁剪为 `copy`(`NotifyPush::truncate` 再兜一次长度与 URL 动作,与入站路径同一道闸)。
- **Linux 发版锚定 glibc 2.35(`.github/workflows/release.yml`)**:发布二进制除 glibc/libgcc 外**全静态**,故 runner 的 glibc 就是它的运行地板。原矩阵用 `ubuntu-latest`(24.04 / glibc 2.39),产物在 glibc 2.31 的主机上**拒绝启动**。改为 `ubuntu-22.04`(glibc 2.35,覆盖 Debian 12 / Ubuntu 22.04 / 当代 RHEL 系),并加一步 `objdump -T … | grep -E 'GLIBC_2\.(3[6-9]|[4-9][0-9])'` 守卫:一旦二进制引用了更新的 glibc **构建即失败**,而不是把不可运行的包发出去。Windows / macOS 行不受影响(CI 测试矩阵不变,仅发布产物变化)。
- **验证与诚实边界**:本机 `cargo fmt --check` / `clippy --workspace --all-targets -D warnings` / `cargo test --workspace --locked`(**271 全绿**,`tests/phase1.rs` 由 9 → 10:新增 `notify_posted_to_the_live_node_api_reaches_the_paired_peer`,断言回环 `/api/notify` 投出的文本**确实抵达对端候选**,且未连接的 `--peer` 被 400 拒绝,使 200 不可能是「无条件收下」)/ `cargo deny`(advisories/licenses/bans/sources)全绿。**诚实边界**:本机 Windows-gnu 工具链的 MinGW.org 6.3.0 是 32 位、其 `ld` 不认 rustc 1.99 发出的 `--high-entropy-va`,故本地链接一度失败——这是**开发机环境问题**,不属项目代码;为取得本地测试证据,本次临时以 rustup 自带的 64 位 self-contained MinGW(gcc 14.2)作链接器、并以 NDK clang 20 补上其缺失的 `as`/C 前端(`zstd-sys`),**均未写入仓库**(`.cargo/config.toml` 保持机器无关,CI 走 MSVC 主机不受影响)。真机 LAN 直连双桌面、8h soak 等仍 Manual-required。

**1.19 相对 1.18(填掉 §9.7 承诺的 macOS/Linux 采集缺口:real macOS CoreGraphics + Linux X11 SHM;两端后端文件自包含、按目标编译验证)**

- **动机**:§9.7 与 §14 一直把「macOS `core-graphics`、Linux X11 SHM(`x11rb`)」标为**下一迭代**;v1.8 只把 Windows 真机链闭合。本次把这两个采集后端按 `cred.rs` 的后端化先例落地——每个后端一个**自包含文件**(只 import 绝对路径),因而可在本机按目标交叉编译验证其编译正确性。
- **macOS `MacSource`(`crates/tetherly-node/src/screen/macos.rs`,新)**:`CGMainDisplayID` → `CGDisplayCreateImage` → 绘入**显式 BGRA8 `CGBitmapContext`**(`PremultipliedFirst | Order32Little`)→ 按框架返回的实际 stride **去行尾填充** → `ScreenFrame::raw`。规范化经过已知格式是刻意的:宽色域 / Retina 屏可能回 16-bit 或 RGBA 采样,远端解码器不该去猜。请求 stride 取 **16 字节对齐**,再读回框架实际 stride 去填充——否则 1365px 这类行宽(5460 字节)会因框架补位而整帧失败。屏幕录制权限被拒时返回 `ScreenRefused`(**不返回空白帧**)。`CGDisplayCreateImage` 被软弃用(建议 ScreenCaptureKit),保留原因:ScreenCaptureKit 需要签名 app bundle + 用户同意弹窗,并会把依赖面扩到 AppKit;弃用警告**只在该 import 一处** `#[allow(deprecated)]`,不 crate 级静默。**呈现**在 macOS 需要 AppKit `NSWindow`(CoreGraphics 位图上下文无法触及屏幕),§9.7 已把它列入下一迭代,故 `present` 返回点名 `NSWindow` 的诚实 `ScreenRefused`,不假装。
- **Linux/BSD X11 `X11Source`(`crates/tetherly-node/src/screen/x11.rs`,新)**:两条传输共用同一解码——**MIT-SHM**(`shm_create_segment` + `shm_get_image`,服务器直接写入我们 `mmap` 的段,整帧不过 socket)为快路径;**plain `GetImage`** 为无 SHM 服务器(远程 X、加固服务器)的回退。SHM 可用性门:**扩展存在 ≠ 允许 fd 传递**,要求 MIT-SHM ≥ 1.2(分段段)。读到 `reply.size` 字节(而非整段),避免上帧残留字节混入本帧尾部。根窗口字节序非 LSB-first、或根深度不在 `{24,32}` 时**诚实拒绝**;depth 24 的未用高字节填 `0xFF`(depth 32 已有真实 alpha,不动)。**呈现**在 X11 可落地,故一并实现:override-redirect 窗口 + `PutImage`,窗口深度经 `COPY_DEPTH_FROM_PARENT` 从根继承,`PutImage` 用**读回的根深度**而非硬编码 24(否则 32 位根上服务器回 `BadMatch`)。
- **接线**:`crates/tetherly-node/src/screen.rs` 按目标 `#[cfg]` 选择 `inner`(`DxgiSource` / `MacSource` / `X11Source`),`SystemSource` 保持单态(无动态分派);`SystemSink::present` 四臂(macOS → 诚实拒绝,`not(any(windows,macos,unix))` → 诚实拒绝)。`Cargo.toml` 新增**目标依赖**:macOS `objc2-core-foundation` + `objc2-core-graphics`(`default-features = false`,避开 objc2-metal/dispatch2 面)、Linux `x11rb`(`shm`)+ `libc`(x11rb 自带的 `libc` 在其 `allow-unsafe-code` 之后,未启用,故显式声明)。**未引入任何新包**:这些 crate 已随 `arboard 3.6.1` 传入 lockfile。
- **验证方式与诚实边界**:两个后端在**本机对各自目标**(`x86_64-apple-darwin` / `x86_64-unknown-linux-gnu`)以最小探针 crate 完成 `clippy -D warnings` **类型检查**(含整个 `screen.rs`,用桩 `zstd` 绕开目标 C 编译器),探针用后即删;后端单元测试(7 + 7 条:stride 对齐、去填充、截断拒绝、depth 处理、非 BGRA 拒绝)是**纯逻辑、不触显示器**,CI 在 `ubuntu-latest` / `macos-latest` 上原生编译并执行。**真机采集需真实显示器/会话**(macOS 需屏幕录制权限,X11 需 X server),仍 Manual-required;本机无 Linux/macOS 链接器,故不做本地执行,只做类型检查 + CI 原生执行。
- **诚实边界**:开放编码器(OpenH264/rav1e)、硬件编码、真机注入、macOS 呈现(NSWindow)、Wayland portal 仍 Manual-required/下一迭代。本次不宣称 macOS/Linux 真机远程桌面已可用,只宣称**后端已实现且按目标编译验证**。

**1.18 相对 1.17(真机准入口:APK 内 Kotlin→JNI→实时会话 UI 全链路;`tetherly pin` 缺陷修复;手机通知 p95 上机实测)**

- **APK 端到端已验(云手机 Galaxy Fold / arm64-v8a / Android 13)**:真机 `app.tetherly` 的前台 `NotificationListenerService` → JNI(`Java_app_tetherly_NativeBridge_*`)→ `libtetherly_android.so` 的实时 Noise 会话整条链路闭合:真机侧推送的测试通知经桌面节点提取为候选 `{app_id:"com.android.shell", has_otp:true, source:"tdev_…"}`(通知**正文与验证码不入快照、不入日志**,只暴露 `has_otp` 布尔与来源设备 id)。§9.6 此前标注的「APK 内 Kotlin→JNI→会话的 UI 全链路仍以真机手动为准」由本次真机实测关闭。
- **M1.1 p95 上机计量(手机→桌面通知往返)**:节点 wall-clock 计量在真机链路上给出 `rtt_min≈17.4ms / p50≈17.9ms / mean≈37.5ms / p95≈56.9ms / p99≈361.8ms / max≈361.8ms`。p95 落在 M1.1 的 **<1.0s 预算内**(约 18× 余量)。**诚实边界**:这是**经 `adb reverse` 隧道的云手机**往返,不是 LAN 直连同机型(后者约 15–16ms);p99 尾尖峰来自云手机 adb 隧道抖动,不当作协议成本。8h soak 仍 Manual-required。
- **`tetherly pin` 缺陷与修复(真机排查暴露)**:CLI 的 `Cmd::Pin` 此前实现为 `Node::start(cfg)?` + `node.generate_pin()?`——它在**另一个一次性进程**里生成 PIN 并打印。而 PIN 只对**持有它的那个进程**有意义(`pin_created_ms` 是纯进程内状态,从不持久化),所以打印出的码**永远无法**被正在运行的守护进程接受;真机表现为「PIN 明明显示出来了,手机却一直 `pin expired`」。**可用路径一直是运行中守护进程的回环 API `POST 127.0.0.1:45716/api/pin`**(UI 的「显示 8 位 PIN」按钮走的就是它)。修复:`Cmd::Pin` 改为携带 `--ui-port`(默认 45716)的**回环客户端**,经新增的 `tetherly_node::loopback_post(port, "/api/pin", b"{}")` 把 PIN 装置到**活着的**节点;没有守护进程监听时**诚实**非零退出并给可操作提示。配套改动:`uihttp::spawn` → `bind_and_spawn`(同步 bind 并返回实际端口,端口被占由静默 `warn!` 变为硬错误)、`Node::spawn_ui()`/`Node::ui_port()`(`Shared.bound_ui: AtomicU16`)、`run()` 返回前先 bind UI 端口,使「节点已起」== 该端口真实可达。
- **回归测试**:`tests/phase1.rs::pin_shown_by_the_live_node_api_is_the_one_that_pairs`——起宿主+对端,`host.spawn_ui()` 取回环端口,`POST /api/pin` 取 PIN 并配对成功;再起第三个只装了硬编码 PIN 的节点,断言 `peer.dial(stale.control_addr()).await.is_err()`,使断言不可能凑巧通过。全量 269 → **270 全绿**。
- **诚实边界**:APK 链路与 p95 为**云手机经 `adb reverse` 隧道**实测,非 LAN 双物理桌面;8h soak、双桌面 100 次物理往返、macOS/Linux 采集与注入、开放编码器(OpenH264/rav1e)仍 Manual-required。本次不宣称真机 LAN 远程桌面或 8h soak 已达标。

**1.17 相对 1.16（把 §13 的属性测试从「单个解码器/单条消息」推进到「有状态状态机 + 任意操作序列」）**

- **动机**：v1.16 的属性测试与变异语料覆盖的是**无状态函数**（解码器、往返编解码）。§13 里真正危险的入站面是**有状态机器**——`ReplayGuard` 的窗口、限流器的令牌/漏桶、`NotifyHub` 的去重与过期、`FileHub` 的接受/落盘门、`ResumeTable` 的偏移领取、屏幕收发与重组、`AncsIngress` 的订阅/退避/持有。这些的危险不在「单条输入能不能解」，而在「**任意顺序的一串输入能否把状态推入违约态**」。v1.17 就补这一层。
- **`proptest`（core，`crates/tetherly-core/tests/state_machines.rs`，11 条）**：以 `program(max_ops)` 策略生成**任意字节操作程序**（`Vec<u8>` 当指令流：取字节选动作、取若干字节做参数），逐条喂给被测机器并**在每一步之后**断言安全契约；`Cur` 游标对越界读**饱和返回**而非 panic，保证程序本身永远不会崩，从而把 panic 全部归因于被测机器。覆盖：
  - `ReplayGuard`：**已被接受过的 id 永不再被接受**；远古 id（高水位之下 `>1024`）恒拒；任意 id 序列下不 panic（衔接 v1.15 的 `saturating_add` 修复）。
  - `TokenBucket`：**冻结时钟下恰好只放行 `floor(burst)` 个**，第 `burst+1` 个必拒（O(1) 摊还，无线性扫描）。
  - `NotifyLimiter`：任意时刻放行数**永不超过漏桶上界** `10 + 2t/1000 + 1`。
  - `ClipLimiter`：**时钟回拨不产生额外额度**（单调性），回拨后的放行数不超过回拨前口径。
  - `FileHub`：任意名字经 `sanitize_file_name` 后**非空、非 `.`/`..`、不含路径分隔符与 NUL**；**永不自接受**（未点击接受前 `is_accepted` 恒假）；洪泛 offer 时**在途表有界**（`INCOMING_MAX = 256`），不再无界增长。
  - `ResumeTable`：`claim` 给出的偏移**恒为 `0 < off < total` 且按 `FILE_CHUNK` 对齐**，并与来源绑定（换源不得蹭偏移）。
  - `NotifyHub`：任意 push/reply/时钟推进序列下去重、回复关联、过期三组不变量恒成立。
  - `ScreenSender`：仅在本机显式同意后 `state().is_streaming()` 才为真——**无本机同意绝不推流**；`ScreenReceiver` 游标单调且 `stats().received` 与实际收到严格一致。
  - `ScreenAssembler`：任意顺序碎片可重组，不完整流被拒，**永不 panic**（衔接 v1.15 的 `checked_mul`/`off >= total` 修复）。
  - `AncsIngress`：任意事件序列不 panic；**未就绪（非 `Ready`）时的回调是惰性的**，绝不让 `PRE_EXISTING` 的 uid 或任何事件泄漏到 `drain_events()`（M4.3）。
- **确定性用例（同文件，7 条，作非空性见证）**：`screen_sender_gate_opens_once_the_local_machine_consents`（证明上面的门控断言**不是空洞真**——同意后确实能进入 streaming）、碎片乱序重组、不完整流拒绝、ANCS 惰性/不暴露既有 uid、冻结时钟去重、`FileHub` 必须先 accept 才能 done、`TcpLimiter` 按 IP 隔离。
- **结果**：**未发现任何新缺陷**——v1.15 的两处修复在状态机层面同样成立，新属性未再击穿。全量 `cargo test` 由 251 升至 **269 全绿**；新测试位于既有 `cargo test --workspace --locked` 步骤，CI 无需新增 job。
- **诚实边界**：覆盖的是**确定性状态机**（时钟与随机源全部注入）；`libfuzzer`/`cargo-fuzz`（需 nightly）、真机采集/注入、双桌面物理往返仍是 Manual-required。本层不冒充 coverage-guided fuzz，也不冒充真机验证。

**1.16 相对 1.15（落地 §13 的 `proptest` 与 `fuzz` 两层：解码器属性测试 + 确定性变异语料）**

- **补上了 §13 声明的两层**：此前 §13 表已列 `proptest`（控制 JSON 往返）与 `fuzz`（内层帧、ANCS tuple），但仓库里**只有单测/向量/集成**三层真实存在。v1.16 把 `proptest` 与 `fuzz` 落成可复现、可在**稳定工具链**与 CI 上跑的测试，不再依赖 nightly。
- **`proptest`（core，`crates/tetherly-core/tests/decoders.rs`，27 条）**：以策略生成任意输入，对**每个入站解码器**断言「永不 panic」，并对**每个可编码消息**断言「往返一致」。never-panic：`InnerFrame::decode`、`ScreenFrame::decode_payload`、`ControlMsg::decode_payload`、`ScreenAssembler::push`、`InputEvent::from_frame`、`NotificationEvent::parse`、`Hello::from_bytes`、`DeviceId::parse`、`parse_pin`、`sanitize_file_name`、JSON 控制面 `from_payload`、`DataAssembler`（附 8 KiB 上界断言）。往返：notify push/dismiss/reply、clip set、file offer/decision/done、caps update、input event、screen frame、control msg、hello。分片：任意顺序重组、不完整流拒绝、载荷上限。
- **`proptest`（net，`crates/tetherly-net/tests/decoders.rs`，7 条）**：`read_len_prefixed`/`read_tagged`/`read_header` 对任意字节**永不 panic**；oversized 长度前缀被拒（`MAX_OUTER` 之上，防对端驱动的分配 DoS）；合法头往返；`TFL1` 旧格式读作 `start=0`；错误 magic 拒绝。`&[u8]` 直接作 `AsyncRead`（`tokio` 实现），`prop_assert!` 不入 `async` 块，一律用 panic 式断言。
- **`fuzz`（core，`crates/tetherly-core/tests/fuzz_decoders.rs`，2 条）**：因只有 stable 工具链（无 libfuzzer/`cargo-fuzz`），以**确定性 xorshift 变异器**驱动一个「每个解码器都有合法语料」的种子集（内层帧、全部 7 类控制 JSON、全部 7 类输入事件、整帧 + 分片屏幕帧、控制 msg、ANCS 通知源事件 + 数据源 tuple、规范 Hello JSON），施加 6 类变异（位翻、极端字节、截断、扩展、多字节覆写、拼接）后喂给**全部解码器**，`catch_unwind` 捕获 panic。`mutating_the_valid_corpus_never_panics_a_decoder`（固定种子 `0x9E3779B97F4A7C15`，每种子 512 次、每 4 次叠加，稳定可复现）+ `adversarial_edge_inputs_are_rejected_cleanly`。
- **结果**：**未发现任何解码器缺陷**——v1.15 的两处修复成立，新属性未再击穿。全量 `cargo test` 由 205 升至 **251 全绿**。新测试位于既有 `cargo test --workspace --locked` 步骤，CI 无需新增 job。
- **诚实边界（Manual-required 顺延）**：**libfuzzer/`cargo-fuzz`（需 nightly）仍是 Manual-required 的后续**；本轮的确定性变异驱动是**稳定工具链下的诚实替代**——覆盖面与可复现性明确写在用例里，不冒充 coverage-guided fuzz。

**1.15 相对 1.14（对端可控输入健壮性加固：重放窗口溢出 + 屏幕碎片越界）**

- **`ReplayGuard` msg_id 溢出**：`check_and_record` 里的 `msg_id + 1024 < self.next` 对**对端可控的 `u64`** 直接相加——`msg_id = u64::MAX` 时在 debug 下 `attempt to add with overflow` panic、在 release（`overflow-checks` 关闭）下回绕为「远古 id 被当成 fresh 接受」，削弱重放防护。改为 `msg_id.saturating_add(1024) < self.next`；语义不变（`recent` 1024 项窗口仍兜底）。补两条回归：`rejects_id_far_behind_the_high_water_mark`、`extreme_peer_ids_do_not_overflow`。
- **`ScreenAssembler::push` 越界下溢**：`off = idx * SCREEN_FRAG_CHUNK_MAX` 后直接 `self.total - off`，未校验 `off < total`——对端用合法 `total`（例如 16）配 `idx = 1` 即令 `total - off` 下溢（debug panic / release 得巨大 `expected` 而错配）。改为 `checked_mul` + `off >= self.total` 直接拒为 `ScreenFragment`，不分配、不 panic。补回归 `assembler_rejects_an_index_past_the_stream_end`。
- **范围**：两处均为**入站防护**缺陷（远端可触发），不改变任何线格式、端口、状态机或对外语义；`cargo test` 由 202 升至 **205 全绿**。

**1.14 相对 1.13(凭据库后端化:macOS Keychain + Linux libsecret)**

- **`cred` 后端化**:`crates/tetherly-node/src/cred.rs` 拆为**按目标平台选择的后端**——`cred/windows.rs`(Credential Manager,已 v1.13)、`cred/macos.rs`(**Keychain 泛型密码**,`security-framework`)、`cred/linux.rs`(**Secret Service / libsecret**,`secret-service`,D-Bus)、`cred/other.rs`(无库目标回退)。每个后端文件**自包含**(只 import 绝对路径),因而可在**本机交叉编译验证**其目标平台的编译正确性;`vault_available()`/`vault_store()`/`vault_delete()`/`load_or_create_identity()` 语义不变,`.ref` 指针与 `identity.bin` 回退对所有平台一致。
- **macOS**:`tetherly.identity`(service)/ `<device_id>`(account)经 `set_generic_password`/`generic_password`/`delete_generic_password`;`errSecItemNotFound` 视作「无条目」而非错误,任何 keychain 失败(无登录 keychain/被锁)一律回退文件,启动不阻塞。
- **Linux/BSD**:`secret-service` **blocking** API,默认 collection 内以属性 `device_id=<id>`、标签 `tetherly.identity` 建项(`replace=true` upsert);无 `org.freedesktop.secrets` 提供者(无 gnome-keyring/KWallet 的 headless)时 `available()==false`,透明回退文件;`default-features=false` + `rt-tokio-crypto-rust`(纯 Rust 密码学,**不引 openssl**)。
- **依赖**:新增 `[target.'cfg(target_os="macos")']`:`security-framework`/`security-framework-sys`;`[target.'cfg(all(unix, not(macos)))']`:`secret-service`。全部 **MIT OR Apache-2.0**(deny 许可检查通过);`unsafe` 仍**仅限** `cred/windows.rs` 的 Win32 FFI,macOS/Linux 后端**无 unsafe**。
- **验证**:本机 `cargo check --target x86_64-apple-darwin` 与 `--target x86_64-unknown-linux-gnu` 对两个后端文件**逐字编译**(经 path-include 探针);探针即实测捕获并修复了 Linux `create_item` 返回 `Item<'_>` 借用于临时值的生命周期错误。**运行时** (真读写 Keychain / Secret Service) 仍 **Manual-required**:CI 的 `macos-latest`/`ubuntu-latest` job 已在真实平台上编译它们,`TETHERLY_VAULT_TEST=1` 可在真机跑一次真库往返。
- **诚实边界**:Windows 已真机验(v1.13);macOS/Linux **仅编译验证 + 平台 CI 编译**,真库运行时读写未在本机验证,标 Manual-required。

**1.13 相对 1.12(Windows 系统凭据库身份存储:Credential Manager + `identity.ref`)**

- **凭据库落地**:新增 `crates/tetherly-node/src/cred.rs`。Windows 上身份 64 字节密钥存为 **Credential Manager 泛型凭据**(`CredWriteW`/`CredReadW`/`CredDeleteW`,`CRED_TYPE_GENERIC`,`CRED_PERSIST_LOCAL_MACHINE`),目标名 `tetherly.identity:<device_id>`;数据目录只留 `identity.ref`(公开 device_id 指针,原子写 0600),**不再落密钥明文**。`unsafe` 仅限 `cred.rs::ffi`(原始 Win32 FFI),与 `platform.rs`/`screen.rs` 既有豁免同规则;新增 windows-sys 特性 `Win32_Security_Credentials`。
- **启动语义**:`cred::load_or_create_identity` 优先凭据库——凭 `.ref`(或旧 `identity.bin` 解析出的 device_id)读回密钥;**旧 `identity.bin` 首次运行自动迁移入库并从磁盘删除**。凭据库不可用/条目缺失/解析失败一律**回退文件**路径,启动永不依赖凭据库。
- **卫生规则(关键)**:凭据库是**机器全局、按 Windows 用户**的存储,只对**已安装数据目录**启用;临时目录节点(全部集成/单元测试)继续用该目录的 `identity.bin`,**绝不写操作者的真实 Credential Manager**。`TETHERLY_VAULT=1/0` 环境变量可覆盖(供测试显式驱动)。
- **测试**:`cred::tests::{file_fallback_roundtrips_on_every_platform, windows_vault_roundtrips_via_the_ref_pointer, installed_data_dir_opts_into_the_vault}`。Windows 上真跑 CredRead/CredWrite 往返(写→`.ref`→读回 device_id 一致→`identity.bin` 不存在→删除后读不到);两项 Windows 测试由 CI 的 `windows` job 覆盖,Unix 上 `vault_available()==false` 走文件回退。
- **诚实边界**:macOS Keychain / Linux libsecret **仍未接**(`vault_available()==false`,回退文件);`identity.bin` 与 `identity.ref` 两种形态在非 Windows/回退路径下并存兼容。

**1.12 相对 1.11(文件跨进程续传:内容寻址 + TFL2 起始偏移 + 持久侧车)**

- **内容寻址 `transfer_id`**:发送方对整份文件取 `sha256`,取前 16 字节十六进制构成 `transfer_id = "tr_" + hex[..16]`(取代随机的 per-transfer id)。同字节 → 同 id → 接收方凭 id 命中其此前落盘的半成品续传;不同字节 → 不同 id → 全新传输,不会误续。
- **core(filexfer)**:新增 `ResumeState{transfer_id,source,total,offset}` 与 `ResumeTable`;`ResumeTable::claim(source,transfer_id,total)` 仅当 **来源匹配 + 大小匹配 + 偏移 >0 且 <总长 且按 64KiB 对齐** 时返回可续偏移,否则 `None`(拒绝伪续)。`put` 将 offset 向下对齐到块边界。`IncomingTransfer` 增 `resume_from: Option<u64>`;`FileHub::offer` 委托新 `offer_resumable(...,resume_hint)`,`accept` 委托 `accept_at(...,resume_from)`(`accept_at` 校验偏移,非法即 `CoreError::ResumeOffset`)。`FileDecision`(JSON)增可选 `resume_from` 字段,新增 `FileDecision::accept_resuming`。
- **net(filechan)**:新线格式 **`TFL2`**(`magic(4) | id | total u64be | start u64be`)在 `TFL1` 基础上加 **起始偏移**;`read_header` 同时接受 TFL2/TFL1(v1 视 start=0)。新 `send_chunks_from(w,token,id,total,start,data)` 仅从 `start` 起发尾块;`recv_chunks_after_header(...,start,...)` 校验对齐与 `out.len()==start` 后追加。**每块 AAD = `id || chan_offset_le`**,把块钉死在其通道位置上 —— 块不可跨续传边界被搬移;nonce 计数 = `chan_offset/64KiB + 1`,续传必须块对齐方能与既有 nonce/AAD 匹配(协议级保证,非约定)。新增 `resumable_offset(path,chunk)` 截断半尾并返回对齐长度。
- **node(runtime)**:发送路径 `offer_and_send_bytes` 改为内容寻址;`FileOffer` 处理时按 `hubs.resume.claim(peer,id,total)` 得 `hint` 走 `offer_resumable`;`FileAccept` 回读 `decision.resume_from` 经 `send_file_data` 从该偏移发尾块。接收路径 `accept_file` 改为**增量落盘**:读头 → 校验 `total`/`start` → 预载既有 `.<transfer_id>.part`(截断到 `start`,拒绝 `aligned<start`)→ `set_len(start)` 后逐块 append + `sync_all`;`on_chunk` 回调即写即持久;中断时 `note_resume` 把块对齐偏移写入 `.<transfer_id>.resume.json` 侧车;完成后 sha256 校验 → 原子 `rename` 到 `inbox/` → 删 `.part`/侧车。**重启续传**:`load_resume_table(data_dir)` 启动时扫 `inbox/.*.resume.json` 重建 `ResumeTable`;若侧车对应的 `.part` 已丢失或短于声明偏移(崩溃撕裂态),**丢弃侧车从 0 重传**,不承诺无法兑现的续传。
- **测试(CI 绿)**:core `resume_claim_requires_source_and_size_and_alignment`、`accept_at_validates_resume_offset`;net `resume_sends_only_the_tail_from_a_chunk_boundary`、`a_chunk_cannot_be_relocated_across_resume_positions`、`resumable_offset_truncates_partial_tail`;phase1 `m1_8_file_resumes_from_persisted_offset`:预置 B 的 `inbox/`(`.part` 前 2 块 + 侧车)后**重启** B(证明重启扫描路径),配对 → offer 回带 `resume_from==2*64KiB` → 接受后 sha256 一致 → `.part`/侧车清除。全量回归见 §13。

**1.11 相对 1.10(notify.reply:桌面快速回复手机通知)**

- **协议落地(不再是 stub)**:crates/tetherly-core/src/frame.rs 新增 NotifyReply{uid,app_id,text},to_frame()/from_payload(),文本按 NOTIFY_TEXT_MAX=4 KiB 截断(不切多字节字符)。TYPE_NOTIFY_REPLY=0x0103 原样保留,MUST_UNDERSTAND 对它不再解除。
- **net 分派**:tetherly-net 的 SessionEvent::ReplyStub 占位删除,改为 SessionEvent::Reply(NotifyReply);反序列化失败即 NetError(与其它 JSON 帧一致)。
- **接收方收件箱**:NotifyHub 增 ingest_reply/take_reply_id/take_reply/reply_views,键为 reply_id(source,app_id,uid)(=sha256),取用即删(快速回复永不二次触发)。空白文本拒绝;同键后写覆盖(幂等)。面向 UI 的 ReplyView 不含正文,只有 take_reply* 才释放文本——沿用「通知正文不进列表视图/日志」的规则;QueuedReply 的 Debug 手写 redact 正文。
- **桌面端(node)**:Node::send_reply / reply_to_candidate(按候选寻址回其来源手机;空文本 CoreError::EmptyReply 本地拒绝)/ take_reply / reply_views;handle_event 收 SessionEvent::Reply 后入箱 + 发 UiEvent::ReplyReceived(只记 text_len,不记正文)。
- **桌面 UI**:ui/index.html 每张候选卡加「快速回复…」输入框 → POST /api/reply {id,text};新增「收到的回复」区 + POST /api/reply/take {id} 取出(正文不落快照)。Snapshot 增 replies:[ReplyViewDto]。
- **Android(手机)**:SessionHandle::take_replies()(Vec<NotifyReply>,一次取空)+ serve 循环解析 TYPE_NOTIFY_REPLY(同 uid+app 覆盖);JNI 新增 nativeTakeReplies(JSON 数组,空则空串;正文为用户内容、禁止写日志);Kotlin NativeBridge.takeReplies()/PhoneReply,NotifyListener.drainReplies() 用通知自身的 RemoteInput action 就地回复(不合成应用未声明的回复;activeNotifications 无匹配则丢弃)。真机触发 RemoteInput 需系统权限/前台通知,Manual-required。
- **测试(CI 绿)**:core 单测 3 条;tests/phase2.rs::m2_6 双节点真实会话桌面→手机 reply 到达且只取一次;crates/tetherly-android/tests/client_e2e.rs::android_client_receives_a_quick_reply_once 真 accept_session 发 reply、客户端 take_replies() 一次拿到。全量 188 passed(原 182)。

**1.10 相对 1.9（Phase 1 Android 原生库：真实身份 + Noise 会话落地）**

- **新 crate `tetherly-android`**（`crates/tetherly-android`）：lib 名 `tetherly_android`，crate-type `["cdylib","rlib"]`；`cdylib` 编出 `libtetherly_android.so`（arm64-v8a），`rlib` 使 host/CI 可跑同一份代码的集成测试。依赖仅 `tetherly-core`/`tetherly-crypto`/`tetherly-net`（`default-features=false`）/`tokio`/`serde_json`/`thiserror`/`tracing`/`zeroize`；`jni` 依赖门控在 `[target.'cfg(target_os = "android")'.dependencies]`，桌面构建不引入。
- **真实身份（取代占位）**：`Identity::from_secrets(id_sk, n_sk)` 生成真 Ed25519 签名密钥 + 独立 X25519 静态密钥（ADR-001），`device_id = DeviceId::from_id_pk(id_pk)`；`identity.bin` 64 字节（temp+rename、`0600`）落盘，`load_or_create_identity` 首启生成、之后加载。**彻底删除** v1.3 的 `idPk = sha256(idSk)` 占位路径。
- **信任持久化**：`trusted.json` 与桌面节点**同构**（`TrustFile { peers: Vec<TrustedPeerDto> }`），两端文件可互换；配对成功即写，resume 无 PIN 重拨复用（ADR-002）。
- **会话客户端**：`AndroidClient::start(addr, pin, once)` 在独立线程跑 `current_thread` tokio runtime，按桌面**相同**的 `dial_session` 路径（首配 Hello + SPAKE2 8 位 PIN，之后 Noise IK resume）拨 45717；`SessionHandle` 暴露 `notify()`/`state_code()`/`connected_peer()`/`last_error()`/`stop()`；指数退避重连 250ms→5s，`TYPE_PING` 回 pong。
- **诚实能力集**：手机 `caps = ["notify","clip","file"]`，**永不**广告 `input`/`screen`（手机不是被控桌面）。落点是 `SessionConfig.caps: Option<Vec<String>>` 显式覆盖（`tetherly-net` 新增字段，默认 `None` 走既有推断）。
- **JNI 薄壳**（`#[cfg(target_os = "android")]` 门控）：`Java_app_tetherly_NativeBridge_nativeAttach/SendNotify/DeviceId/IdPk/State/LastError/Stop`，与 Kotlin `NativeBridge`/`NodeSession` 对接。lib 顶层 `#![deny(unsafe_code)]`，仅 `jni.rs` 以 `#![allow(unsafe_code)]` 作 JNI ABI 的**唯一**豁免。
- **端到端已验（真机）**：`android_probe`（同 crate `--bin`，aarch64-linux-android）推到云手机 `adb shell` 直接跑——两台 Galaxy Fold 分别生成新身份、以 PIN 配对成功、桌面节点收到带验证码的 `notify.push`（`has_otp:true`，正文不入快照/日志）、写双方 `trusted.json`；**换错 PIN 仍连接成功**，证明走的是 Noise IK resume 而非重新配对。
- **CI**：`android` job 先 `dtolnay/rust-toolchain`（target `aarch64-linux-android`）+ `cargo install cargo-ndk` + `cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build -p tetherly-android --release`，再跑 Gradle；`jniLibs/` 是产物、已入 `.gitignore`（不入库）。

**1.9 相对 1.8（Phase 3 键鼠真机链：SendInput 注入 + LL 钩子捕获）**

- **受控端注入**：`SystemInputSink`（`InputSink` 端口实现）——`SendInput` 绝对移动（1000 单位→65535 归一化）、按钮 mask（`BTN_LEFT=1/BTN_RIGHT=2/BTN_MIDDLE=4`）、滚轮、VK 键盘；`NodeConfig.input_sink` 注入，CI 缺省 `None` 仅观察者。协议级过滤（seq 窗口/座位门控）后经 `MemorySink` 观察者 trace 镜像到真实注入。
- **主控端捕获**：`Node::spawn_input_capture` —— `WH_MOUSE_LL` 恒挂（只读光标做过边检测）、`WH_KEYBOARD_LL` **仅远程座位时挂**（本地键盘隐私）、注入事件按 `LLMHF/LLKHF_INJECTED` 过滤防单机回环；喂给与 CI 相同的 `drive_input*` 路径。**注入探针实测**：过边后受控端光标真实移至左缘 (0,539)、中心移动精确、Win/Esc 注入生效。
- **Manual-required**：双桌面 100 次物理往返（需两台交互桌面）。

**1.8 相对 1.7（Phase 5 真机闭环：DXGI 采集 + zstd 编码 + BitBlt 呈现 + p95）**

- **真机采集**：`crates/tetherly-node/src/screen.rs` 的 `DxgiSource` 实现 Windows DXGI Desktop Duplication——`D3D11CreateDevice`（BGRA）→ `IDXGIDevice::GetAdapter` → `IDXGIAdapter1::EnumOutputs(0)→IDXGIOutput1::DuplicateOutput` → `AcquireNextFrame(0)`（`DXGI_ERROR_WAIT_TIMEOUT` 映射为 `Ok(None)`，桌面未变时不产帧）→ staging texture `CopyResource` + `Map(READ)` → 行尾裁剪成 tightly-packed BGRA8 → `ScreenFrame::raw`。COM 单线程模式 `CoInitializeEx(MULTITHREADED)`；D3D11/DXGI 对象经 `unsafe impl Send/Sync`（单 pump 串行 + `Mutex`），是 D3D11 的标准单线程用法。
- **真机编码（可换端口）**：`ZstdCodec`（无损，level 3）实现 `ScreenCodec`；`EncodedSource<S>` 是 `ScreenSource` 装饰器——每帧压缩，压不小就发原帧（病态编码器不可能放大传输）。生产默认链为 `EncodedSource<SystemSource>`；未来 OpenH264/rav1e 作为同一端口替换。**先无损帧 + 后接编码器**的落点。
- **真机呈现**：`SystemSink` 消费侧用 GDI——`CreateWindowExW` + `CreateCompatibleDC` + `CreateDIBSection`（32bpp top-down）+ `BitBlt`（`SRCCOPY`）。**同步 BitBlt**（本线程无消息泵，不依赖 `WM_PAINT`），每帧直写窗口 DC 立即显示；尺寸变化时重建 DIB。
- **分片传输 + 重组**：`ScreenFrame::to_frames()` 把超 64 KiB 内层预算的整帧拆成 `TMF1` 分片（header 24 字节：magic+total+index+count+seq）；`ScreenAssembler::push` 重组（跨流重置、单流内存上限 `SCREEN_PAYLOAD_MAX`，谎报 count 被拒而非分配）。整帧 `TMV1` 走单帧快路径，小帧线上字节不变。
- **客户端解码链**：`attach_screen_client` = `ScreenAssembler` 重组 → 若 `compressed` 则 `screen_codec.decompress(expected_len)` 校验解压长度 → `ScreenReceiver::apply` 校验 seq 窗口 → `screen_sink.present`。呈现目标 `screen_sink` 默认 = 观察者 `MemoryScreenSink`（CI 断言），生产注入 `SystemSink`。
- **p95 帧间隔预算**：`tetherly-node` 加 `ScreenIntervalTracker`（wall-clock，微秒，bounded 1024 样本），`screen_send_frame` 每次成功入队记录一次；`Node::screen_latency_p95_us()` 暴露，UI `Snapshot.screen.p95_frame_interval_us` 输出。core 保持无时钟（确定性）。
- **生产默认**：`Node::start` 默认 `screen_source = EncodedSource<SystemSource>`（DXGI+zstd）、`screen_codec = ZstdCodec`、`screen_sink = 观察者`；`tetherly run` 注入 `SystemSink` 并起 16ms 帧 pump（非 Streaming 状态直接跳过 grab，空闲零开销）。`--screen-adb <serial>` 把源换成 `AdbSource`（云手机屏幕）。测试注入 `MemoryScreenSource`（原帧），路径不受影响。
- **AdbSource 跨设备真机 E2E（云手机 → Windows）**：`AdbSource`（`adb exec-out screencap -p` → PNG 解码 → BGRA8，PNG 字节不变返回 `Ok(None)`）接入既有链路实测：1063 帧发出 / 1060 收达 / 0 丢帧，每帧解压后精确 720x1280x4 BGRA，GDI 窗口画面与真机屏 ±8 容差 97.6% 一致（残差 = loading 动画相位 + DWM 色彩管理），p95 ≈0.9–1.45s（`screencap` 源速率主导）。真机流程暴露并修复两处缺口：`/api/screen/start` 现自动拨号 screen 通道（单对端时；多对端诚实拒绝）；visitor 接收计数 `screen_rx_stats` 接入 Snapshot。
- **测试**：`tests/phase5.rs` 4 个测试全绿（M5.1 帧序 1..=5 严格单调、M5.2 Start-before-allow 拒绝计数、M5.3 对端离开回 `Idle` 并重置 seq、M5.4 45720 拒未配对 dial）。`cargo fmt --check` / `clippy -D warnings` / `cargo test --workspace` 全绿（core 116 + phase1 7 + phase2 4 + phase3 4 + phase4 10 + phase5 4 + …）。
- **Manual-required（收窄）**：真实开放编码器 OpenH264/rav1e（端口已就位、未接入）；DXGI 采集端的真实像素捕获与 Windows 跨桌面 100 帧 p95（需交互桌面）；macOS CGImage/CGWindow、Linux X11 SHM（Wayland 仍标「不支持」）。**DXGI 采集 / zstd 编码 / BitBlt 呈现 / p95 计量 / 跨设备（云手机源）真机闭环已从 Manual-required 转为已实现**。

**1.7 相对 1.6（Phase 5 远程屏幕 CI 切片）**

- 新增 `tetherly-core::screen`（**无 OS API**）：屏幕帧魔数 `TMV1`、proto 1、`SCREEN_SEQ_WINDOW=65536`、`PixelFormat{Bgra8,Rgba8}`、`ScreenFrame` 编解码（header 31 字节：magic+proto+seq+rect+fmt+compression+stride）、`ControlMsg{Start,Stop,Ack{seq}}` 编解码（14 字节）、`ScreenSender`（单调 seq + consent 门控 + push/drain/refused 计数）、`ScreenReceiver`（seq 窗口丢旧/拒大跳/bounded trace）、`MemoryScreenSource`/`MemoryScreenSink`（确定性 CI 假实现）、`ScreenSink`/`ScreenSource` 端口。
- 新端口 **45720**（`SCREEN_PORT`）：Noise IK resume only；`screen_only: true` 的会话在 `HelloDisposition::Pair` 时返回 `NetError::ScreenRequiresTrust`——配对永远只在 45717。Hello 完只允许已配对对端。
- **强制本机同意（M5.2）**：对端连上 45720 仅置 `ScreenState::Requested`，必须本机显式调用 `Node::screen_allow()` 才进入 `Allowed`；对端 `Start` 在 `Allowed` 前被拒且计数。`ScreenSender::push` 仅在 `Streaming` 状态出帧。
- **RustDesk 仅作架构借鉴**：RustDesk 客户端与服务端均为 **AGPL-3.0**，不得以任何形态链入 `Cargo.toml`；`deny.toml` 加 `hbb_common` / `rustdesk` / `rustdesk-server` / `scrap` 为结构化红线。借鉴的公开工程常识（分离采集+编码器 swap+P2P+中继）**不**包含居会合服务器（hbbs/hbbr），匹配 §2.3 禁云账号。
- `tetherly-node`：`NodeConfig.screen_port`（默认 45720）+ `screen_source: Option<Arc<dyn ScreenSource>>`（测试注入 MemoryScreenSource）；`Shared` 加 `bound_screen: AtomicU16`、`screen_out`、`screen_engine: Mutex<ScreenSender>`、`screen_source: Arc<dyn ScreenSource>`、`screen_sink: Mutex<MemoryScreenSink>`、`screen_ctl`；`spawn_listeners`/`spawn_overlay` 各加 45720 绑定与接受；`attach` 的 `caps.update` 加 `"screen"` + `screenport=`；`Node::screen_allow/revoke/start/stop/state/stats/send_frame/sink_snapshot/port/addr`。
- `crates/tetherly-node/src/screen.rs`：`SystemSource`/`SystemSink`按目标 `#[cfg]` 选择后端。**v1.19**：`MacSource`（`screen/macos.rs`,CoreGraphics 采集；呈现需 AppKit `NSWindow`，诚实 `ScreenRefused`）与 `X11Source`（`screen/x11.rs`,MIT-SHM 采集 + plain `GetImage` 回退；呈现 `PutImage`）。另有 Windows `DxgiSource`（DXGI Desktop Duplication）与 `AdbSource`（adb 云手机）；其余目标与 Wayland 诚实 `ScreenRefused`。**真机采集 Manual-required**。
- UI：`ui/index.html` 加「远程屏幕（实验）」区——「① 需本机点击允许才推流；② Wayland 暂不支持；③ 真机采集为 Manual-required」。路由 `POST /api/screen/allow|start|stop`。`Snapshot` 加 `screen_state` 和 `screen` 计数器。
- `tests/phase5.rs`：M5.1 consent-then-start 帧序严格递增 1..=n、M5.2 Start-before-allow 被拒且计数、M5.3 对端离开→`Idle` 重置 seq、M5.4 45720 拒绝未配对 dial。`cfg()` 注入 `MemoryScreenSource`。
- **Manual-required**：真机 DXGI Desktop Duplication / WGC（Windows）、CGImage（macOS）、X11 SHM（Linux）采集；真实编码器（OpenH264/rav1e）；真实窗口呈现（Windows HWND / macOS CGWindow / Linux Wayland portal）；p95 帧间隔预算真机验证。M5.5 硬件编码 / 真机注入「未做」。

**1.6 相对 1.5（Phase 4 iPhone ANCS CI 切片）**

- 新增 `tetherly-core::ancs`（**无 OS API**）：Notification Source 8 字节事件解析、Control Point 编码（`GetNotificationAttributes` / `GetAppAttributes` / `PerformNotificationAction`）、Data Source **按字节**分片重组、`AncsIngress` 状态机、`AncsTransport` 端口与 `MemoryAncsTransport`。
- §9.4 硬约束逐条落地：先订 Data Source 再订 Notification Source；`on_notification_source` **只入队不写** Control Point（写在 `tick`）；Control Point 串行（同一时刻最多一个未应答请求）；分片按字节拼满；静默首响应重试一次后放弃；断线指数退避（500ms→30s 封顶）；uid 去重。
- M4.3：`PreExisting` 标志的通知一律丢弃，重连不连弹；uid 与 app 名缓存在重连后保留。
- M4.2：动作只由 `ancs_actions(app_id, allowlist)` 决定——恒有 `copy`/`dismiss`，**只有**白名单命中本地 scheme 时才加 `open`。ANCS 载荷里的 url 从不被读取或执行；`NotifyPush::truncate` 仍会剔除含 `://` 的动作。
- `tetherly-node`：`NodeConfig.ancs`（`Option<Arc<dyn AncsTransport>>`）、`Node::ancs_connect/ancs_notification_source/ancs_data_source/ancs_tick/ancs_disconnected/ancs_state`、`set_open_allowlist`/`add_open_rule`、`open_candidate(id, &dyn Opener)`。`Opener` 是端口：生产用 `SystemOpener`（`rundll32 url.dll,FileProtocolHandler` / `open` / `xdg-open`，无 shell），CI 用 `MemoryOpener`。
- UI：配对向导拆成两步——①「系统蓝牙里连 iPhone（ANCS）→ 启动 ANCS」②「与本机/其他设备 SPAKE2 配对」。新增 `/api/ancs/connect`、`/api/ancs/allow`、`/api/open`。
- `tests/phase4.rs`：M4.1 模拟时延预算、M4.2 白名单门控 + 载荷 url 不打开、M4.3 重连不连弹、Control Point 串行与回调线程不写、跨 UTF-8 分片重组、`Removed` 清候选、无传输时惰性、退避拒绝热重连。
- **Manual-required**：真实 BLE GATT（Windows `windows` crate / macOS CoreBluetooth / Linux BlueZ）、物理 iPhone 的 p95 < 2s、蓝牙关 30s 再开的真机复现。M4.4 / M4.5（可选 iOS App、无 VPN）本切片不涉及。

**1.5 相对 1.4（Phase 3 原生键鼠 CI 切片）**

- 键鼠走独立 TCP **45719**。Hello 之后只允许 `ResumeNoise`；未知对端返回 `InputRequiresTrust`，配对仍只在 45717。
- 载荷魔数 `TIN1`、proto 1、`seq` u64be、屏幕单位 0..=1000。内层 type `0x0401`–`0x0407`，带 `MUST_UNDERSTAND`。控制面 JSON 表仍不含这些 type，也没有 `input.*` JSON。
- 引擎在 `tetherly-core`（无 OS API）。节点用 `MemorySink` 注入；`caps.update` 广告 `inputport=`。过边第一帧 `x=0` 是进入，不是离开；只有先进屏幕内部再回到 `x=0` 才回 server。
- `tests/phase3.rs`：未配对 45719 拒绝、过边+按键跟随、对端断开光标回本机、键鼠聚焦时 `clip.set` 仍走 45717。
- 真机光标注入（Win SendInput / macOS / Wayland portal+libei）与双桌面 100 次物理往返：**Manual-required**。M3.4 DeskFlow 网关保持 experimental、默认关，**不链接** DeskFlow / Lan Mouse / KDE 源码。

**1.4 相对 1.3（Phase 2 EasyTier 侧车 CI 切片）**

- 仍 **不** 把 `easytier*` 放进 `Cargo.toml`。发现走本机 RPC `127.0.0.1:15888`（2s）或 `easytier-cli -o json peer list`（失败则 `peer`），任意 JSON walker + 文本 IPv4 扫描；缺侧车是成功（LAN only，M2.4）。
- 虚网 bind 与 LAN bind **分开**：LAN/mDNS 永不在 EasyTier TUN 上 advertise。overlay 监听仍禁止 `0.0.0.0`/`::`；无 TUN IP 时 bind 空列表成功。
- 路径：`PathKind::Lan` 赢过 overlay attach；overlay 会话结束只拆自己的 `sess_addr`，不拆仍活着的 LAN 条目（M2.2）。
- UI：缺侧车显示「仅局域网（未检测到 EasyTier 侧车，不影响 LAN）」；在线显示「虚网在线」。CLI：`--overlay-cidr`、`--no-overlay`。
- `tests/phase2.rs`：M2.3 停侧车不崩、M2.4 未装侧车 Phase 1 不变、M2.5 无 mDNS 靠 extra_peers 单播、M2.2 重复 overlay 扫描仍保 LAN。夹具 CIDR 用 `10.199.199.0/24` 隔离本机真实 EasyTier TUN。
- M2.1 Android 蜂窝 ↔ 电脑通知 p95 < 3s：**Manual-required**（真机蜂窝 + 侧车 soak）。

**1.3 相对 1.2（Phase 1 LAN 桌面 + Android）**

- 新增 `tetherly-node`：身份/信任落盘、LAN bind（禁止 `0.0.0.0`、跳过 EasyTier `10.144.144.0/24`）、mDNS、`notify.push` / clip / file 控制面、文件数据走 **45718**、本机 HTTP UI **45716**（仅 127.0.0.1）。
- 桌面壳本阶段是 loopback HTTP + `ui/index.html`，**不是** Tauri/WebView2。`src-tauri` 标为下一桌面壳迭代，避免 CI 依赖 WebView2。
- Android：`NotificationListenerService` + 前台服务 + gradle CI。`libtetherly_android.so` / 真 Ed25519 JNI **尚未构建**；无 `.so` 时不擅自 raw-TCP。协议路径由 `tests/phase1.rs` 双节点 loopback 覆盖。
- `cargo deny` 允许 **BSL-1.0**（Boost，OSI/FSF）：`arboard` → `clipboard-win`。仍拒绝 GPL/AGPL/LGPL；`easytier*` 仍 ban。
- M1.2 / M1.3（MemoryInsertor）/ M1.4 Win 回环 / M1.5 persist 重拨 / M1.6 日志脱敏 / M1.7 文件确认+10MiB sha256：CI 全绿。M1.1 物理 Android p95(v1.18 真机实测关闭)、Win+Android 8h、真记事本 UIA、macOS M1.1：**Manual-required** / 「mac 下一迭代」；v1.24 把文件单文件上限 `FILE_MAX`（报价即拒、拒绝不拆会话）与网卡分类（`BindClass` 单一真理源 + 可解释）补实。

**1.2 相对 1.1（Phase 0 评审修订）**

- PairBind AEAD nonce 按 TCP 角色拆开（`tetherlybndI` / `tetherlybndR`），禁止同一 `(pair_key, nonce)` 加密两份明文。
- 状态机改为 PairBind → NoiseIK → **成功后**写信任库；Hello `name`/`caps` MITM 不得留下信任记录。
- 内层帧带 `u64 msg_id`；握手 20s 超时；新 TCP 每 IP 30/min。
- `NotifyPush` 截断必须落在 UTF-8 字符边界。PIN 生成在 `getrandom` 失败时 fail-closed。

**1.1 相对 1.0（调研修订）**

- EasyTier 许可证已核实为 **LGPL-3.0**，侧车策略从「待查」变成硬约束；默认虚网是 `10.144.144.0/24` 不是 `10.126.126.0/24`。
- 跨网发现改为 **EasyTier 节点单播**，不依赖 TUN 上的 mDNS（组播经常过不去）。
- SPAKE2 必须把 Hello 里的身份公钥绑进口令派生；PIN 先经 **Argon2id** 再进 PAKE，避免抓包离线爆 8 位码。
- `PairBind` 必须在 SPAKE2 会话密钥下加密，不能明文送 X25519 公钥。
- v1 **不在线上单独发 `otp.candidate`**：转发 `notify.push`，各展示端自己提取。短信正文里本来就有码。
- 键鼠 **原生 Noise 端口为主**；DeskFlow 网关要讲人家的 **TLS（默认端口 24800）**，Phase 3 退出不依赖互通。
- Phase 0 crate 收成四件套；ANCS / 键鼠 / uniffi 按阶段再加。
- 分清两套配对：系统蓝牙（ANCS）≠ Tetherly SPAKE2（IP 会话）。
- 锁定密码学 crate：`snow`、`spake2`、`argon2`、`ed25519-dalek`、`x25519-dalek`、`mdns-sd`。

---

## 1. 怎么用这份文档

| 角色 | 先读 |
|---|---|
| 写第一行代码 | §3 ADR、§6 配对、§7 帧、§8 Phase 0 范围、§12 M0.* |
| 加能力 | §5、§8 插件、§10 反模式 |
| 验收 | §12，数字必须可测 |
| 安全 | §4、§6、§11 |

分层：ADR → 协议 → 里程碑 → Phase 5 候选。ADR 不经修订记录不得违反。

---

## 2. 产品

### 2.1 一句话

一个人的 Windows / macOS / Linux / Android / iPhone 在同一信任圈里：通知和验证码到电脑、剪贴板和文件可传、多台桌面共用一套键鼠。同一局域网直连；跨网靠用户自己的 EasyTier 虚网。

### 2.2 必须守住的差异

| 点 | 现实 |
|---|---|
| 跨网 | 桌面与 Android 走 EasyTier **侧车**；iPhone 跨网不是 v1 |
| 验证码 | 本机打分提取；写入必须当次点击 |
| 键鼠 | 桌面 ↔ 桌面；自有 Noise 端口。Windows 真机链 v1.9 已落地（SendInput 注入 + LL 钩子捕获，键盘 hook 座位门控）；macOS/Linux 下一迭代。DeskFlow 仅实验网关 |
| iPhone | 电脑用 ANCS/BLE 收新通知，再经 IP 会话转给其他已配对电脑 |

### 2.3 非目标（v1）

- 远程桌面 / 屏幕镜像（**v1 非目标已由 v1.7 Phase 5 取代**：`tetherly-core::screen` 帧+控制编解码、强制本机同意、45720 独立端口的 CI 切片；**v1.8 已落地** Windows DXGI 采集 / zstd 编码 / BitBlt 呈现；macOS/Linux 与开放编码器仍 Manual-required。完整远程桌面仍不在 v1 承诺范围）
- 云账号、云端存通知或验证码
- iOS 被控
- Tetherly 进程内嵌 EasyTier 或 Network Extension VPN
- 兼容 KDE Connect
- 把对端 URL/正文当命令执行
- Android 10+ 后台静默读剪贴板
- 在 EasyTier TUN 上依赖组播/mDNS
- 把 8 位配对码当「高熵密钥」而不做 PAKE + KDF

### 2.4 端能力

| 能力 | Win | macOS | Linux | Android | iPhone |
|---|---|---|---|---|---|
| LAN 会话 | 听 + 拨 | 听 + 拨 | 听 + 拨 | 主要拨号 | 可选 App 拨号 |
| EasyTier 跨网 | 侧车，单播 | 同左 | 同左 | 侧车，拨虚网 IP | ❌ v1 |
| 通知上行 | — | — | — | NotificationListener | ANCS → 电脑 |
| 通知展示 | ✅ | ✅ | ✅ | ✅ | 可选 App |
| OTP 提取 | 展示端本地 | 同左 | 同左 | 同左 | 电脑提取后展示 |
| OTP 填入 | UIA，点击 | AX，点击 | AT-SPI，点击 | 无障碍，点击 | 仅复制 |
| 剪贴板 | ✅ | ✅ | ✅ | 按钮发送 | 前台 |
| 文件 | ✅ | ✅ | ✅ | SAF | 可选 App |
| 键鼠 server/client | ✅ | ✅ | Wayland beta | ❌ v1 | ❌ |
| 远程屏幕 server | ✅（v1.8 真机闭环） | **已实现（v1.19）**:`MacSource` CoreGraphics 采集;呈现需 AppKit NSWindow（诚实拒绝） | **已实现（v1.19）**:`X11Source` MIT-SHM 采集 + `PutImage` 呈现;Wayland 不支持 | ❌ v1 | ❌ |

话术：桌面全功能；Android 是信源和文件端；iPhone 是通知/验证码发送源。

---

## 3. 约束决策（ADR）

### ADR-001 身份：Ed25519 签名 + 独立 X25519 静态密钥

首次启动生成：

- `id_sk` / `id_pk`：Ed25519，只用于签名
- `n_sk` / `n_pk`：X25519，只用于 Noise

```
device_id = "tdev_" + base32lower(sha256(id_pk)[0..10])  // RFC 4648，无填充
```

信任库存对端 `id_pk` 与 `n_pk`，校验 `device_id` 必须能从 `id_pk` 重算。禁止随机 UUID 当长期身份。禁止把 Ed25519 标量直接当 X25519。

密钥:**Windows 系统凭据库已接(v1.13)**;否则用户目录文件 ACL 仅本人;进程内 `zeroize`。

### ADR-002 首次配对：Argon2id + SPAKE2；之后 Noise IK

1. 发起端显示 **8 位数字**，TTL **3 分钟**，最多 **5 次失败**，然后对该对端或源 IP **锁 15 分钟**。
2. `pwd = Argon2id(PIN, salt, m=64MiB, t=3, p=1)`  
   `salt = sha256("tetherly-pair-v1" || min(id_pk_A, id_pk_B) || max(id_pk_A, id_pk_B))`  
   这样 PIN 与 Hello 里已经交换的身份绑定，中间人换公钥会让 PAKE 失败。
3. SPAKE2 用 crate **`spake2`**（RustCrypto 系，Apache-2.0/MIT），群与密码套件在 Phase 0 写入 `tetherly-crypto` 测试向量后冻结。
4. SPAKE2 成功得到 `pair_key`。用 `pair_key` 加密 **PairBind**（双方 `n_pk` + 对 `id_pk||n_pk||device_id` 的 Ed25519 签名）。ChaCha20-Poly1305 nonce 按 TCP 角色拆开：发起者 `tetherlybndI`，响应者 `tetherlybndR`。禁止同一 `(pair_key, nonce)` 加密两份明文。
5. PairBind 校验通过后做 Noise IK。**信任库只在 Noise 进入 transport 之后写入**；prologue 失败必须不留信任记录。别名用已签名的 `device_id`，不用未认证 Hello `name`。
6. 之后每次连接：**Hello（明文）→ Noise_IK**（已知对端 `n_pk`），不再走 PIN。

禁止：6 位 PIN、HMAC(双方公钥) 当同一显示码、自绘 TLS、明文 PairBind、把 PIN 当 Noise 预共享密钥而不做 PAKE。

### ADR-003 业务只走一层 Noise

模式字符串冻结：

```
Noise_IK_25519_ChaChaPoly_BLAKE2s
```

实现 crate：**`snow`**（纯 Rust Noise）。prologue = `sha256(canonical_hello_A || canonical_hello_B)`，双方按 `device_id` 字典序拼接，防止 Hello 被换。

Hello / SPAKE2 / 密文 PairBind 在 Noise 之外；此后控制面与文件面都是 Noise transport。不再叠 TLS。

### ADR-004 ANCS 是 Ingress，不是 Transport

`Transport` 只有 IP：LAN TCP、Phase 2 虚网上的 TCP。iPhone 通知由**电脑**上的 ANCS 客户端变成 `PhoneNotification`，进 `NotifyHub`。禁止把 BLE 当 RPC。

iPhone v1 **不做** Tetherly SPAKE2，除非可选 iOS App（M4.4）。系统蓝牙配对 ≠ 应用配对。

### ADR-005 键鼠走独立 Noise 端口

- 主路径：`45719/tcp`，Noise IK（可复用已配对 `n_pk`），自有长度前缀事件帧。不进 JSON 控制面。
- DeskFlow 网关：默认 **关闭**。官方默认端口 **24800**，现代 DeskFlow **默认 TLS**。要互通就必须实现其证书/指纹模型，并避开已知 TLS 接受失败卡死问题。Phase 3 **退出不要求** 与官方 client 互通。
- 禁止链接 DeskFlow / Lan Mouse / Barrier 源码。

### ADR-006 EasyTier 侧车（已核实 LGPL-3.0）

详见 [adr/006-easytier.md](adr/006-easytier.md)。`easytier*` 不准进依赖图。跨网用官方进程；发现用虚网 **单播**，不用 mDNS。

### ADR-007 iOS v1 无应用内 VPN

可选 iOS App 只跑 LAN。不写「再开 EasyTier VPN」——双 VPN 互斥，用户会按文档去做然后骂你。

### ADR-008 打开应用的 URL 只来自本机规则

`(app_id glob → 本机 url 常量)`。丢弃对端一切 url/command。验证码按钮只复制。

### ADR-009 许可证

| 依赖 | 允许 |
|---|---|
| MIT / Apache-2.0 / BSD / BSL-1.0 | 链接（BSL = Boost，OSI；`arboard`/`clipboard-win` 需要） |
| EasyTier LGPL-3.0 | 仅侧车 |
| DeskFlow / Lan Mouse / KDE Connect 源码 | 禁止复制实现 |
| `cargo deny` | 拒绝 GPL/AGPL/**LGPL** 进入 Tetherly 依赖图；ban `easytier*` |

Tetherly 自身：**Apache-2.0 OR MIT**。

### ADR-010 验证码写入必须用户当次触发

探测可常驻；`insert()` 只挂按钮/快捷键。日志只记 `code.len()`、score、规则 id。

### ADR-011 v1 线上不单独发 OTP

展示端对 `notify.push.body` 本地跑提取器。不新增 `otp.candidate` 消息（码已在正文里，再发一遍只增加不一致）。

### ADR-012 桌面听、手机拨

LAN 上桌面默认 `listen=true`。Android 默认只拨号（蜂窝网入站困难）。桌面之间全互拨已发现的地址。防火墙默认只绑专用网/已知虚网，不绑公网 `0.0.0.0` 除非用户改配置。

### ADR-013 显示 PIN 必须作用于「活着的」节点

PIN 只对**生成它的那个进程**有意义:`Node` 的 `shared.pin`(含 `pin_created_ms`)是纯进程内状态,**从不持久化**。因此在**另一个**进程里生成并打印的 PIN 永远无法被正在运行的守护进程接受(其 `pin_created_ms=0` ⇒ 任何 dial 都判 `PinExpired`)。

结论:`tetherly pin` **不得**自起节点生成 PIN;它必须是运行中节点的**回环客户端**(`POST 127.0.0.1:<ui_port>/api/pin`,默认 45716),无守护进程监听时**诚实非零退出**。UI「显示 8 位 PIN」按钮走同一条路由。v1.18 前 `Cmd::Pin` 违反此条(在一次性进程里 `Node::start` + `generate_pin`),真机排查时表现为「PIN 显示了却一直 expired」,已修复并加回归测试。

**同一结论推广到全部 CLI 控制面(v1.20)**:`tetherly dial` 与 `tetherly notify` 同样不得自起节点——前者的第二次 `Node::start` 会与运行中守护进程抢同一批 LAN 端口而死于 `no LAN address bound`,后者的 `inject-notify` 在本进程构造推送、永远到不了对端。两者均改为活节点的回环客户端(`POST /api/dial` / `POST /api/notify`),把 PIN 装置、拨号、persist、`send_notify` 全部交给持有进程内状态的那个进程;`inject-notify` 已删除,由 `notify` 取代。UI「连接」按钮一直走的 `/api/dial` 即此路径。

---

## 4. 威胁模型

| 对手 | 能力 | 保证 | 不保证 |
|---|---|---|---|
| LAN 路人 | mDNS、扫端口、改 Hello | 未完成 SPAKE2 读不到 PairBind 与业务 | 设备名、id 指纹会广播 |
| 配对 MITM | 换身份公钥 | salt 绑定双方 `id_pk`；PAKE 失败 | 用户把 8 位码念给攻击者 |
| 抓 SPAKE2 包离线猜 PIN | 约 10^8 空间 | Argon2id 64MiB×3 次拉高成本 + 在线锁定 | 超算长时间离线（可接受） |
| 已配对恶意节点 | 垃圾通知/文件 | 限流；文件要确认 | 能看见你允许转发的通知正文 |
| EasyTier 中继 | 密文 | Noise 对中继保密 | 流量形态 |
| 已解锁电脑 | 读密钥文件 | ACL / 凭据库 | 全盘加密是用户的事 |
| 恶意通知正文 | 钓鱼链接 | 截断；不执行 URL | 用户仍可能被骗去复制码 |

崩溃报告默认无载荷。无遥测。

---

## 5. 架构

```
┌──────────────────────────────────────────────────────────┐
│ UI：Phase 1 = 本机 HTTP 127.0.0.1:45716 + ui/index.html   │
│     下一迭代：Tauri/React（src-tauri）；Android；可选 iOS   │
├──────────────────────────────────────────────────────────┤
│ NotifyHub  Clip  File     │ InputEngine（45719，Phase 3）  │
├──────────────────────────────────────────────────────────┤
│ Session：Hello → [SPAKE2|Noise IK] → caps                  │
├──────────────────────────────────────────────────────────┤
│ LanTcp（mDNS 仅真实 LAN） │ Phase2：虚网单播（无 mDNS）     │
├──────────────────────────────────────────────────────────┤
│ Ingress：Android NLService │ 电脑 ANCS/BLE（Phase 4）      │
├──────────────────────────────────────────────────────────┤
│ tetherly-core：OTP、帧、状态机、去重、白名单（无 OS API）    │
└──────────────────────────────────────────────────────────┘
```

**iPhone 验证码**

```
短信通知 --BLE ANCS--> 电脑 Ingress --> NotifyHub
  --> 本机 extract(body) --> 弹窗
  --> 可选转发 notify.push 给其他电脑（对方再 extract）
用户点填入 --> Insertor
```

**Android 验证码**

```
NLService --> notify.push --> 电脑 NotifyHub --> extract
```

**跨网**

```
Android --EasyTier 虚网 TCP 45717--> 家里电脑
发现：easytier-cli peer / RPC 列出虚网 IPv4，逐个拨端口，Hello 认人
```

### 5.1 发现

**真实 LAN（Phase 1）**

- 服务：`_tetherly._tcp`
- crate：`mdns-sd`
- TXT（短，避免超 UDP 限制）：

```
v=1
id=<device_id>
fp=<hex(sha256(id_pk)[0..6])>   // 12 hex 字符，仅展示
caps=notify,clip,file
```

端口在 SRV。完整 `id_pk` 只在 Hello。

**虚网（Phase 2，已接线）**

- 默认 CIDR `10.144.144.0/24`，可用 `--overlay-cidr` 覆盖。接口名 `et*` / `easytier` / `tun*`（不含 `eth*`）只作启发式分类；**bind / 广告仍要求地址落在 overlay CIDR**，避免把无关 TUN 当虚网。
- `GET` EasyTier RPC（本机 `15888`，超时 `OVERLAY_PROBE_MS=2000`）或解析 `easytier-cli -o json peer list`（失败则 `peer`）。JSON 任意形状 walker + 文本 dotted-quad 扫描。Windows CLI 用 `CREATE_NO_WINDOW`。
- 对每个 overlay peer IPv4 **TCP 连接 45717**，超时 2s。`extra_peers` 是拨号目标，**不必**落在 CIDR（测试 / 手动 IP）。
- 虚网 bind 与 LAN bind 分开；**禁止**在虚网接口上发 mDNS 作为发现手段。
- 同设备已有 LAN 会话时丢弃 overlay attach；overlay 会话结束不拆 LAN。
- 未装或停 EasyTier：`lan_only`，UI「仅局域网」，不崩。

IPv6：v1 不做。

---

## 6. 身份、配对、会话

### 6.1 存储

| 材料 | 位置 |
|---|---|
| `id_sk`、`n_sk` | **凭据库已接(v1.13 Windows / v1.14 macOS+Linux)**：Windows Credential Manager 泛型凭据、macOS Keychain 泛型密码、Linux/BSD Secret Service（libsecret）；回退 `%LOCALAPPDATA%\tetherly\identity.bin` 等，ACL 仅当前用户 |
| 信任库 | `trusted.json`：`device_id, id_pk, n_pk, alias, paired_at, revoked` |
| 配置 | `config.toml` |

写入：临时文件 + `rename`。忘记设备：标 `revoked` 或删除行。**凭据库（v1.13/v1.14）**：Windows→Credential Manager 泛型凭据（`tetherly.identity:<device_id>`）、macOS→Keychain 泛型密码、Linux/BSD→Secret Service（属性 `device_id=<id>`）；数据目录只留公开的 `identity.ref` 指针，旧 `identity.bin` 首次运行自动迁移入库并删除。**仅「已安装数据目录」启用凭据库**——临时目录节点（全部集成测试）仍用本地 `identity.bin`，绝不写操作者真实凭据库（`TETHERLY_VAULT=1/0` 可覆盖）。无可用凭据库（headless Linux / 无 keyring / 非三平台）时透明回退文件。

### 6.2 状态机

```
Idle → Discovering → TcpConnected → Hello
Hello + 未知 id_pk → Pairing(SPAKE2) → PairBind → NoiseIK → 写信任库 → Active
Hello + 信任库命中且 n_pk 一致 → NoiseIK → Active
Hello + 信任库命中但 n_pk 变了 → 断开（需用户重新配对）
失败 → Backoff 1s,2s,4s,…,60s
Active 断线 → Backoff
忘记设备 → 删信任 → Idle
```

Hello 之后、Noise（或 SPAKE2）完成前 **禁止业务帧**。`device_id` 与 `id_pk` 对不上立即断开。

Hello JSON，最大 4 KiB，明文长度前缀：

```jsonc
{
  "proto": 1,
  "device_id": "tdev_...",
  "id_pk": "<base64 32B>",
  "n_pk": "<base64 32B>",   // 未配对也发，供 salt 排序；Noise IK 仍以信任库为准
  "caps": ["notify", "clip", "file"],
  "name": "Mango-PC",
  "platform": "windows"
}
```

`n_pk` 在未配对 Hello 里可见：这是 TOFU 指纹材料，真正绑定靠 SPAKE2 + 签名。

### 6.3 配对 UX

1. A「添加设备」显示 `2517 0394`（空格分组），3 分钟倒计时。
2. B 输入 8 位数字。两端可再显示 `fp` 供人工核对（非必须）。
3. 成功：写入信任库，PIN 与 `pwd` zeroize。
4. 文案：「不要把数字发给任何人，只在你自己的另一台设备上输入。」

测试向量：固定 PIN `25170394`、固定 `id_sk`/`n_sk` 种子、固定 transcript 哈希，放进 `tetherly-crypto`。

### 6.4 Noise

- crate `snow`，模式见 ADR-003。
- IK：发起者必须已有响应者 `n_sk` 对应公钥（来自信任库，不来自本次 Hello 的 `n_pk`，除非指纹一致）。
- 应用层 `msg_id`：内层帧 `u16 type | u16 flags | u64 msg_id | payload`；每会话单调；收端 LRU 防重放。
- 每 2^16 条 Noise 消息或 3600s 调用 `rekey`（若当前 `snow` 版本 TransportState 无 rekey，改为重建 IK 会话，Phase 0 写进注释）。
- Hello / SPAKE2 / PairBind / Noise 握手读超时 20s。新 TCP 每源 IP 30/min。

### 6.5 能力

Active 后可 `caps.update`。未知 cap 忽略。未声明的 type 丢弃。

---

## 7. 线协议

### 7.1 连接上的字节

```
TCP
  ├─ u32be hello_len | hello_json          // 双方各发一次
  ├─ 若未配对：SPAKE2 消息（u32be len | bytes）若干轮
  │            u32be len | PairBind_ciphertext
  └─ Noise handshake 消息（snow 默认带长度由我们再加 u32be）
     └─ 之后：u32be noise_msg_len | noise_transport_payload
              解密后内层：u16be type | u16be flags | u64be msg_id | payload
```

内层 payload 上限 `INNER_PAYLOAD_MAX`（v1.22 起由传输上限推导：Noise 单条消息 65535 − 16 字节 AEAD tag − 12 字节内层帧头 = **65507**；详见 §9.7 与「1.22 相对 1.21」）。文件块走第二条 TCP：Hello 可省略，用 `file_token` 做 Noise PSK 或把 token 放在 IK payload。`file_token = HKDF-SHA256(ck, "tetherly-file-v1" || transfer_id)`，60s 过期。文件帧上限 256 KiB。

`flags` bit0 = `MUST_UNDERSTAND`：未知 type 则断开。

### 7.2 内层 type（v1）

| 符号 | 值（建议） | 载荷 |
|---|---|---|
| ping / pong | 0x0001 / 0x0002 | `unix_ms: u64` |
| notify.push | 0x0101 | JSON：uid, app_id, app_name, title, body, ts, actions[]（无 url） |
| notify.dismiss | 0x0102 | uid, app_id |
| notify.reply | 0x0103 | JSON:uid, app_id, text(快速回复,桌面→手机;≤4 KiB,仅投递、不执行) |
| clip.set | 0x0201 | mime + text 或 blob_ref |
| file.offer | 0x0301 | transfer_id, files[{name,size,sha256}] |
| file.accept / reject | 0x0302 / 0x0303 | transfer_id |
| file.done | 0x0304 | transfer_id, sha256 |
| caps.update | 0x00F0 | caps[] |

没有 `otp.candidate`。没有 `input.*` JSON。

### 7.3 去重与限流

通知键：`(source_device_id, app_id, uid)`，LRU 10 分钟。uid 不保证单调。

| 对象 | 限制 |
|---|---|
| notify.push | 每源 2/s，突发 10，超出丢弃 |
| clip.set | 4/s |
| 新 TCP | 每 IP 30/min |
| title/body | 各 4 KiB 截断 |
| 剪贴板文本 | 1 MiB，更大走文件 |

### 7.4 文件

默认必须点确认。自动接收默认关。文件名只取 basename，拒绝 `..`。v1 无跨进程续传；同连接可重发块 → **v1.12 起**:`transfer_id` 内容寻址(`"tr_"+sha256(file)[..16]`);接收方半成品写 `inbox/.<transfer_id>.part`、块对齐偏移持久化到 `inbox/.<transfer_id>.resume.json` 侧车,Node 重启扫描侧车重建续传表;发送方按 offer 回带的 `resume_from` 仅发尾块。续传**须 64KiB 块对齐**——每块 AAD=`transfer_id||chan_offset_le`、nonce=偏移/块+1,块不可跨位置搬移;侧车与 `.part` 撕裂则丢弃并从 0 重传。断线 offer 作废。控制面 offer/accept 走 45717；数据走对端 `caps.update` 广告的 **45718**（`fileport=`）。`file_token = HKDF-SHA256(handshake_hash, "tetherly-file-v1" || transfer_id)`；块 ChaCha20-Poly1305，魔数 `TFL1`（v1）/`TFL2`（v1.12 带起始偏移），块 64KiB。

### 7.5 剪贴板

桌面：`clip_seq` + last-hash 防回环。Android：按钮「发送剪贴板」。iOS：仅前台。写入 OTP 后默认 60s 清本机剪贴板。

---

## 8. Workspace

Phase 0 四件套仍在。Phase 1 **已加**：

```
tetherly/
├── Cargo.toml
├── crates/
│   ├── tetherly-core/      # OTP、帧、状态机、白名单、去重、bind/clip/file/notify/persist
│   ├── tetherly-crypto/    # Ed25519、X25519、Argon2id、SPAKE2、Noise、file AEAD
│   ├── tetherly-net/       # TCP、hello、mDNS、内层 dispatch、filechan 45718
│   ├── tetherly-node/      # LAN 节点、落盘、hubs、本机 HTTP UI、EasyTier sidecar 发现、tetherly 二进制
│   └── tetherly-android/   # 手机原生库：真身份 + Noise 会话 → libtetherly_android.so（cdylib+rlib）+ android_probe
├── bins/tetherly-cli/      # Phase 0 配对夹具仍保留
├── tests/loopback.rs       # Phase 0
├── tests/phase1.rs         # Phase 1 loopback 双节点
├── tests/phase2.rs         # Phase 2 sidecar unicast / LAN 优先 / 缺侧车
├── tests/phase3.rs         # Phase 3 键鼠 45719
├── tests/phase4.rs         # Phase 4 ANCS 编解码与状态机（含 m4_5 入站泵端到端）
├── ui/index.html           # 本机 UI（loopback HTTP）
├── android/                # NLService + gradle CI
└── docs/
```

其后按阶段加：`tetherly-ffi`、`src-tauri`、`ios/`。`android/` 已存在且 JNI `.so` 已由 `tetherly-android`（v1.10）构建。ANCS 与键鼠引擎都放在 `tetherly-core`（`ancs` / `input`），不开新 crate。

红线：`tetherly-core` 无 `cfg(target_os)`、无 `windows`/`objc`/`jni`。`tetherly-net` 无 `easytier` crate。`tetherly-android` 的 `jni` 依赖锁在 `cfg(target_os = "android")`，桌面构建不引入；JNI `unsafe` 仅豁免 `jni.rs` 一处。

### 8.1 Phase 0 依赖冻结（名称，版本在 lock 时钉）

| crate | 用途 |
|---|---|
| `snow` | Noise |
| `spake2` | PAKE |
| `argon2` | PIN KDF |
| `ed25519-dalek` | 身份签名 |
| `x25519-dalek` | Noise 静态/临时 |
| `sha2` / `hkdf` / `zeroize` | 派生与擦除 |
| `mdns-sd` | LAN 发现 |
| `tokio` `serde` `thiserror` `tracing` | 运行时 |

升级大版本视为协议风险，要重新跑测试向量。

### 8.2 核心类型

```rust
pub struct DeviceId(/* tdev_ + base32 */);

pub struct PhoneNotification {
    pub source: DeviceId,
    pub app_id: String,
    pub app_name: String,
    pub uid: String,
    pub title: String,
    pub body: String,
    pub received_at: SystemTime,
    pub actions: Vec<ActionId>, // Copy | Dismiss | OpenApp，无 URL
}

pub const DEFAULT_MIN_SCORE: i32 = 5;

pub trait OtpExtractor: Send + Sync {
    fn extract(&self, text: &str) -> Option<String>; // 内部仍有 score，对外过阈值才 Some
}
```

OTP 语义冻结为 sms-pop-rs：4–8 位连续数字；关键词前 48 / 后 24 字节；11 位电话整段跳过；1900–2099 四位年剔除；宁缺毋错。先移植测试，不准「改进」规则。

平台 trait（core 定义，平台实现）：`Clock` `Rng` `TrustStore` `Insertor` `Clip` `Notifier`。

---

## 9. 功能模块

### 9.1 NotifyHub

入口：ANCS、Android、loopback。截断、去重、限流、本地 extract、UI、按 caps 转发 `notify.push`。默认只转给桌面，不回推手机。

### 9.2 填入

`insertion::plan(current, code)`：空则整写；前缀则补差；已完整则不写；只读则拒绝。Simulate 必须拿着探测到的 element 引用，禁止对「当前前台」盲打。

### 9.3 打开白名单

```toml
[[open_rules]]
app_id = "com.tencent.xin"
url = "weixin://"
```

### 9.4 ANCS（Phase 4，电脑侧）

与 sms-pop-rs 相同的硬约束：先订 Data Source 再订 Notification Source；禁止在 ValueChanged 线程写 Control Point；Control Point 串行；分片按字节拼满；首次属性延迟重试；断线退避；uid 去重。

Windows：`windows` crate GATT。macOS：CoreBluetooth。Linux：BlueZ，不要把电脑广告成音箱。

用户路径：系统蓝牙设置里让 iPhone 连电脑（ANCS），**再**在 Tetherly 里把这台电脑与 Android/其他电脑 SPAKE2 配对。两步都要在 UI 向导里拆开写。

**v1.23 已落地 Windows 真机传输栈**：`tetherly-node::ancs::win_gatt`（自包含、`#[cfg(windows)]`，照 `screen/macos.rs` 先例）——专属 `tetherly-ancs` 线程 + **MTA 公寓**持有全部 WinRT 接口；`FromBluetoothAddressAsync` → `GetGattServicesForUuidAsync(ANCS)` → `GetCharacteristicsAsync` → 按 UUID 找三特征并**校验属性**（Notification Source / Data Source 必须 `Notify`、Control Point 必须 `Write`，缺一点名报错）；CCCD 写 `Notify` 订阅；`ValueChanged` 回调**只把字节拷进通道**（§9.4 禁止在回调线程写 Control Point）；`write_control_point` 经 `DataWriter` → `WriteValueWithResultAsync` 并核对 `GattWriteResult::Status`，**阻塞到结果为真**（「串行且只有一条在途」成为事实）；前端每次命令的 ack 等待设 **10s 上界**，超时即报 `AncsSubscribe`/`AncsWrite`（不谎报成功）；`ConnectionStatusChanged` 置标志 → 退出会话 → `ancs_backoff_delay` 指数退避（1s→30s）；`Session::drop` 显式 `RemoveValueChanged`/`RemoveConnectionStatusChanged`。**同步的三个端口方法直接用 `windows` 0.59 的阻塞 `IAsyncOperation::get()`**（OS `Waiter` 信号，不空转），故**无新依赖、无自造 executor**。node 侧 `Node::spawn_ancs_pump`（独占线程 + current-thread 运行时）把 `AncsInbound`（`Ready`/`NotificationSource`/`DataSource`/`Disconnected`）喂给 `ancs_connect`/`ancs_notification_source`/`ancs_data_source`/`ancs_disconnected`，并每 500ms `ancs_tick`；CLI `tetherly run --ancs <11:22:33:44:55:66>`。macOS（CoreBluetooth）与 Linux（BlueZ）栈**未实现**，`open()` 诚实返回 `Ok(None)`。物理 iPhone 的运行时 **Manual-required**。

**v1.6 已落地（CI 切片）**：`tetherly-core::ancs` 协议编解码 + `AncsIngress` 状态机 + `AncsTransport` 端口 + `MemoryAncsTransport`。硬约束按构造强制：订阅顺序由 `connect()` 固定；`on_notification_source` 只入队，Control Point 只在 `tick` 写；`in_flight` 保证串行；`DataAssembler` 只在整条属性元组到齐后才产出；静默一次后重试、再静默则放弃；`on_disconnected` 走 500ms→30s 退避。真实 GATT 栈与物理 iPhone 仍 Manual-required。

### 9.5 键鼠（Phase 3）

拓扑在 `config.toml`。断线光标回 server。Wayland 走 portal+libei，不行就标不支持。DeskFlow 网关另开端口、默认关、TLS 未实现就不要宣称兼容。

**TIN1 语义（v1.9 明确）**：`Move` 的 x/y 是 `SCREEN_UNITS=1000` 千分比归一化（避免浮点过线），受控端映射回本地屏幕；`Button` 的 mask 位为 `BTN_LEFT=1 / BTN_RIGHT=2 / BTN_MIDDLE=4`；`Wheel` 的 dx/dy 为 i16 delta；`Key` 的 code 为 **Win32 虚拟键码（VK）**（u16），`mods` 为信息性字段——修饰键以独立 key 事件传输。

**v1.5 已落地**：`tetherly-core::input` 编解码与边缘状态机；`tetherly-node` 在 LAN 与 overlay 上另听 45719，会话 `resume_only`。CI 注入是 `MemorySink`，不调用 Win32 `SendInput`。进入帧落在 `x=0` 时保持焦点；离开只在已经进入内部后再回到左缘时发生。剪贴板仍走 45717，不进键鼠帧。

**v1.9 真机链已落地（Windows）**：受控端 `SystemInputSink`（`InputSink` 端口实现；SendInput 绝对移动/按钮/滚轮/VK 键盘，`NodeConfig.input_sink` 注入，CI 缺省 `None` 仅观察者）——收到的事件在协议级过滤（seq 窗口/座位门控）后经 `MemorySink` 观察者 trace 镜像到真实注入。主控端 `Node::spawn_input_capture`：`WH_MOUSE_LL` 恒挂（只读光标做过边检测）、`WH_KEYBOARD_LL` **仅在远程座位时挂**（本地键盘隐私）、注入事件按 `LLMHF/LLKHF_INJECTED` 过滤防单机回环；事件喂与 CI 相同的 `drive_input*` 路径。**注入探针实测**：过边后受控端光标真实移至左缘 (0,539)、中心移动精确、Win/Esc 键盘注入生效。双桌面 100 次物理往返仍 Manual-required。

### 9.6 Android（Phase 1）

`NotificationListenerService` + 前台服务。Android 13+ `POST_NOTIFICATIONS`。帮助页教各 ROM 自启动，不搞黑保活。不读短信库。Play 上架需通知使用权声明，内部测试可先 sideload。

**原生库（v1.10 已落地）**：`crates/tetherly-android` 产出 `libtetherly_android.so`（arm64-v8a，cargo-ndk）。身份为**真 Ed25519 + X25519**（`Identity::from_secrets`，`device_id = DeviceId::from_id_pk`），`identity.bin`/`trusted.json` 与桌面节点同构、可互换。会话走与桌面**相同**的 `dial_session`（首配 Hello + SPAKE2 8 位 PIN，之后 Noise IK resume），手机仅拨 45717、不做 listener。**手机能力集固定 `["notify","clip","file"]`，永不广告 `input`/`screen`**。Kotlin 侧 `NativeBridge`/`NodeSession` 经 JNI 薄壳（`Java_app_tetherly_NativeBridge_*`）对接，拿到真实 `device_id` 并驱动 `notify.push`。**v1.3 的 `idPk = sha256(idSk)` 占位已删除**；无 `.so` 时 `state()==0` 且 `identity=="unavailable"`，**禁止**自己拼 SPAKE2/Noise TCP。

**已验（真机 v1.10）**：两台 Galaxy Fold（SM-F900F / arm64-v8a / Android 13）分别以 `android_probe` 完成真实配对 + notify 投递（桌面 `has_otp:true`）+ 换错 PIN 仍 resume 成功。CI 跑 `cargo ndk` 出 `.so` 后再 `gradle :app:testDebugUnitTest` + `assembleDebug`（`jniLibs/` 是产物，不入库）。协议与 OTP 路径另由桌面双节点 loopback 覆盖(物理机 M1.1 p95 由 v1.18 真机实测关闭,见下段;8h soak 仍 Manual-required)。

**APK UI 全链路已验(v1.18 真机)**:真机 `app.tetherly`(已装 APK,前台 `NotificationListenerService`)→ JNI → `libtetherly_android.so` 实时会话闭合;真机侧推送测试通知经桌面节点提取为候选 `{app_id:"com.android.shell", has_otp:true, source:"tdev_…"}`(正文/验证码不入快照/日志)。**M1.1 p95 上机实测**:手机→桌面通知往返 `rtt_p50≈17.9ms / p95≈56.9ms / p99≈361.8ms`(云手机经 `adb reverse` 隧道,非 LAN;LAN 同机型约 15–16ms),p95 在 <1.0s 预算内。**Manual-required(收窄)**:8h soak 与 LAN 直连双物理桌面往返;`.so` 缺失、APK 链路、物理 Android M1.1 p95 计量三项阻塞已消除。

**快速回复(v1.11)**:桌面可对手机通知就地回复——notify.reply(0x0103,JSON uid/app_id/text)。NotifyListener.drainReplies() 把收到的回复交给通知自身的 RemoteInput action(actionIntent.send)执行,不合成应用未声明的回复;activeNotifications 无匹配则丢弃。文本为用户内容,不入日志。协议与收件箱由 CI 覆盖(phase2::m2_6 + client_e2e::android_client_receives_a_quick_reply_once);真机触发 RemoteInput(需系统权限/前台通知)Manual-required。

### 9.7 远程屏幕（Phase 5）

**协议**：`tetherly-core::screen`（`crates/tetherly-core/src/screen.rs`）。帧魔数 `TMV1`（`SCREEN_MAGIC`），proto 1，seq u64be 单调递增窗口 `SCREEN_SEQ_WINDOW=65536`，trace 上限 64。单帧 header 31 字节：`TMV1`(4) | proto(1) | seq(8) | x(4) | y(4) | w(2) | h(2) | fmt(1) | compress(1) | stride(4) | pixels。控制消息 14 字节：`TMV1`(4) | proto(1) | tag(1) | seq(8)；tag 取值 Start=0 / Stop=1 / Ack=2。像素格式 `PixelFormat{Bgra8,Rgba8}`，默认 BGRA（一致 Win32）。传输走独立 TCP 端口 **45720**（`SCREEN_PORT`），`resume_only`：只在已配对对端间建会话；`screen_only: true` 的会话在 `HelloDisposition::Pair` 时返回 `NetError::ScreenRequiresTrust`。Hello cap 广告 `"screen"` 与 `screenport=N`。**分片预算(v1.22 修正)**:超内层预算的整帧拆成 `TMF1` 分片(header 24 字节),每片 payload `SCREEN_FRAG_HEADER + SCREEN_FRAG_CHUNK_MAX` 必须 `<= INNER_PAYLOAD_MAX`;`INNER_PAYLOAD_MAX` 由传输上限推导(= `65535 − 16 − 12 = 65507`),故分片 payload 恰为 65507、编码后恰为可加密上限 65519。此前 `INNER_PAYLOAD_MAX` 写死 64 KiB 时,分片 payload 65536、编码后 65548 明文**无法加密**,是「分片屏幕帧发不出去」的潜伏缺陷根因(见「1.22 相对 1.21」)。

**门控模型**：`ScreenState` 四态状态机：`Idle → Requested → Allowed → Streaming`。对端 dial 45720 成功仅置 `Requested`，不开始推流。必须本机显式调用 `Node::screen_allow()` 才进 `Allowed`。对端发 `ControlMsg::Start` 进入 `Streaming` 后才出帧；在 `Allowed` 之前收到的 `Start` 被拒绝并计数（`refused`）。`Node::screen_revoke()` 回到 `Idle`。对端断开（`on_peer_gone`）强制回 `Idle` 并清零 seq。重连不重放旧帧——接收方 seq 窗口丢弃 seq < 已收最大 seq 的帧。

**采集**：`ScreenSource` 端口在 `tetherly-core` 声明，`tetherly-node` 注入实现。CI 用 `MemoryScreenSource`（确定性帧序列）。Windows 真机（v1.8 已落地）：`DxgiSource`（`windows` crate DXGI Desktop Duplication）——`D3D11CreateDevice(BGRA)` → `IDXGIDevice::GetAdapter` → `IDXGIOutput1::DuplicateOutput`，`AcquireNextFrame(0)` 遇到 `DXGI_ERROR_WAIT_TIMEOUT` 返回 `Ok(None)`（无新帧），staging 纹理 + `Map(READ)` + 行尾裁剪产出紧凑 BGRA8 帧；`SystemSource` 按目标选择采集器（Windows=`DxgiSource`；macOS=`MacSource`；X11=`X11Source`）。另有 `AdbSource`（v1.8）：`adb exec-out screencap -p` 抓取 adb 连接的 Android 设备（云手机场景），PNG 解码 → 紧凑 BGRA8；PNG 字节与上帧相同则 `Ok(None)`（DXGI 同款空闲语义）；`tetherly run --screen-adb <serial>` 选用，`TETHERLY_ADB` 指定 adb 路径。**v1.19 落地**:macOS `MacSource`(`core-graphics` `CGDisplayCreateImage` → 显式 BGRA8 `CGBitmapContext`,读回框架实际 stride 去行尾填充)与 Linux/BSD X11 `X11Source`(`x11rb`;MIT-SHM 快路径 + plain `GetImage` 回退,含 `shm_query_version >= 1.2` 门与按 `reply.size` 精确读取)。两者按 `cred.rs` 先例各为**自包含文件**(只 import 绝对路径),`Cargo.toml` 加目标依赖 macOS `objc2-core-foundation`/`objc2-core-graphics`(`default-features = false`)、Linux `x11rb`(shm)/`libc`,**不引入新包**(均随 `arboard` 已在 lockfile)。真机采集需真实显示器(macOS 需屏幕录制权限,X11 需 X server),**Manual-required**;Wayland 仍标「不支持」(纯 Wayland 无 X11 根窗口可读,portal 路线属下一迭代)。

**压缩**：生产默认链为 `EncodedSource<SystemSource>`——`ZstdCodec`（无损，level 3）实现 `ScreenCodec` 端口，每帧压缩，压不小就发原帧（病态编码器不可能放大传输）；接入侧按 `raw_len` 校验 decompress 后长度。未来 OpenH264（BSD-3-Clause）/ rav1e（BSD-2-Clause/ISC）作为同一 `ScreenCodec` 端口替换——「**先无损帧 + 后接编码器**」的落点。不走 GPL/AGPL（x264/x265 明确出局）。

**呈现**：`ScreenSink` 端口在 `tetherly-core` 声明。CI 用 `MemoryScreenSink`（断言 seq 单调）。Windows 真机（v1.8 已落地）：`SystemSink` → GDI 窗口——`RegisterClassW`/`CreateWindowExW` + `CreateDIBSection`（32bpp top-down）+ 同步 `BitBlt`（此线程无消息泵，不能依赖 `WM_PAINT`，直接画到窗口 DC 立即显示）。**v1.19 落地 X11 呈现**:`SystemSink` → `x11rb` override-redirect 窗口 + `PutImage`(窗口深度经 `COPY_DEPTH_FROM_PARENT` 继承自根,`PutImage` 用**读回的根深度**而非硬编码 24,否则 32 位根上回 `BadMatch`)。macOS 呈现需 AppKit `NSWindow`(CoreGraphics 位图上下文无法触及屏幕),`present` 返回点名 `NSWindow` 的诚实 `ScreenRefused`;Wayland portal 与其余非 Windows/macOS/X11 目标同样诚实 `ScreenRefused`。

**RustDesk 借鉴范围**：架构借鉴仅限分离采集+编码器 swap+P2P 模式——这些都是公开工程常识，不构成知识产权。RustDesk 客户端与服务端均为 AGPL-3.0，`deny.toml` 已 ban `hbb_common` / `rustdesk` / `rustdesk-server` / `scrap`。不借鉴 RustDesk 的居会合（hbbs/hbbr）服务器——匹配 §2.3 禁云账号。

**v1.7 落地范围**：`tetherly-core::screen` 帧+控制编解码、`ScreenSender`/`ScreenReceiver`、强制本机同意 M5.2、45720 独立端口 + `ScreenRequiresTrust` 门、`tetherly-node` 14 处锚点（`bound_screen` / `screen_out` / `screen_engine` / `screen_sink` / `screen_ctl` / `spawn_listeners` / `spawn_overlay` / caps / `LivePeer.screen_port` / `open_screen` / `accept_screen` / `attach_screen_server` / `attach_screen_client` / `screen_send_frame`）、UI 路由 `/api/screen/allow|start|stop` + 前端「远程屏幕（实验）」区、`tests/phase5.rs` 4 个绿测试。真机采集 / zstd 编码 / BitBlt 呈现 / p95 计量其后由 **v1.8 真机闭环**落地(见 §12)。**v1.19 补齐 macOS/Linux 采集**（见本节「采集」与「呈现」段）：开放编码器（OpenH264/rav1e）与 macOS 呈现（NSWindow）仍 Manual-required。M5.5 硬件编码 / 真机注入「未做」。

---

## 10. 模式与反模式

用：Transport 策略、显式 `ConnState`、ANCS 串行队列、插件注册表、TOFU、指数退避。

禁止：core 里 `target_os`；自动填码；日志里的码/正文/剪贴板；执行对端 URL；JSON 鼠标坐标；BLE RPC；iOS VPN entitlement；虚网 mDNS 当唯一发现；把 EasyTier 链进二进制。

---

## 11. 安全工程

- CI：`cargo deny`（license + bans）+ `cargo audit`
- lockfile 入库
- Tauri updater 公钥内置；更新不上传通知
- SECURITY.md：禁止公开 issue 贴验证码
- `TETHERLY_INSECURE_LOG=1` 仅 debug profile，启动警告

---

## 12. 阶段与里程碑

未勾选退出标准不得开下一阶段功能代码（文档/UI 草图可以并行）。工期是上限。

### Phase 0 — 协议骨架（约 2 周）

四个 crate + CLI。无 GUI、无 BLE、无 EasyTier。

| ID | 验收 | 测法 |
|---|---|---|
| M0.1 | OTP 移植用例全绿 | `cargo test -p tetherly-core` |
| M0.2 | 双进程 SPAKE2 成功；错 PIN 5 次锁定 | CLI |
| M0.3 | 配对后 ping；杀一端退避重连 | CLI |
| M0.4 | 夹具改 Hello `id_pk`：SPAKE2 失败；线路上改 Hello `name`：Noise 失败。均不写信任库 | 测试 |
| M0.5 | `device_id` 与 `id_pk` 不一致则断开 | 单测 |
| M0.6 | 固定种子测试向量（Argon2+SPAKE2+Noise prologue） | `tetherly-crypto` |
| M0.7 | CI：fmt、clippy `-D warnings`、test、deny、audit | Actions |

退出：M0.1–M0.7。许可证调查已完成（本 ADR-006），不再作为退出条件。

### Phase 1 — LAN 桌面 + Android（约 4 周）

| ID | 验收 | 状态（v1.3） |
|---|---|---|
| M1.1 | Android 测试通知含 `524681` 到桌面弹窗，**p95 < 1.0s**（listener 回调 → 窗口可见） | loopback `notify.push` CI 绿、耗时 < 1s。**真机链已落地（v1.10）**：`libtetherly_android.so`（真 Ed25519 + Noise IK）+ `android_probe` 在两台 Galaxy Fold 完成配对 + notify 投递（桌面 `has_otp:true`）。物理 Android p95 计量 / 8h soak：**Manual-required**。macOS M1.1：**mac 下一迭代** |
| M1.1(v1.18 更新) | 同上 | **APK UI 全链路真机已验**(云手机 `app.tetherly` 前台 NLService → JNI → 实时 Noise 会话 → 桌面候选 `has_otp:true`,正文不入日志)。**p95 上机实测**:往返 `p50≈17.9ms / p95≈56.9ms / p99≈361.8ms`(经 `adb reverse` 云手机隧道;LAN 同机型约 15–16ms),p95 在 <1.0s 预算内。8h soak + LAN 直连双桌面往返仍 Manual-required |
| M1.2 | 复制后剪贴板为该码；120s 候选消失 | CI 绿（`tests/phase1.rs` + ManualClock） |
| M1.3 | 记事本可填；只读拒绝 | CI 绿（`MemoryInsertor` 空写/只读拒绝）。真记事本 UIA：**Manual-required** |
| M1.4 | Win↔mac 文本剪贴板（有双桌面时）；至少 Win 本机回环 | Win 双节点 loopback CI 绿。Win↔mac：**Manual-required** |
| M1.5 | 杀 Android 进程后自动重连，不重新配对 | persist trust + 无 PIN 重拨 CI 绿。**真机已验（v1.10）**：`android_probe` 二次运行以**错误 PIN** 仍连上（走 Noise IK resume 而非配对），身份持久化于 `identity.bin`。真杀 Android 进程：**Manual-required** |
| M1.6 | 运行日志 `grep 524681` 无命中 | CI 绿（tracing MakeWriter 捕获） |
| M1.7 | 未确认不收文件：确认后 10MB sha256 一致 | CI 绿（reject 无 inbox 文件；accept 10MiB sha256）。**v1.24**：声明超过 `FILE_MAX` 的报价在**用户看到之前**即被拒（`FileTooLarge`，候选不出现）且**不拆会话**，随后一笔正常传输仍成功 |
| M1.8 | 中断后**跨进程**续传：重启接收方，凭持久偏移只补发尾块，最终 sha256 一致 | **CI 绿（v1.12）**：`tests/phase1.rs::m1_8_file_resumes_from_persisted_offset`（预置 `.part`+侧车后重启 B → offer 回带 `resume_from==2*64KiB` → 接受后 sha256 一致 → 侧车清除）；无侧车则从 0 重传 |

退出：M1.1–M1.3、M1.5–M1.8。Win+Android 8h 无崩溃。macOS 能跑通 M1.1 或文档标明「mac 下一迭代」。

**v1.3 退出裁定（历史）**：CI 切片（协议、落盘、日志脱敏、文件确认、本机 UI、Android 工程骨架）已绿。物理 Android p95 / 8h / JNI `.so` / 真 UIA / Win↔mac 仍是 Manual-required。后续以用户「继续按文档开发」为豁免，进入 Phase 2 CI 切片。

### Phase 2 — EasyTier 侧车（约 3 周）

| ID | 验收 | 状态（v1.4） |
|---|---|---|
| M2.1 | Android 蜂窝网 ↔ 电脑，通知 p95 < 3s | **Manual-required**（真机蜂窝 + 用户 EasyTier soak） |
| M2.2 | 同 Wi-Fi 走 LAN 接口；断 Wi-Fi 后 **新通知** 走虚网。进行中文件允许失败并提示 | CI：已有 LAN 时丢弃 overlay attach，重复扫描仍保一条 LAN。真机断 Wi-Fi 切虚网：**Manual-required** |
| M2.3 | 停 EasyTier：UI「仅局域网」，不崩 | CI 绿（无 RPC/CLI/TUN → `lan_only`，shutdown 不崩） |
| M2.4 | 未装 EasyTier 时 Phase 1 不变 | CI 绿（隔离 CIDR 下仍可配对 + `notify.push`） |
| M2.5 | 虚网接口上关闭 mDNS 仍能靠 peer 列表连上 | CI 绿（`advertise=false` + `extra_peers` 单播 45717） |

**v1.4 退出裁定**：Phase 2 CI 切片（侧车探测、单播、LAN 优先、缺侧车仅局域网、UI 文案）已绿。M2.1 物理蜂窝 p95 与真机断 Wi-Fi 切路径仍是 Manual-required，**不得开 Phase 3 功能代码**，除非另下豁免。

### Phase 3 — 桌面键鼠（约 4 周）

| ID | 验收 | 状态（v1.5） |
|---|---|---|
| M3.1 | 光标过 Win 右缘到第二台桌面，键盘跟随；脚本往返 100 次丢失 0 | CI：45719 过边 + 按键进 `MemorySink`；core 100 次往返 0 丢失。**真机链已落地（v1.9）**：受控端 `SystemInputSink`（SendInput：1000 单位→65535 绝对坐标、按钮/滚轮/VK 键盘）+ 主控端 `spawn_input_capture`（WH_MOUSE_LL 恒挂过边检测、WH_KEYBOARD_LL 仅远程座位时挂、注入事件按 LLMHF/LLKHF_INJECTED 过滤防回环）；注入探针实测光标过边到左缘 (0,539) 与中心移动精确。**双桌面 100 次物理往返：仍 Manual-required** |
| M3.2 | 对端断开，光标回本机 | CI 绿（输入会话结束 `on_peer_gone`，座位回 Local） |
| M3.3 | 键鼠模式下剪贴板仍可用 | CI 绿（焦点在对端时 `clip.set` 仍走 45717） |
| M3.4 | 加分：DeskFlow 官方 client + TLS 能移动光标。失败则网关标 experimental | **未做**。网关保持关闭，不链 GPL 源码，不挡退出 |

Wayland 不挡退出。真机注入未落地前不得宣称可替代键鼠。

**v1.5 退出裁定**：Phase 3 CI 切片（resume-only Noise、二进制帧、过边、断线、剪贴板并存）已绿。物理双桌面与 OS 注入仍是 Manual-required。

**豁免记录（2026-10-10）**：上述「不得开 Phase 4」的闸门已由项目方豁免，Phase 4 ANCS CI 切片据此落地（见 v1.6）。物理双桌面与 OS 注入的 Manual-required 状态**不变**。

### Phase 4 — iPhone ANCS（约 4 周）

| ID | 验收 | 状态 |
|---|---|---|
| M4.1 | 系统蓝牙连接后短信通知 p95 < 2s 到 Win 或 mac | **CI 代理**：`tests/phase4.rs` 用注入时钟断言端到端预算 < 2s，且 app 名等待封顶 400ms。真机 p95 = Manual-required |
| M4.2 | 仅白名单显示「打开」；payload 带 url 也不打开 | **CI 绿**：`ancs_actions` 门控；`open_candidate` 只走白名单 url；载荷 url 从不读取 |
| M4.3 | 关蓝牙 30s 再开：重连且不连弹旧通知 | **CI 绿**：`PreExisting` 丢弃 + uid 去重；退避拒绝热重连 |
| M4.4 | 可选 iOS App：同 LAN 50MB sha256；无此 App 不挡退出 | 未做（不挡退出） |
| M4.5 | iOS App 无 VPN / Network Extension | 未做（随 M4.4） |

**v1.6 落地范围**：`tetherly-core::ancs`（Notification Source 解析、Control Point 编码、Data Source 按字节重组、`AncsIngress`）、`tetherly-node` 的 `ancs_*` 入口与 `Opener` 端口、UI 两步向导、`tests/phase4.rs`。**Manual-required**：真实 BLE GATT（Windows `windows` / CoreBluetooth / BlueZ）、物理 iPhone p95、蓝牙关 30s 真机复现。

**v1.23 落地范围（补上电脑侧那段真机传输栈）**：`tetherly-node::ancs`（装配点 + `AncsInbound` 通道）+ `tetherly-node::ancs::win_gatt`（Windows WinRT BLE GATT，自包含 `#[cfg(windows)]`）；`Node::spawn_ancs_pump` 把入站值接到 `ancs_*` 入口并驱动 `ancs_tick`；CLI `--ancs <addr>`。**Manual-required 不变**：macOS（CoreBluetooth）/ Linux（BlueZ）栈**仍未实现**（`open()` 诚实 `Ok(None)`），物理 iPhone 的运行时行为需真机验证。§9.4 硬约束在真机栈中逐条落实（订阅顺序、回调不写 Control Point、Control Point 串行 + 10s ack 上界、字节拼接、退避、uid 去重）。

### Phase 5 — 远程桌面（屏幕串流）（约 4 周）

| ID | 验收 | 状态（v1.8） |
|---|---|---|
| M5.1 | 已配对桌面间单向看屏，p95 帧间隔在预算内 | **已实现（真机闭环）**：Windows DXGI Desktop Duplication + zstd 无损 + GDI BitBlt；p95 帧间隔计量（`ScreenIntervalTracker`）+ UI 暴露。**跨设备真机已验（v1.8 AdbSource）**：云手机（Galaxy Fold 720x1280）→ Windows 双节点，1063 帧发出 / 1060 帧收达 / 0 丢帧，GDI 窗口画面与真机屏 ±8 容差 97.6% 一致（残差为动画相位 + DWM 色彩管理），p95 帧间隔 ≈0.9–1.45s（`adb screencap` 源速率主导，诚实计量）。**DXGI 采集端真实像素捕获已验（v1.9）**：注入制造的真实桌面变化（Win 键开/关开始菜单）被 DXGI 捕获为 1920x1080 真实帧（8,294,400 字节 = w*h*4 精确）。跨桌面 p95 数据仍 Manual-required。**macOS / Linux 已实现（v1.19）**:`MacSource`（CoreGraphics 采集）与 `X11Source`（MIT-SHM 采集 + `PutImage` 呈现）按目标编译验证；真机采集仍 Manual-required |
| M5.2 | **未本机点击允许，绝不推流**；载荷不含可执行指令 | **CI 绿**：`ScreenState` 门控；无 `allow` 时 `Start` 被拒并计数 |
| M5.3 | 对端断开 → 状态回 `Idle`，seq 清零，重连不重放旧帧 | **CI 绿**：`on_peer_gone` + seq 窗口（tests/phase5.rs） |
| M5.4 | 45720 拒绝配对；配对只在 45717 | **CI 绿**：`resume_only` 门（`ScreenRequiresTrust`） |
| M5.5 | 加分：硬件编码 / 真机注入 | **未做**，不挡退出。OpenH264/rav1e 端口已就位 |

**v1.7 落地范围/豁免（2026-10-10）**：远程桌面原为 §2.3 v1 非目标，本项目方要求引入 Phase 5 并同意**先无损帧 + 后接编码器**（编码器作为 `ScreenSource` 端口实现后接），构成 Phase 5 开工豁免（照 Phase 4 豁免样式）。本次交付为 CI 切片：`tetherly-core::screen` 帧+控制编解码、强制本机同意 M5.2、45720 独立端口 + `ScreenRequiresTrust` 门、`tetherly-node` 屏幕锚点、UI「远程屏幕（实验）」、`tests/phase5.rs`。RustDesk 仅作架构借鉴，AGPL 不可链，`deny.toml` 已 ban。**Manual-required 不变**：真机 DXGI 采集 / 编码器 / 呈现 / p95 真机验证。M5.5 未做，不挡退出。

**v1.8 真机闭环豁免续（2026-10-11）**：Phase 5 真机闭环落地——`DxgiSource`（DXGI Desktop Duplication 采集，`AcquireNextFrame` WAIT_TIMEOUT→`Ok(None)`）、`ZstdCodec` + `EncodedSource`（无损编码可换端口）、`SystemSink`（GDI BitBlt 同步呈现）、`TMF1` 分片 + `ScreenAssembler` 重组、接入侧 decompress 链、p95 计量。`tetherly run` 注入 `SystemSink` + 16ms 帧泵（非 Streaming 跳过 grab）。本机验证：DXGI 设备/adaptor/output/duplication 初始化成功、WAIT_TIMEOUT→None 空闲路径正常、GDI 呈现成功（60/60）。**AdbSource 真机 E2E（同日）**：云手机（Galaxy Fold 720x1280）屏幕经 `--screen-adb` 全链路推流至第二 Windows 节点 GDI 窗口——1063 帧 / 1060 收达 / 0 丢帧、每帧解压后精确 720x1280x4 BGRA、像素 ±8 容差 97.6% 一致、p95 ≈0.9–1.45s（源速率主导）。**UI 两处修复**：`/api/screen/start` 自动拨号 screen 通道（此前 visitor 从不调 `open_screen`，真机 Start 必失败）；visitor 接收计数（`screen_rx_stats`）接入 Snapshot（此前 B 端 UI 永远显示 0）。 同日性能专项：`ZstdCodec` 持久 bulk 上下文（1080p 压缩 16.1→12.6ms / 解压 4.7→4.1ms）、`ScreenAssembler` 偏移式重组（3.0→1.28ms）、`SystemSink` 单拷贝直写 DIB、`[profile.release]` lto=thin + codegen-units=1、`AdbSource` 持久长度前缀 PNG 流（每帧省 adb spawn，grab 543→436ms；带宽实测 overlay ~187KB/s ⇒ raw 3.7MB/帧=17.5s 不可用、PNG 31KB/帧=0.17s，故 PNG 为默认、`TETHERLY_ADB_RAW=1` 走 raw 仅限高速链路）；基准工具 `cargo run -p tetherly-node --example bench_screen --release`。注意：dedup 生效后静止屏幕的 p95 帧间隔反映的是**内容变化周期**而非传输成本。**Manual-required（收窄）**：DXGI 采集端实际像素捕获（需交互桌面）、Windows DXGI 跨桌面 p95、真实开放编码器 OpenH264/rav1e、macOS/Linux 实现。Wayland 仍标「不支持」。

**v1.19 macOS/Linux 采集落地(2026-10-15)**:补掉 §9.7 与 §14 长期标为「下一迭代」的macOS/Linux 采集缺口。`crates/tetherly-node/src/screen/macos.rs`(`MacSource`,CoreGraphics `CGDisplayCreateImage` → 显式 BGRA8 `CGBitmapContext`,请求 16 字节对齐 stride 并按框架实际 stride 去行尾填充；屏幕录制权限被拒→诚实 `ScreenRefused`而非空白帧；呈现需 AppKit `NSWindow`,因此诚实拒绝）与 `crates/tetherly-node/src/screen/x11.rs`(`X11Source`,MIT-SHM 快路径 `shm_create_segment`+`shm_get_image` + plain `GetImage` 回退,要求 ≥ 1.2 并按 `reply.size` 精确读取；呈现为 override-redirect 窗口 + `PutImage`，depth 取自根而非硬编码)。两者按 `cred.rs` 先例各为**自包含文件**（只 import 绝对路径），`Cargo.toml` 加目标依赖（macOS `objc2-core-*`(default-features=false)；Linux `x11rb`(shm)+`libc`），**未引入新包**（均随 `arboard` 已在 lockfile）。**验证**：两后端在本机向 `x86_64-apple-darwin`/`x86_64-unknown-linux-gnu` 以探针 crate（含整个 `screen.rs` + 桩 `zstd`）完成 `clippy -D warnings` 类型检查，探针用后即删；7+7 条后端单元测试为纯逻辑、不触显示器，由 CI 在 `ubuntu-latest`/`macos-latest` 原生执行。**Manual-required（不变）**：真机采集（macOS 需屏幕录制权限、X11 需 X server）、macOS 呈现（NSWindow）、Wayland portal、开放编码器（OpenH264/rav1e）、硬件编码。本次不宣称 macOS/Linux 真机远程桌面可用。

**v1.9 键鼠真机闭环豁免续（2026-10-11）**：Phase 3 真机链落地——`SystemInputSink`（SendInput 注入端口）+ `spawn_input_capture`（LL 钩子捕获，键盘 hook 座位门控）+ 受控端 trace 镜像转发 + `TIN1` 语义明确（VK 码/1000 单位/mask 位）。注入探针在本机实测：光标过边注入、中心移动、键盘 Win/Esc 注入全部生效，并以注入制造的真实桌面变化完成了 DXGI 像素捕获验证（M5.1 收窄）。`Cargo.toml` 新增 `Win32_UI_Input_KeyboardAndMouse` feature。**Manual-required（收窄）**：双桌面 100 次物理往返（M3.1 device）、macOS/Linux 注入与捕获（CGEvent/uinput/portal+libei）、跨桌面 p95。

**v1.10 Android 原生库落地（2026-10-11）**：补齐 Phase 1 的 §9.6 诚实缺口——新增 `crates/tetherly-android`（`cdylib`+`rlib`）产出 `libtetherly_android.so`（arm64-v8a）：**真 Ed25519 + X25519** 身份（`Identity::from_secrets`，取代 v1.3 `idPk=sha256(idSk)` 占位）、与桌面同构的 `identity.bin`/`trusted.json`、桌面**同一条** `dial_session`（SPAKE2 首配 + Noise IK resume）、`SessionConfig.caps` 显式广告 `["notify","clip","file"]`（永不 `input`/`screen`）、JNI 薄壳（`unsafe` 仅豁免 `jni.rs`）。**真机已验**：`android_probe` 推到两台 Galaxy Fold（SM-F900F / arm64-v8a / Android 13），经 `adb reverse` 打隧道到宿主桌面节点（45717）——首跑真实配对、双方 `trusted.json` 落盘、桌面候选 `has_otp:true`（正文不入快照/日志）；二次以**错误 PIN** 仍连上（证明 Noise IK resume）。CI `android` job 先 `cargo ndk` 出 `.so` 再跑 Gradle，`jniLibs/` 入 `.gitignore`。**Manual-required（收窄）**：物理 Android M1.1 **p95 计量**与 8h soak（需真通知链路 + 计时）；`.so` 缺失这一阻塞项已消除。APK 内 Kotlin→JNI→会话的 UI 全链路已于 **v1.18 真机实测关闭**(见 §12 M1.1(v1.18 更新) 与 1.18 变更块)。

### Phase 6 — 单独立项

Android 被控、WinFsp 挂载、文件持久续传、Linux ANCS 体验、嵌入 EasyTier FFI。(notify.reply 于 v1.11 结项;文件持久续传于 v1.12 结项;Windows 凭据库于 v1.13 结项;均不再列入 backlog。)

---

## 13. 测试

| 层 | 内容 |
|---|---|
| 单测 | OTP、帧、状态机、白名单、路径净化、去重、device_id |
| 向量 | Argon2+SPAKE2+Noise prologue |
| proptest | `tetherly-core/tests/decoders.rs`（27）：控制 JSON 全类型 + notify/clip/file/caps/input/screen 往返；每个入站解码器任意输入不 panic（帧、屏幕帧、控制 msg、屏幕重组、输入、ANCS、Hello、device_id、PIN、路径净化）。`tetherly-net/tests/decoders.rs`（7）：长度前缀/带标签/文件头任意字节不 panic、oversized 拒绝、合法头往返、TFL1 兼容。**v1.16 落地**；**v1.17 扩展到有状态状态机**：`tetherly-core/tests/state_machines.rs`（11 条 `proptest` + 7 条确定性见证）以任意**操作序列**驱动 `ReplayGuard`/`TokenBucket`/`NotifyLimiter`/`ClipLimiter`/`NotifyHub`/`FileHub`/`ResumeTable`/`ScreenSender`/`ScreenReceiver`/`ScreenAssembler`/`AncsIngress`，每步后断言安全契约（重放 id 不再接受、限流不越界、文件名安全且不自接受、续传偏移对齐且绑源、无本机同意不推流、非 `Ready` 的 ANCS 回调惰性且不暴露既有 uid） |
| fuzz | `tetherly-core/tests/fuzz_decoders.rs`（2）：确定性 xorshift 变异驱动（稳定工具链替代 libfuzzer）：合法语料 → 6 类变异 → 全部解码器不 panic，`catch_unwind` 捕获；固定种子可复现。**v1.16 落地**；**v1.17 把同一确定性驱动思路推广到状态机操作序列**（`state_machines.rs`）；libfuzzer/`cargo-fuzz`（nightly）仍 Manual-required |
| 集成 | `tests/loopback.rs`：配对、错误 PIN、MITM Hello。`tests/phase1.rs`：notify、clip TTL、insert 拒绝、persist 重拨、日志脱敏、文件拒绝/接受、**跨进程续传(M1.8)**、**`tetherly pin` 活节点回环 API 回归(v1.18:** `pin_shown_by_the_live_node_api_is_the_one_that_pairs`)**、**`tetherly notify` 活节点回环 API 投递回归(v1.20:** `notify_posted_to_the_live_node_api_reaches_the_paired_peer`,断言 `/api/notify` 文本抵达对端候选、未连接 `--peer` 被 400)**。`tests/phase2.rs`：缺侧车、停侧车、无 mDNS 单播、LAN 优先。`tests/phase3.rs`：45719 拒绝配对、过边按键、断线回光标、聚焦时剪贴板。`tests/phase4.rs`：M4.1 模拟时延、M4.2 白名单门控打开、M4.3 重连不连弹、Control Point 串行、分片重组、`Removed` 清候选、退避、**m4_5 入站泵端到端(v1.23:** `m4_5_the_transport_pump_subscribes_in_order_and_feeds_the_ingress`：断言订阅顺序 Data Source 先、回调只入队而 tick 才写 Control Point、DataSource 成候选、`Disconnected` 落 `Backoff`)**。`tests/phase5.rs`：M5.1 consent-then-start 帧序严格递增、M5.2 Start-before-allow 被拒且计数、M5.3 对端离开→`Idle` 重置 seq、M5.4 45720 拒绝未配对 dial 。**v1.19 后端单元测试**:`screen/macos.rs`(7 条:stride 16 字节对齐、去行尾填充、截断/窄 stride 拒绝、BGRA8 位型、`present` 点名 NSWindow)、`screen/x11.rs`(7 条:24 位 alpha 填充、去 stride、`to_frame` 打包/校验、非 Bgra8 拒绝)——纯逻辑、不触显示器,CI 在 `ubuntu-latest`/`macos-latest` 原生编译并执行。**v1.22 内层帧预算回归**:`tetherly-crypto/tests/limits.rs`(2 条:真实 IK 握手后实测明文上限 65519、满额内层帧可加密且多 1 字节被拒)、`tetherly-net/tests/screen_wire.rs`(1 条:4 MiB 屏幕帧分片经真实会话往返重组逐字节相等)、`tetherly-net/tests/backpressure.rs`(2 条:v1.21 写停滞——不读对端得 `WriteStalled`、读对端不误报)。**v1.23 WinRT 传输栈单测**:`tetherly-node/src/ancs/win_gatt.rs`(4 条:Windows 地址记法 `:`/`-`/裸十六进制解析且首字节为高位、6 类畸形输入拒绝、四个 ANCS UUID 的 LE 解码 ↔ 规范 GUID 逐字段一致、畸形 UUID 报错)——纯逻辑、不开蓝牙,`windows-latest` 原生编译并执行 **v1.24 文件/网卡边界回归**：`tetherly-core/src/bind.rs`（1 条：`classify` 九类原因 + 「只有 `Listen` 会 listen、非 `Listen` 必给 reason」的不变量）、`tetherly-core/src/filexfer.rs`（1 条：超限报价 `FileTooLarge` 且不入表、恰好等于上限可接受）、`tetherly-net/src/filechan.rs`（1 条：通道头声明超 `max_size` → `TooLarge` 且零字节缓冲）、`tetherly-node/src/lan.rs`（1 条：`interface_report` 与 `lan_ips()` 必须同一批地址）、`tests/phase1.rs::an_oversized_offer_is_refused_without_dropping_the_session`（端到端：超限报价不成为候选且会话存活） |
| 真机 | 发版清单：Win × Android；Phase 4 再加 iPhone |

PR 守门：loopback + clippy + deny。BLE 与 DeskFlow 不挡 PR。

---

## 14. CI、发版、默认端口

- **CI-First 开发与验证准则（v1.25 确立）**：本项目跨多端与复杂环境（Windows WinRT/DXGI/SendInput、macOS CoreGraphics、Linux glibc 2.31/X11、Android NDK JNI），多平台矩阵构建与安全合规**全面以 GitHub Actions 为核心准绳**。本地开发机保持纯净，仅负责代码实现与单机快速逻辑验证；所有的多平台矩阵测试（Windows/macOS/Ubuntu）、Android NDK 原生构建与 Gradle 打包、`cargo deny` 许可/漏洞守卫，以及 Linux 发布容器打包**一律由 GitHub CI 承担**。开发者通过 `git push` 触发，借助 GitHub CLI（`gh run list`、`gh run watch`）验收云端构建。
- **CI 工作流架构 (`.github/workflows/ci.yml`)**：
  1. `check` 矩阵作业：在 `ubuntu-latest`、`windows-latest`、`macos-latest` 三平台上并行执行 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` 以及 `cargo test --workspace --locked`。任何平台格式、警告或测试不通过即时熔断。
  2. `android` 容器化作业：在 `ubuntu-latest` 上安装 Rust `aarch64-linux-android` 目标与 `cargo-ndk`，编译生成 `libtetherly_android.so`（arm64-v8a）放入 `android/app/src/main/jniLibs`；再经 Java 17 (Temurin) + Gradle 8.9 原生执行 `:app:testDebugUnitTest` 与 `:app:assembleDebug`。构建产物 `jniLibs/` 动态产生、绝不入版本库。
  3. `deny` 安全合规作业：使用 `EmbarkStudios/cargo-deny-action@v2` 针对 RustSec 漏洞数据库与 `deny.toml` 进行 `advisories`、`licenses`、`bans`、`sources` 四重核查，杜绝 AGPL/GPL 链入与已知 CVE 依赖。
- **发布产物与容器守卫 (`.github/workflows/release.yml`)**：
  - Linux 发布二进制在 **`debian:bullseye`(glibc 2.31) 容器**内构建并加 `GLIBC_2.32+` 守卫（`objdump -T` 打印实际地板），确保符号地板 **≤ 2.31**，完全覆盖 Debian 11 / Ubuntu 20.04 及更新主机。容器内强制 `bash` 且 `sources.list` 指向 `archive.debian.org`。
  - Windows 与 macOS 产物分别在 `windows-latest` 与 `macos-latest` 上编译为 `tetherly` 与 `tetherly-cli` 剥离二进制。
  - 打 tag `v*` 时触发 `publish` 作业，自动将三端产物归档并发布至 GitHub Releases。
- 版本号 workspace 统一 bump。
- 控制 **45717/tcp**，文件 **45718/tcp**，键鼠 **45719/tcp**，远程屏幕 **45720/tcp**。本机 UI **45716/tcp** 仅 127.0.0.1。DeskFlow 网关默认关。
- 本机 UI **45716** 同时是 CLI 的**控制面入口**(v1.20):`tetherly pin` / `dial` / `notify` / `send`(v1.25, v1.59 多文件批量) / `resume`(v1.59 续传画像) / `status`(v1.28) 均以它为回环客户端目标(`POST /api/pin` / `/api/dial` / `/api/notify` / `/api/file/send` / `/api/file/send-batch` / `/api/file/resume` 与 `GET /api/state`),不再自起节点。该 HTTP 面**永不**绑非回环地址,且请求须带 `Host: 127.0.0.1|localhost`,否则 403。
- 安装程序提示放行 45717–45720；默认 **LAN** 监听：`127.0.0.1` + RFC1918 / 链路本地，不含公网、**跳过** EasyTier `10.144.144.0/24`（避免 mDNS 打到 TUN）。Phase 2 另绑 overlay CIDR 内的本机 IP，仍永不 `0.0.0.0`/`::`。
- Phase 1 桌面不要求 WebView2（loopback HTTP）。Tauri 壳下一迭代才需要 WebView2、MSVC 运行库；ANCS 需要蓝牙。
- **单文件上限 `FILE_MAX = 64 GiB`（v1.28）**（§12 M1.7）：v1.26 全面确立了 $O(1)$ 内存常数界流式引擎（64 KiB 分块加密与增量 SHA-256 流水线），消除了整文件内存驻留束缚。v1.28 正式将单文件传输上限提升至 64 GiB（`64 * 1024 * 1024 * 1024` 字节）；在报价时按声明大小拒绝（`CoreError::FileTooLarge(FILE_MAX)`），在 45718 数据通道头校验（`recv_chunks_after_header`）与发送端入口严格把关，超限在分配零字节内存前即刻拒绝且绝不拆除已有会话。
- **网卡分类可解释（v1.24）**：`tetherly-core::BindClass`/`classify` 是**唯一真理源**（`should_listen` 即 `classify(..).listens()`）；被跳过的地址在启动日志各占一行 `reason`，并随 `/api/state` 的 `lan` 字段回给本机 UI。关键一类：**CGNAT 段 `100.64.0.0/10`（Tailscale/NetBird）既非 RFC1918 也非链路本地**，v1 **不会**自动绑定，须显式 `--overlay-cidr 100.64.0.0/10`（reason `cgnat-overlay-needs-explicit-cidr`）；`127/8` 非规范别名（如解析器 stub 的 `127.0.0.53`）归 `LoopbackAlias`。

---

## 15. 风险

| ID | 风险 | 缓解 |
|---|---|---|
| R1 | EasyTier LGPL | 侧车，deny LGPL |
| R2 | GPL 键鼠实现 | 自写；网关实验 |
| R3 | iOS 审核 | 可无 App；有则无 VPN |
| R4 | Android 杀后台 | 前台服务 + ROM 说明 |
| R5 | Wayland | beta |
| R6 | 验证码 | 本机、TTL、点击才写、日志脱敏 |
| R7 | BLE 适配器 | 兼容表；向导失败可操作 |
| R8 | 四端成本 | iOS 最小；core 共享 |
| R9 | 8 位 PIN | Argon2id + 锁定 + 绑定身份 |
| R10 | snow/spake2 误用 | 测试向量；不手写曲线 |
| R11 | 虚网无组播 | 单播 peer 列表 |
| R12 | DeskFlow TLS 坑 | 不挡 Phase 3 |
| R13 | AGPL 屏幕采集库（scrap/captrs） | deny 列表 ban + 自有实现（DXGI/核心图形/x11rb） |

---

## 16. 参考仓库

| 仓库 | 用法 |
|---|---|
| sms-pop-rs MIT | OTP、ANCS 时序、UIA 边界 |
| EasyTier LGPL-3 | 侧车组网，不链代码 |
| LocalSend Apache-2.0 | 文件确认 UX |
| Sefirah MIT | Win/Android 体验 |
| DeskFlow GPL | 线格式与端口 24800/TLS，不链代码 |
| KDE Connect | 能力协商思路 |
| ancs4linux | Linux BLE 坑 |
| mcginty/snow | Noise 实现 |
| DecentPaste Apache-2.0 | 配对 UX，算法按本文 |

---

## 17. 载荷例（Noise 内）

```jsonc
{
  "app_id": "com.apple.MobileSMS",
  "app_name": "信息",
  "uid": "42",
  "title": "网易",
  "body": "【网易】验证码：868740，您正在登录网易手机账号",
  "ts": 1760054400,
  "actions": ["copy", "dismiss"]
}
```

不变式：未知 type 默认忽略；业务帧必在 Noise 后；路径与 URL 不来自对端；展示端本地提取 OTP。

---

## 18. Phase 0 开工清单

1. 在 `E:\code\tetherly` 建 workspace（§8 四件套）。
2. 文件头 `Apache-2.0 OR MIT`。
3. 移植 OTP 测试到 `tetherly-core`。
4. `tetherly-crypto`：Argon2id+SPAKE2+PairBind+Noise IK + 向量。
5. `tetherly-cli` + `tests/loopback.rs` 覆盖 M0.2–M0.5。
6. GitHub Actions：M0.7。
7. 不要加 EasyTier、不要加 GUI、不要加 BLE。

---

## 19. Phase 1 开工与落地清单

1. `tetherly-node`：persist identity/trust（temp+rename）、LAN bind 策略、mDNS `_tetherly._tcp`、session 循环、caps.update 广告 `fileport=`。
2. NotifyHub 本地 extract；`insert()` 只挂 UI 点击；候选 120s；剪贴板 OTP 60s 清。
3. 文件：控制面 offer/accept；数据面 45718 `TFL1` + HKDF token；默认确认才收。
4. 本机 UI：`ui/index.html` + `uihttp`，Host 必须是 127.0.0.1/localhost。
5. Android：NLService、前台服务、Wire JSON 无 url action、gradle CI。**JNI `.so` 已落地（v1.10）**：`crates/tetherly-android`（真 Ed25519 + Noise IK，`identity.bin`/`trusted.json` 与桌面同构）→ `libtetherly_android.so`；`jni` 门控 `cfg(target_os="android")`，`unsafe` 仅豁免 `jni.rs`。手机 caps 固定 `["notify","clip","file"]`。
6. `tests/phase1.rs` 覆盖 M1.2–M1.8 与模拟 M1.1/M1.6。
7. **不要**加 EasyTier crate、不要加 DeskFlow 源码、不要改 OTP 规则。Phase 1 清单至此为止；Phase 2 见 §20。

---

## 20. Phase 2 开工与落地清单

1. `tetherly-core::overlay`：CIDR、`PathKind`、JSON/文本 peer IPv4 扫描；默认虚网 `10.144.144.0/24`。
2. `tetherly-node::sidecar`：本机 RPC 15888 → `easytier-cli` → 接口 CIDR；`extra_peers` 作拨号目标。缺侧车返回空列表，不算错误。
3. overlay bind 与 LAN bind 分开；mDNS 只挂 LAN。已有 LAN 则丢 overlay attach。
4. UI `/api/state` 暴露 `lan_only` / `overlay_present` / `overlay_source` / peer `path`。
5. `tests/phase2.rs` 覆盖 M2.2–M2.5 的 loopback 切片；夹具 CIDR 不得撞本机真实 EasyTier TUN。
6. **不要**把 `easytier*` 写进依赖图、不要在 TUN 上发 mDNS、不要改 OTP 规则。Phase 3 见 §21。

---

## 21. Phase 3 开工与落地清单

1. `tetherly-core::input`：`TIN1` 二进制事件、seq 窗口、`InputServer` / `InputClient` / `MemorySink`。无 `cfg(target_os)`。
2. 45719 与 45717/45718 分开 bind（LAN + overlay，仍禁止 `0.0.0.0`）。`SessionConfig.resume_only` 拒绝在此端口配对。
3. `caps.update` 增加 `inputport=`。键鼠帧只走 45719 的独立发送通道，不进控制面 JSON。
4. `tests/phase3.rs` 覆盖未配对拒绝、M3.1 过边按键、M3.2 断线、M3.3 剪贴板。
5. **不要**链接 DeskFlow / Lan Mouse / Barrier / KDE 源码，不要调用 Win32 注入当作 CI，不要改 OTP 规则，不要开 Phase 4。

---

## 22. Phase 4 开工与落地清单

1. `tetherly-core::ancs`：UUID 解码、`NotificationEvent`（8 字节）、Control Point 编码器、`DataAssembler`（按字节重组）、`AncsIngress` 状态机、`AncsTransport` 端口、`MemoryAncsTransport`。**无 `cfg(target_os)`**。
2. 顺序与线程约束写进代码结构，不靠注释：`connect()` 先 `subscribe_data_source` 再 `subscribe_notification_source`；`on_notification_source` 只入队，`tick()` 才写 Control Point；`in_flight` 保证同一时刻一个未应答请求。
3. `AncsIngress` 只用注入的 `Clock`：静默一次后重试、再静默放弃；`APP_NAME_WAIT_MS` 封顶 app 名等待；`NOTIF_ASSEMBLE_DEADLINE_MS` 兜底半截属性；`on_disconnected` 退避 500ms→30s。
4. M4.2 动作只经 `ancs_actions(app_id, &OpenAllowlist)`；「打开」的 url 只来自 `allowlist.url_for`，`open_candidate` 在 `Opener` 端口上执行，**从不**读取通知里的 url。
5. `tests/phase4.rs` 覆盖 M4.1 模拟时延、M4.2、M4.3、串行/回调不写、分片重组、`Removed`、无传输惰性、退避。
6. **不要**在 `tetherly-core` 里放 BLE 栈；不要宣称真机 p95 已达标；不要执行对端 URL；不要改 OTP 规则；不要把 `otp.candidate` 放上线。
7. **v1.23**：真机栈落在 `tetherly-node/src/ancs/win_gatt.rs`（自包含、`#[cfg(windows)]`）——**任何 WinRT 对象都不离开专属 `tetherly-ancs` 线程**；`AncsTransport` 前端只持 `Sender`（按构造 `Send + Sync`）。`ValueChanged` 回调**只拷字节**，写 Control Point 只发生在该线程且**阻塞到 `GattWriteResult::Status == Success`**（前端 ack 等待设 **10s 上界**）；断言「订阅顺序 / 回调不写 / 串行」的回归在 `tests/phase4.rs::m4_5` 与 `win_gatt` 单测。macOS/Linux 栈 `open()` 诚实 `Ok(None)`。不要在 `tetherly-core` 引入 `windows`；不要谎报真机可用。

---

## 23. Phase 5 开工与落地清单

1. `tetherly-core::screen`（**无 OS API**，`#![forbid(unsafe_code)]`，无 `cfg(target_os)`）：`SCREEN_MAGIC=*b"TMV1"`、`SCREEN_PROTO=1`、`SCREEN_SEQ_WINDOW=65536`、`SCREEN_TRACE_MAX=64`；`PixelFormat{Bgra8,Rgba8}`；`ScreenRect`/`ScreenFrame` 编解码（header **31 字节**）；`ControlMsg{Start,Stop,Ack{seq}}`（14 字节）；`ScreenSender`（单调 seq + consent 门控 + push/drain/refused）；`ScreenReceiver`（seq 窗口丢旧/拒大跳/bounded trace）；`MemoryScreenSource`/`MemoryScreenSink`。`ScreenSink`/`ScreenSource` 端口进 `ports.rs`。
2. `tetherly-net`：`SCREEN_PORT=45720`，`NetError::ScreenRequiresTrust`；`session_config_inner` 加 `screen_only: bool`；`hello()` cap 广告 `"screen"`。hello 完只允许已配对对端，`HelloDisposition::Pair` 时 `screen_only` 会话返回 `ScreenRequiresTrust`。
3. `tetherly-node`：`NodeConfig.screen_port`（默认 45720）+ `screen_source: Option<Arc<dyn ScreenSource>>` + `screen_codec: Option<Arc<dyn ScreenCodec>>` + `screen_sink: Option<Arc<dyn ScreenSink>>`；`Shared` 加 `bound_screen`/`screen_out`/`screen_engine`/`screen_observer`/`screen_ctl`/`screen_source`/`screen_codec`/`screen_sink`/`screen_interval`；`spawn_listeners`/`spawn_overlay` 各加 45720 绑定与接受；`attach` 的 caps 加 `"screen"` + `screenport=`；`open_screen`/`accept_screen`/`attach_screen_server`/`attach_screen_client`/`screen_send_frame`；`Node::screen_allow/revoke/start/stop/state/stats/sink_snapshot/port/addr/latency_p95_us`。
4. **红线**：`open_screen` 只建 Noise 通道；开推只由 `screen_allow()` 后的 `screen_start()` 触发；`screen_revoke()` 回 `Idle`；`on_peer_gone` 强制回 `Idle` 并清零 seq。屏幕帧**不携带可执行指令**；被控侧不认识的控制消息一律拒绝，不落盘、不转发。
5. `crates/tetherly-node/src/screen.rs`：**v1.8 真机闭环已落地**——`DxgiSource`（DXGI Desktop Duplication，Windows）、`ZstdCodec` + `EncodedSource<S>`（无损可换端口）、`SystemSink`（GDI BitBlt 同步呈现）。**v1.19 已落地 macOS/Linux 采集**：`MacSource`(`screen/macos.rs`,CoreGraphics `CGDisplayCreateImage` → 显式 BGRA8 `CGBitmapContext` → 去 stride 填充)与 `X11Source`(`screen/x11.rs`,MIT-SHM 快路径 + plain `GetImage` 回退；呈现走 override-redirect 窗口 + `PutImage`,深度取自根而非硬编码)。两者各为自包含文件（只 import 绝对路径），`Cargo.toml` 加目标依赖（macOS `objc2-core-*`；Linux `x11rb`+`libc`）。macOS 呈现需 AppKit `NSWindow`（诚实 `ScreenRefused`）；其余非 Windows/macOS/X11 目标诚实 `ScreenRefused`；Wayland 标「不支持」。真机采集 Manual-required。**不要**链 AGPL/GPL（`deny.toml` 已 ban `hbb_common`/`rustdesk`/`rustdesk-server`/`scrap`）。
6. UI：`/api/screen/allow|start|stop` 路由 + `ui/index.html`「远程屏幕（实验）」区，措辞写清「① 需本机点击允许才推流；② Wayland 暂不支持」。Windows 真机链路已可用（v1.8）；macOS/Linux 与开放编码器仍 Manual-required。
7. `tests/phase5.rs` 覆盖 M5.1 帧序严格递增、M5.2 未 allow 的 Start 被拒且计数、M5.3 对端离开→`Idle` 重置 seq、M5.4 45720 拒未配对 dial。**不要**宣称 Windows DXGI 跨桌面远程桌面已达标：本机 DXGI 采集→zstd→BitBlt 呈现闭环与跨设备（云手机 AdbSource 源）真机闭环均已由 v1.8 落地并实测，DXGI 采集端的像素捕获与 Windows 跨桌面 p95 仍 Manual-required；M5.5（硬件编码 / 真机注入）未做。
8. **不越线**：不链接 RustDesk/AGPL/GPL/LGPL 源码；core 无 `cfg(target_os)`、保持 `#![forbid(unsafe_code)]`；真实采集/编码/注入落在 `tetherly-node` 平台模块并 `#[cfg]` 门控；不绑 `0.0.0.0`；配对永不在 45720；OTP 评分不动；日志与提交信息不含验证码/正文/剪贴板。
9. **v1.8 真机闭环**：`DxgiSource`（DXGI 采集，WAIT_TIMEOUT→`Ok(None)`）+ `ZstdCodec`/`EncodedSource<S>`（无损编码可换端口）+ `SystemSink`（GDI BitBlt 同步呈现）+ `TMF1` 分片 / `ScreenAssembler` 重组 + 接入侧 decompress 链 + `ScreenIntervalTracker`（wall-clock 微秒，bounded 1024，`screen_latency_p95_us()` 暴露 p95）。`tetherly run` 注入 `SystemSink` + 16ms 帧泵（非 Streaming 跳过 grab）。`--screen-adb <serial>` 选用 `AdbSource`（云手机屏幕，PNG→BGRA8，不变屏 `Ok(None)`）。非 Windows 返回诚实 `ScreenRefused`。UI：`/api/screen/start` 自动拨号 screen 通道；Snapshot 的 received/dropped/bytes 来自 visitor 侧 `screen_rx_stats`。
10. **v1.9 键鼠真机链**：受控端 `SystemInputSink`（SendInput：1000 单位→65535 绝对坐标、按钮 mask 位、滚轮、VK 键盘；`NodeConfig.input_sink` 注入，CI 缺省 None）+ 主控端 `spawn_input_capture`（`WH_MOUSE_LL` 恒挂过边检测、`WH_KEYBOARD_LL` 仅远程座位挂、`LLMHF/LLKHF_INJECTED` 过滤防回环）+ 受控端观察者 trace 镜像转发。注入探针实测：过边→光标左缘 (0,539)、中心移动精确、键盘 Win/Esc；DXGI 真实像素捕获 1920x1080（M5.1 收窄）。`TIN1` 语义：VK 码 / 1000 单位 / `BTN_LEFT=1,BTN_RIGHT=2,BTN_MIDDLE=4`。双桌面 100 次物理往返仍 Manual-required。

---

## 24. v1.25 开工与实施清单（文件发送闭环与 CI-First）

1. **GitHub CI-First 构建体系确立**：
   - 任何涉及 Windows（WinRT/DXGI/SendInput）、macOS（CoreGraphics）、Linux（X11/bullseye 容器）以及 Android（NDK arm64-v8a）的跨平台构建、格式检查（`cargo fmt`）、代码规范（`clippy -D warnings`）、单元/集成测试（`cargo test --workspace --locked`）与安全审计（`cargo deny check`）均以 GitHub Actions 为核心准绳。
   - 本地主机仅做轻量代码编写与局部功能调试，禁止在本地维护不可靠的交叉构建链。
   - 发布二进制打包必须由 `.github/workflows/release.yml` 驱动，Linux 严格受限于 `debian:bullseye`（glibc ≤ 2.31 守卫）容器。
2. **生产文件发送入口闭环 (v1.25 核心功能)**：
   - **回环 API 扩充**：在 `tetherly-node::uihttp` 新增 `POST /api/file/send` 路由，接收 JSON 载荷 `{ "path": "<file_path>", "peer": "<device_id>" }`。
   - **API 契约与校验**：
     - 校验 `path` 必须存在且为常规文件，读取长度不超过 `FILE_MAX`（512 MiB）；
     - 目标 `peer` 必须在当前活跃连接列表中（`live_peers`），若无对端或目标不在线返回诚实 400 错误；
     - 校验通过后，调用 `Node::send_file`，内部发起 `FileOffer` 并监听数据通道分块推送。
   - **CLI 客户端扩充**：在 `bins/tetherly-cli` 与 `tetherly` 二进制中增设 `tetherly send <file_path> [--peer <device_id>] [--ui-port <port>]` 子命令。与 `tetherly pin`/`dial`/`notify` 保持一致的 ADR-013 回环客户端机制，严格向 `127.0.0.1:45716` 发送请求，不得自起节点。
   - **UI 呈现**：在 `ui/index.html` 增设「发送文件」表单，允许用户输入或选择本机文件路径，指定目标设备发起直传。
3. **大文件流式架构演进设计 (Blueprint)**：
   - **核心痛点**：目前单文件上限 `FILE_MAX = 512 MiB` 受制于收发双端整文件全量驻留 `Vec<u8>` 内存缓冲。
   - **流式解耦目标**：
     - 发送端：将 `offer_and_send_bytes` 演进为 `offer_and_send_stream`，按 64 KiB 块流式读取磁盘并直接输入 ChaCha20-Poly1305 加密块，内存仅保留单块缓冲；
     - 接收端：分块验证通过后直接写盘 `.part` 文件，同时以 `ring::digest::Context` 流式累积计算 SHA-256 哈希；全部块传输完成后比较最终摘要并原子重命名；
     - 该架构落地后，可平滑将 `FILE_MAX` 提升至数十 GiB 乃至无限制（仅受目标磁盘容量限制）。
4. **测试与回归断言**：
   - `tests/phase1.rs` 新增 `file_sent_via_live_node_api_reaches_the_paired_peer` 端到端回归；
   - 验证通过 `POST /api/file/send` 触发真实双节点文件传输，断言对端接收端收到 offer、点击确认后接收完整、sha256 一致且双方会话存活；
   - 断言文件不存在、超限文件报价被拒且错误码可解释；
   - 维持全量 290+ 测试通过。

---

## 25. v1.26 实施与交付清单（流式大文件传输引擎与 O(1) 内存常数界）

1. **动机与设计准则（ADR-019）**：
   - 突破 v1.24/v1.25 中将 `FILE_MAX = 512 MiB` 绑定到内存驻留的妥协方案；
   - 彻底消灭发送端和接收端整文件 `Vec<u8>` 缓冲，将两端内存占用彻底收敛至单个 64 KiB 传输块的 $O(1)$ 常数界；
   - 采用纯 Rust 零 OS 依赖的增量 SHA-256 状态机，兼顾流式数据吞吐与逐字节校验完整性。
2. **核心模块重构与增量演进**：
   - **`tetherly-core::frame::IncrementalSha256`**：
     - 基于 `sha2::Sha256` 构建流式哈希抽象，支持任意字节分片的 `update`，并提供 32 字节原生指纹与 64 位小写十六进制字符串输出；
     - 纯内存状态机，单测证明与一次性 `hex_sha256` 100% 逐位一致。
   - **`tetherly-net::filechan` 流式收发原语**：
     - `send_stream_from<R, W>`：严格遵循 `TFL2` 魔数规范，按 64 KiB 分块流式读取与封包，维持 `counter_nonce(chan_offset / CHUNK + 1)` 与 `aad_for(transfer_id, chan_offset)` 的 ChaCha20-Poly1305 加密安全约束，直写 Socket；
     - `recv_chunks_to_sink<R, W, F>`：边收边解密边校验 AAD，解密明文即刻异步刷入目标 `AsyncWrite`（`.part` 临时文件句柄）并同步推进 `IncrementalSha256` 状态，触发 `on_chunk` 回调。
   - **`tetherly-node::runtime` 流式节点管线**：
     - `OutboundFile` 抽象为 `OutboundSource::Path(PathBuf)` 与 `OutboundSource::Bytes(Vec<u8>)`；
     - `offer_and_send_file`：通过流式块迭代器增量计算目标文件 SHA-256，免去全量读盘；
     - `on_peer_accepted_file`：对端同意后，异步打开 `tokio::fs::File`，按块对齐精确 seek 到断点续传位置，直接直通网络写入流；
     - `accept_file`：续传时先流式读取磁盘已有前缀注入 `hasher`，随后直接将网络块流追加到 `.part` 临时文件，校验成功后原子重命名为目标文件。
   - **回环 API 对接**：
     - `POST /api/file/send` 检测到 `d.path` 时直接路由至 `offer_and_send_file`，完全杜绝内存二次拷贝。
3. **测试覆盖与 GitHub CI 验证**：
   - 核心单测：`incremental_sha256_matches_one_shot_hex_sha256`；
   - 网络单测：`streaming_roundtrip_with_sink_and_incremental_hasher`、`streaming_resume_from_chunk_boundary`；
   - 端到端回归：`tests/phase1.rs::multi_chunk_file_streamed_via_live_node_api_reaches_inbox`，跨多个 64 KiB 块验证端到端流式落盘与 SHA-256 完整性；
   - 遵循 ADR-018 GitHub CI-First 准则，GitHub Actions 工作流 `ci.yml`（run `38973071906`，5 个跨平台矩阵任务）与 `release.yml`（run `38973572292`，Debian 11 bullseye glibc 2.31 容器构建、Windows x86_64、macOS arm64）全量 100% 绿灯通过。

---

## 26. v1.27 实施与交付清单（传输进度实时追踪、双向观测状态机与 UI 进度可视化）

1. **动机与设计准则（ADR-020）**：
   - 彻底解决 v1.26 大文件流式传输过程中的黑盒问题，让用户与控制面实时掌握传输瞬态与最终状态；
   - 维持 ADR-019 的 $O(1)$ 内存常数界与零拷贝流式性能，进度汇报以零额外内存分配的闭包形式挂载；
   - 保持 ADR-013 回环客户端架构与 ADR-018 GitHub CI-First 自动化工程原则。
2. **核心模块重构与增量演进**：
   - **`tetherly-net::filechan` 发送端进度通知**：
     - 新增 `send_stream_from_with_progress<R, W, F>`，支持在每次分块写出后执行 `on_chunk(chan_offset, want)`；
     - 原 `send_stream_from` 作为零开销特化委托调用新原语；
     - 增设单测 `streaming_send_with_progress_reports_accurate_offsets`，断言每个 64 KiB 偏移量的单调递增性与准确性。
   - **`tetherly-node::runtime` 传输跟踪状态机**：
     - 定义 `TransferDirection`（`Inbound`, `Outbound`）与 `TransferPhase`（`Transferring`, `Verifying`, `Completed`, `Failed`）；
     - 定义 `ActiveTransfer` 模型，包含 `transfer_id`, `peer`, `direction`, `filename`, `total`, `transferred`, `phase`, `error`, `started_ms`, `updated_ms`；
     - 在 `Node` 运行时中维护 `shared.active_transfers` 哈希表，提供 `update_transfer_progress`、`set_transfer_phase` 及 `active_transfers(&self) -> Vec<ActiveTransfer>`（按 `started_ms` 降序排列，严格符合 Clippy `sort_by_key` 规范）；
     - 完整覆盖从 `offer_and_send_file`、`on_peer_accepted_file` 到 `user_accept_file` 及 `handle_file_stream` 的双向全生命周期流转。
   - **`tetherly-node::uihttp` 回环 API 呈现**：
     - `Snapshot` 增加 `transfers: Vec<crate::runtime::ActiveTransfer>` 字段；
     - `GET /api/state` 实时序列化输出当前活跃与近期完成的文件传输列表。
   - **`ui/index.html` Web UI 实时动态呈现**：
     - 新增「实时传输状态」区块与现代化 UI 样式（`.progress-bar-bg`, `.progress-bar-fill`, `.badge`）；
     - 渲染文件传输卡片，动态计算传输百分比，格式化字节显示（B/KB/MB/GB），根据阶段渲染不同色彩标识。
3. **测试覆盖与 GitHub CI 验证**：
   - `tests/phase1.rs` 新增端到端集成测试 `streaming_file_transfer_tracks_active_progress_and_completion_states`：
     - 发起真实大文件流式发送，断言发送端初始状态为 `Outbound`、`Transferring`、`transferred: 0`；
     - 对端确认接收后，断言文件到达收件箱并验证 SHA-256 哈希值；
     - 验证发送端与接收端均平滑跃迁至 `Completed` 阶段，且 `transferred == total`；
     - 通过直接向 `127.0.0.1:45716` 发送 `GET /api/state` 请求，断言回环接口中的 `transfers` 数组内容 100% 匹配；
   - 遵循 ADR-018 GitHub CI-First 原则，在 GitHub Actions CI（run `38979042878`）上全量验证：
     - `check (ubuntu-latest)`: fmt, clippy, test 耗时 3m 2s 全部通过；
     - `check (windows-latest)`: fmt, clippy, test 耗时 2m 47s 全部通过；
     - `check (macos-latest)`: fmt, clippy, test 耗时 2m 16s 全部通过；
     - `android`: NDK arm64-v8a 原生库构建、Gradle 测试与 APK 打包耗时 1m 43s 全部通过；
     - `deny`: 许可证与依赖合规审计耗时 51s 全部通过。

---

## 27. v1.28 实施与交付清单（64 GiB 企业级传输上限、EWMA 速率平滑、CLI 状态监控与 Web UI 可观测性）

1. **动机与设计准则（ADR-021）**：
   - 基于 v1.26 全面落地的 $O(1)$ 内存常数界流式引擎，彻底解除 v1.24/v1.25 的 512 MiB 内存保护边界限制；
   - 将单文件传输上限安全拓展至 64 GiB（`64 * 1024 * 1024 * 1024` 字节），覆盖操作系统磁盘镜像、4K/8K 超高清工程视频等大容量传输场景；
   - 建立传输速率度量模型，采用指数加权移动平均（EWMA）算法平滑消除网络抖动，杜绝瞬态速率毛刺；
   - 增强 CLI 控制面能力，提供 `tetherly status` 状态汇总与动态巡检，为 `tetherly send` 提供终端光标原地回退的交互式进度条；
   - 强化 Web UI 呈现，增加符合人类阅读直觉的字节单位转换、实时传输速率与动态剩余时间（ETA）估算；
   - 严格遵循 ADR-018 GitHub CI-First 构建体系与 ADR-013 进程唯一回环架构。

2. **核心模块重构与增量演进**：
   - **`tetherly-core::filexfer` 企业级上限拓展**：
     - `pub const FILE_MAX: u64 = 64 * 1024 * 1024 * 1024;`（64 GiB）；
     - `FileHub::offer_resumable` 在报价解析层对 `size > FILE_MAX` 严格返回 `CoreError::FileTooLarge(FILE_MAX)`；
     - 增加单测精确验证 64 GiB 边界条件（恰好 64 GiB 允许，64 GiB + 1 字节拒绝）。
   - **`tetherly-node::runtime` EWMA 速率平滑模型**：
     - `ActiveTransfer` 模型新增 `pub speed_bps: u64` 属性，在所有出站与入站传输创建点均规范初始化为 0；
     - `update_transfer_progress` 计量 `delta_bytes` 与 `delta_ms`，应用 EWMA 经典平滑权重：
       $$\text{speed\_bps} = \frac{\text{speed\_bps} \times 7 + \text{inst\_speed} \times 3}{10}$$
     - 当传输跃迁进入 `Completed` 或 `Failed` 终态时，显式将 `speed_bps` 复位为 0。
   - **`tetherly-node::uihttp` 回环客户端拓展**：
     - 提供 `loopback_get(port: u16, path: &str) -> std::io::Result<(u16, Vec<u8>)>`，与 `loopback_post` 共同构成标准回环客户端底座；
     - `POST /api/file/send` 内部元数据检查与 64 GiB `FILE_MAX` 对齐，安全强转 `meta.len() as u64`。
   - **`tetherly` 与 `tetherly-cli` 命令行可观测性加固**：
     - 新增 `Cmd::Status { watch, interval, json, ui_port }` 子命令，向 `127.0.0.1:45716/api/state` 查询运行时状态，提供节点身份、PIN 状态、LAN 监听与已连接对端列表；`--watch` 支持清屏动态轮询；
     - `Cmd::Send` 扩充 `--watch` 参数：向回环接口发起发送后，每 250ms 轮询活跃传输状态，终端以 `\r` 刷新实时进度：
       `[ 68.4%] 43.78 GiB / 64.00 GiB (84.20 MiB/s) [transferring]`
     - 终态检测：`completed` 打印绿标确认退出，`failed` 提取错误信息退出。
   - **`ui/index.html` Web UI 可视化升级**：
     - 实现 `formatBytes(bytes)`（自适应 B / KiB / MiB / GiB / TiB 双精度浮点）；
     - 实现 `formatSpeed(bps)`（格式化字节速率并添加 `/s` 后缀）；
     - 实现 `formatEta(remainingBytes, bps)`（基于当前速率计算剩余秒数并格式化为 `XmYs` 或 `Xs`）；
     - 在传输卡片中实时呈现传输速率与动态剩余时间。

3. **测试覆盖与 GitHub CI 全量验证**：
   - `tests/phase1.rs` 新增端到端集成测试 `enterprise_file_limits_speed_and_status_observability`：
     - 启动配对节点并初始化回环 UI 服务；
     - 发送伪造的大于 64 GiB 报价，断言对端拒绝且候选列表为空（边界门禁有效）；
     - 通过 `loopback_get(port, "/api/state")` 验证回环客户端连通性；
     - 验证流式传输过程中的 `speed_bps` 动态计算与终态复位；
   - 遵循 ADR-018 GitHub CI-First 原则，在 GitHub Actions CI（run `38982885049`，commit `abdca31`）上全量验证：
     - `check (ubuntu-latest)`: fmt, clippy, test（Debian 11 bullseye glibc 2.31 容器）耗时 1m 50s 全部通过；
     - `check (macos-latest)`: fmt, clippy, test（macOS arm64）耗时 2m 48s 全部通过；
     - `check (windows-latest)`: fmt, clippy, test（Windows x86_64）耗时 4m 26s 全部通过；
     - `android`: NDK arm64-v8a 原生库编译与 Gradle 打包耗时 1m 57s 全部通过；
     - `deny`: 许可证与漏洞合规审计耗时 1m 1s 全部通过。

## 28. 全双工传输任务取消机制与控制面非阻塞架构 (v1.29 / ADR-022)

1. **动机与设计准则（ADR-022）**：
   - 超大文件传输必须提供即时的、确定性的双向取消能力，严禁通过强杀进程或中断主网络会话作为替代方案；
   - 45717 控制通道是节点的心跳中枢，必须与 45718 数据通道彻底物理隔离与异步解耦，禁止任何数据流式操作阻塞会话读循环。

2. **核心机制与实现**：
   - **非阻塞控制会话循环**：在 `on_peer_accepted_file` 中，通过 `tokio::spawn(async move { ... })` 将出站数据传输解耦为独立异步任务，避免 `send_stream_from_with_progress` 阻塞 45717 会话读循环与心跳检测；
   - **运行时原子取消映射表 (`transfer_cancels`)**：`Shared` 新增 `transfer_cancels: Mutex<HashMap<String, tokio::sync::oneshot::Sender<()>>>`。入站（`accept_file`）与出站（`on_peer_accepted_file`）启动时注册 oneshot 通道，数据流通过 `tokio::select!` 挂载；
   - **`cancel_transfer` 控制原语**：
     - 从 `pending_out` 移除报价，阻止后续接受竞态；
     - 校验阶段必须为 `Transferring` 或 `Verifying`，否则返回 `CoreError::File("transfer is not active")`；
     - 设置 `phase = TransferPhase::Failed`，`error = Some("cancelled by user".into())`，`speed_bps = 0`；
     - 发送 oneshot 信号，通过 RAII 立即释放底层 TCP 套接字与文件句柄；
   - **回环 API 与 CLI/UI 联动**：
     - 回环控制面暴露 `POST /api/file/cancel`（校验 `Host: 127.0.0.1|localhost`）；
     - `tetherly` 与 `tetherly-cli` 增设 `cancel <id>` 子命令，并为 `tetherly-cli` 补齐 `notify`；
     - Web UI（`ui/index.html`）在传输进行中呈现红色 `[取消]` 按钮。

3. **测试覆盖与 GitHub CI 全量验证**：
   - `tests/phase1.rs` 新增端到端集成测试 `in_flight_streaming_transfer_can_be_cancelled_via_loopback_api`：
     - 启动配对节点并初始化回环 UI 服务；
     - 发起流式多分块文件传输；
     - 通过 `POST /api/file/cancel` 取消传输，断言返回 `{"ok": true, "transfer_id": ...}`；
     - 验证传输状态跃迁为 `Failed`、错误信息为 `"cancelled by user"`、瞬态速率清零；
     - 验证重复取消返回 400 幂等保护；
     - 验证 45717 控制通道未受影响，通知可秒级送达对端；
     - 验证后续全新文件传输可正常完成并校验哈希。

## 29. 传输生命周期全域管控、会话级传输度量与有限留存治理 (v1.30 / ADR-023)

1. **动机与设计准则（ADR-023）**：
   - 守护进程长期驻留运行时，历史传输记录若无限累积会导致内存与回环快照无限膨胀；
   - 终态记录（`Completed`、`Failed`）必须实施有限留存治理（`MAX_TERMINAL_TRANSFERS = 64`），按淘汰时间戳淘汰最久远的终态项；
   - 处于传输中或校验中的在途任务具有绝对隔离保护，决不能被误淘汰；
   - 引入节点生命周期累积的传输度量（`TransferStats`），持久化统计收发字节数与成功/失败计数；
   - 提供回环 API `POST /api/transfers/clear` 与 CLI 子命令 `clear-transfers` 支持用户手动清理历史记录。

2. **核心机制与实现**：
   - **终态记录受控治理**：在 `crates/tetherly-node/src/runtime.rs` 定义 `MAX_TERMINAL_TRANSFERS = 64`，实现 `prune_terminal_transfers_locked` 算法；
   - **会话级度量统计**：定义 `TransferStats { bytes_sent, bytes_received, completed_count, failed_count }`，在出站与入站传输终态变更时原子累加；
   - **手动历史清理**：暴露 `clear_terminal_transfers(&self) -> usize` 原语与 `POST /api/transfers/clear` 路由；
   - **终端与 Web 控制台对接**：`tetherly clear-transfers` 与 `tetherly-cli clear-transfers`，并在 `ui/index.html` 增设「清除完成记录」按钮。

3. **测试覆盖与 GitHub CI 验证**：
   - `tests/phase1.rs` 增设 `terminal_transfers_pruning_and_stats_accumulation` 集成测试，验证 64 项上限滚动淘汰、在途传输免疫、度量单调递增及手动清理接口。

## 30. 信任关系主动解绑与设备安全生命周期治理 (v1.31 / ADR-024)

1. **动机与设计准则（ADR-024）**：
   - 设备配对后公钥持久化保存在 `trusted.json`，遗失、转借或淘汰设备必须具备主动撤销信任（Unpair）能力；
   - 撤销信任必须具备原子级熔断特性：同时清理内存信任库、持久化覆盖磁盘 `trusted.json`、清除物理寻址缓存 `routes.json`，并立即强行关闭所有活跃网络通道（45717 会话与流通道）；
   - 驱逐对端输入控制权，重置回本地模式，广播 `UiEvent::PeerDown`。

2. **核心机制与实现**：
   - **存储层路由遗忘原语**：在 `crates/tetherly-node/src/store.rs` 实现 `forget_route(dir, device_id)`，原子重写 `routes.json`；
   - **节点运行时解绑原语**：在 `crates/tetherly-node/src/runtime.rs` 实现 `unpair(&self, device_id) -> bool`，原子清理内存信任、磁盘信任文件、寻址路由、丢弃通道发送端以即时掐断底层 TCP 连接；
   - **回环控制面与前端**：暴露 `POST /api/peers/unpair`（严格校验 base32 设备 ID 格式），CLI 增加 `unpair <device_id>` 子命令，Web UI 在对端卡片上增加危险解除按钮。

3. **测试覆盖与 GitHub CI 验证**：
   - `tests/phase1.rs` 增设 `peer_unpair_and_trust_revocation` 集成测试，验证初始信任与活跃连接、调用解绑后内存与磁盘信任彻底擦除、网络会话即时断开、输入座位释放及幂等操作安全性。

## 31. 跨端安全剪贴板主动同步、回环控制与全端闭环 (v1.32 / ADR-025)

1. **动机与设计准则（ADR-025）**：
   - 消除发送端人工平台限制，解除 `filter(|p| is_desktop_platform(&p.platform))` 限制，使 Android 与各平台设备均能安全接收剪贴板流；
   - 提供显式序号推进与广播原语 (`ClipHub::force_prepare_local`)，支持手动发送与轮询防回环解耦；
   - 提供受 127.0.0.1 绑核与 Host 头校验保护的回环 API (`POST /api/clip/sync`、`GET /api/clip`)；
   - 严格遵循红线守卫：剪贴板正文绝不打印到守护进程日志；
   - CLI 增设 `clip sync/send/get` 子命令，Web UI 增设「剪贴板同步」交互卡片。

2. **核心机制与实现**：
   - **核心状态机显式步进**：`ClipHub::force_prepare_local` 显式递增 `last_seq` 并更新 `last_hash`，强制生成广播帧；
   - **运行时原语强化**：`send_clipboard_to`、`send_clipboard_text` 与 `get_clipboard_text`；
   - **回环 API 与缓冲区扩容**：`MAX_HEADER = 16 KiB`，`MAX_BODY = 2 MiB`，安全支撑 1 MiB (`CLIP_TEXT_MAX`) 剪贴板文本；
   - **CLI 终端直通**：支持文本参数、标准输入管道 (`stdin`) 与定向 `--peer` 投递。

3. **测试覆盖与 GitHub CI 验证**：
   - `tests/phase1.rs` 增设 `clip_sync_and_send_via_live_node_api_reaches_peers` 集成测试，验证空状态查询、剪贴板主动同步、重复内容回环抑制、显式文本广播、定向投递及超 1 MiB 严格 400 拦截。

## 32. 通知候选生命周期全域治理、OTP 命令行安全提取与双向快速回复闭环 (v1.33 / ADR-026)

1. **动机与设计准则（ADR-026）**：
   - 彻底打通无头/终端环境下的通知候选查询与 OTP 提取闭环，满足开发机、服务器与脚本化自动化运维场景；
   - 严格遵循 ADR-010（验证码写入与读取必须用户当次触发，禁止任何静默外部泄露或日志打印）；
   - 提供轻量级专用轮询 API (`GET /api/notify/candidates`、`GET /api/notify/replies`、`POST /api/otp/get`)，避免拉取全量节点状态快照的性能损耗；
   - 确保双向快速回复单次消费（Single-use）原则，避免同一短信回复被重复触发给移动端底层通道。

2. **核心机制与实现**：
   - **轻量级查询与安全提取 API**：
     - `GET /api/notify/candidates`：返回当前未过期的候选列表 (`Vec<CandidateViewDto>`)；
     - `GET /api/notify/replies`：返回当前待交付至宿主系统的回复列表 (`Vec<ReplyViewDto>`)；
     - `POST /api/otp/get`：显式提取当前候选关联的 OTP 纯文本，脱敏返回，日志绝不打印；
     - `POST /api/reply/take`：增强返回值携带 `text`，严格执行单次消费后即从队列移除。
   - **CLI 终端全功能命令群**：
     - `candidates [--json]`：列表或 JSON 呈现待处理候选与过期倒计时；
     - `otp <id> [--copy]`：直接打印验证码（支持管道传参）并可选复制到剪贴板；
     - `dismiss <id>`：主动丢弃失效候选；
     - `reply <id> <text>`：针对候选通知反向发送快速回复短信；
     - `replies [--json]` 与 `take-reply <id>`：查询和单次消费入站回复。
   - **双端 CLI 同构**：`tetherly` 与 `tetherly-cli` 具备完全一致的命令行参数与回环客户端机制。

3. **测试覆盖与 GitHub CI 验证**：
   - `tests/phase1.rs` 增设 `notify_candidate_lifecycle_and_quick_reply_via_loopback_api` 集成测试：
     - 验证端到端通知推送与候选正则解析；
     - 验证 `POST /api/otp/get` 提取与 `POST /api/copy` 写入剪贴板；
     - 验证通过 `POST /api/reply` 发送回复并在对端安全入队；
     - 验证 `POST /api/reply/take` 成功提取并遵循单次消费不变量；
     - 验证 `POST /api/dismiss` 销毁候选与非法参数 400 保护。

## 33. 远程屏幕与键鼠通道全域控制、定向拨号与无头命令行交互闭环 (v1.34 / ADR-027)

1. **动机与设计准则（ADR-027）**：
   - 彻底打破无头、终端与 SSH 会话环境下的远程桌面串流与键鼠控制交互屏障；
   - 严格维护 ADR-010 与 ADR-013 原则：屏幕推流严格依赖本机显式授权（Local Consent），且一切控制命令必须作用于 127.0.0.1 活节点回环客户端；
   - 消除多对端在线时的定向盲区：全面支持可选 `--peer <device_id>` 参数，多设备组网时精准定向建链，单设备时智能推导；
   - 补齐状态管理与可观测性闭环：暴露 `POST /api/screen/revoke` 撤销端点以及 `GET /api/screen`、`GET /api/input` 轻量级低延迟查询接口。

2. **核心机制与实现**：
   - **轻量级查询与控制回环 API**：
     - `GET /api/screen`：返回当前屏幕流状态、通道是否建立、已发/接收/丢弃/字节/拒绝计数以及 p95 帧间隔；
     - `GET /api/input`：返回键鼠通道是否连接以及光标当前归属座位 (`local` / `remote`)；
     - `POST /api/screen/revoke`：撤销本机授权，丢弃在途帧并将状态置回 `Idle`；
     - 升级 `POST /api/screen/start` 与 `POST /api/input/start`：接收 JSON `{ "peer": "tdev_..." }`，定向连接指定设备的 45720 / 45719 端口。
   - **CLI 终端全功能命令群（`tetherly` 与 `tetherly-cli` 全量同构）**：
     - `screen allow`：本机授权对端观看屏幕；
     - `screen revoke`：撤销屏幕观看许可；
     - `screen start [--peer <id>]`：发起屏幕推流；
     - `screen stop`：停止屏幕推流；
     - `screen status [--json]`：展示屏幕通道与帧指标；
     - `input start [--peer <id>]`：连接远程键鼠控制通道；
     - `input stop`：断开键鼠控制通道并归还本地光标；
     - `input status [--json]`：展示键鼠通道连接状态与当前座位。
   - **Web UI 补齐**：在 `ui/index.html` 远程屏幕卡片中增设「撤销允许」按钮。

3. **测试覆盖与 GitHub CI 验证**：
   - `tests/phase5.rs` 增设 `screen_and_input_lifecycle_and_peer_targeting_via_loopback_api` 集成测试：
     - 验证 `GET /api/screen` 与 `GET /api/input` 查询；
     - 验证 `POST /api/screen/allow` 与 `POST /api/screen/revoke` 状态跃迁；
     - 验证未授权时 `POST /api/screen/start` 被 400 拦截；
     - 验证定向 `--peer` 跨节点成功开启屏幕推流且帧数递增；
     - 验证 `POST /api/input/start` 定向建链与光标座位转移，断开后安全复位。

## 34. 对端拓扑全景可观测性、物理路由缓存透视、轻量级对端与网络检索及命令行同构 (v1.35 / ADR-028)

1. **动机与设计准则（ADR-028）**：
   - 彻底打破对端在线状态与底层物理路由信息的黑盒化；
   - 消除受信任离线设备的“隐形”问题，使得信任库中的全量资产无论在线与否均清晰可查，支持离线解绑；
   - 提供轻量级专用轮询 API（`GET /api/peers` 与 `GET /api/routes`），避免调用全量 `GET /api/state` 带来的沉重序列化开销；
   - 严格维护 ADR-010 与 ADR-013 原则：127.0.0.1 绑核、Host 头校验与命令行回环客户端模式。

2. **核心机制与实现**：
   - **轻量级对端全景与路由查询 API**：
     - `GET /api/peers`：返回本机 ID、在线设备统计、受信任设备统计，以及在线对端列表（`live`）与全景受信任列表（`trusted`，标记 `online: true/false`）；
     - `GET /api/routes`：返回节点在 `routes.json` 中记录的物理寻址路由缓存列表；
   - **CLI 终端全功能命令群（`tetherly` 与 `tetherly-cli` 全量同构）**：
     - `peers [--json] [--trusted]`：展示在线对端表格或全量受信任设备（含离线标记）；
     - `routes [--json]`：展示物理寻址缓存表格（对端 ID、物理地址、最后活跃时间）。
   - **Web UI 补全**：在 `ui/index.html` 增设受信任设备区块，呈现红绿在线/离线指示徽标，并支持一键解除配对。

3. **测试覆盖与 GitHub CI 验证**：
   - `tests/phase1.rs` 增设 `peers_and_routes_inspection_via_loopback_api` 集成测试：
     - 验证未配对前 `peers` 与 `routes` 均为空；
     - 验证配对并建立会话后，`GET /api/peers` 准确反映在线设备与受信任设备且 `online: true`；
     - 验证配对后 `GET /api/routes` 包含该设备的物理路由条目；
     - 验证对端节点下线后，受信任对端状态准确跃迁为 `online: false`；
     - 验证执行 `POST /api/peers/unpair` 后，`trusted` 与 `routes` 同步抹除。

## 35. 网卡物理分类透视、EasyTier 侧车穿透诊断与网络探测回环闭环 (v1.36 / ADR-029)

1. **动机与设计准则（ADR-029）**：
   - 彻底打破网络接口物理分类与 EasyTier 侧车深层拓扑的黑盒；
   - 摆脱必须请求全量快照 `GET /api/state` 才能知晓网卡状态的高开销弊端，提供轻量级专用回环接口 `GET /api/net`；
   - 将 EasyTier 侧车发现的虚拟 IP (`local_ips`) 与对端 (`peers`) 完整透视给运维与用户；
   - 提供全平台对齐的 CLI 诊断工具 `tetherly net` 与 `tetherly-cli net`，支持 `--interfaces` 详细跳过原因展现与 `--json` 格式化管道输出；
   - 严格维护 ADR-006（EasyTier 仅为外部侧车，严禁链接进 `Cargo.toml`）与 ADR-010/ADR-013（回环 127.0.0.1 绑核、Host 头校验与命令行轻量客户端模式）。

2. **核心机制与实现**：
   - **轻量级网络与侧车透视回环 API (`GET /api/net`)**：
     - 返回当前全部物理与虚拟网卡的分类报告列表 (`interfaces: Vec<IfaceReport>`)，包含接口名、IP、是否监听及跳过判定理由（`None` 表示监听，否则给出诸如 `LoopbackAlias`、`LinkLocal`、`Tailscale / NetBird CGNAT overlay` 等分类理由）；
     - 返回 EasyTier 侧车的深度拓扑模型 (`overlay`)：包含是否启用 (`enabled`)、是否就绪 (`present`)、来源 (`source`：`Absent`、`Ifaces`、`Rpc`、`Cli`、`Manual`)、本机虚拟 IP 清单 (`local_ips`) 及对端虚拟 IP 清单 (`peers`)；
   - **节点运行时扩展 (`Node::network_report`)**：
     - 提供 `network_report(&self) -> (Vec<IfaceReport>, OverlayStatus)` 方法，聚合 `lan::interface_report()` 与当前节点的 `overlay_status()`；
   - **CLI 终端诊断命令群（`tetherly` 与 `tetherly-cli` 全量同构）**：
     - `net [--interfaces] [--json] [--ui-port <port>]`：
       - 默认模式：打印 EasyTier 侧车状态概览（就绪态、来源、本机虚拟 IP、对端虚拟 IP 统计）及当前活跃监听的局域网 IP 清单；
       - `--interfaces` 模式：进一步展开格式化网卡分类判定表格，明确打印所有网络接口的监听 (`[LISTEN]`) 或跳过 (`[SKIP]`) 状态及其理由；
       - `--json` 模式：以纯 JSON 形式输出底层透视数据，方便外部监控集成；
   - **Web UI 补全 (`ui/index.html`)**：
     - 在 `#net` 区块中渲染 EasyTier 侧车深度拓扑与接口分类诊断表格，提供直观的色彩徽章指示（绿色 `[监听]` / 灰色 `[跳过]`）。

3. **测试覆盖与 GitHub CI 验证**：
   - 在 `tests/phase2.rs` 增设 `network_interface_classification_and_sidecar_inspection_via_loopback_api`：
     - 验证 `GET /api/net` 成功返回 200 OK 且响应字段合法；
     - 验证本地回环 `127.0.0.1` 恒定被纳入并处于 `listen: true` 状态且 `reason: null`；
     - 验证包含各网络接口的判定逻辑；
     - 验证在默认局域网模式下 `overlay.present` 正确反映侧车就绪态；
     - 验证注入附加虚拟对端 (`extra_peers`) 后 `overlay.present` 与来源准确转换为 `Manual` 且对端 IP 准确体现在响应清单中。

## 36. ANCS 苹果通知服务深层可观测性、白名单规则治理与命令行回环全同构 (v1.37 / ADR-030)

1. **动机与设计准则（ADR-030）**：
   - 彻底打破 ANCS 苹果通知服务在终端环境与监控探针下的状态黑盒；
   - 摆脱必须请求全量快照 `GET /api/state` 才能知晓 ANCS 状态的高开销弊端，提供轻量级专用回环接口 `GET /api/ancs`；
   - 完善 ANCS Ingress 的全生命周期控制：支持查询连接态、重试倒计时、触发重连握手 (`connect`) 与动态追加应用跳转规则 (`allow`)；
   - 提供全平台对齐的 CLI 诊断与治理工具 `tetherly ancs` 与 `tetherly-cli ancs`，支持 `status [--json]`、`connect` 与 `allow <app_id> [--url <template>]`；
   - 严格维护 ADR-004（ANCS 是 Ingress，不是 Transport，禁止把 BLE 当 RPC）、ADR-010/ADR-013（回环 127.0.0.1 绑核、Host 头校验与轻量客户端模式）与安全红线（载荷 URL 绝不自动执行，跳转必须经过 OpenAllowlist 白名单精确匹配）。

2. **核心机制与实现**：
   - **轻量级 ANCS 状态回环 API (`GET /api/ancs`)**：
     - 返回模型包含：
       - `ok: bool`：操作成功标识；
       - `attached: bool`：当前节点是否挂载了硬件 BLE 传输栈实例；
       - `state: String`：当前 Ingress 状态（如 `off`、`idle`、`subscribing`、`ready`、`backoff`）；
       - `retry_at_ms: Option<u64>`：处于 `backoff` 状态时的计划重试时间戳（毫秒）；
       - `allowlist: Vec<String>`：已加载的允许通过 `POST /api/open` 打开的 App ID 列表；
   - **节点运行时扩展 (`Node::ancs_report`)**：
     - 统一聚合底层传输栈存活性、`AncsIngress` 状态、退避时钟与 `OpenAllowlist` 规则；
   - **CLI 终端诊断与治理命令群（`tetherly` 与 `tetherly-cli` 全量同构）**：
     - `ancs status [--json] [--ui-port <port>]`：表格化呈现 ANCS 传输栈挂载状态、Ingress 状态、退避重连时钟以及生效白名单清单；
     - `ancs connect [--ui-port <port>]`：调用 `POST /api/ancs/connect` 主动触发对端 iPhone 的 ANCS 特征值订阅；
     - `ancs allow <app_id> [--url <template>] [--ui-port <port>]`：调用 `POST /api/ancs/allow` 追加白名单规则；
   - **Web UI 补全 (`ui/index.html`)**：
     - 在 Web UI 中增加「ANCS 苹果通知服务」管理面板，显示传输栈状态徽标与重试倒计时，集成一键「连接/重连」按钮与白名单规则添加表单。

3. **测试覆盖与 GitHub CI 验证**：
   - 在 `tests/phase4.rs` 增设 `ancs_status_inspection_connection_and_allowlist_via_loopback_api`：
     - 验证默认无 transport 时 `GET /api/ancs` 正确返回 `attached: false`、`state: "off"`；
     - 验证在默认状态下调用 `POST /api/ancs/connect` 返回 400（"no ANCS transport attached"）；
     - 验证通过 `POST /api/ancs/allow` 动态追加白名单规则，`GET /api/ancs` 立即反映更新；
     - 验证挂载 `MemoryAncsTransport` 后，`GET /api/ancs` 正确呈现 `attached: true`，调用 `POST /api/ancs/connect` 成功触发订阅并返回 `ready` 状态；
     - 验证非法请求参数（空 JSON、缺少参数）被严格拒绝并返回 400。

## 37. 节点数据目录全景可观测性、存储配额分析与孤儿分块垃圾回收 (v1.38 / ADR-031)

1. **动机与设计准则（ADR-031）**：
   - 打破长周期运维下节点数据目录（`data_dir`）与接收收件箱（`inbox`）的存储状态黑盒；
   - 彻底治理大文件传输在网络抖动、进程中断或任务取消后残留的孤儿分块文件（`.{transfer_id}.part`）与侧车（`.resume.json`）；
   - 严格维护安全红线：正在在途传输（`Transferring` / `Verifying`）的活跃分块绝对隔离保护，严禁被清理删除；
   - 提供全平台对齐的 CLI 诊断与清理命令 `tetherly storage` 与 `tetherly-cli storage`，支持 `status [--json]` 与 `clean [--all]`；
   - 严格遵循 127.0.0.1 绑核、Host 头校验与轻量回环客户端架构。

2. **核心机制与实现**：
   - **轻量级存储可观测性回环 API (`GET /api/storage`)**：
     - 单遍高效扫描 `data_dir` 与 `inbox` 目录；
     - 返回结构体 `StorageReport`：包含 `ok`、`data_dir`、`inbox_dir`、`inbox_total_bytes`、`inbox_file_count`、`partial_bytes`、`partial_count`、`orphaned_partial_count`、`orphaned_partial_bytes` 以及 `meta_bytes`；
   - **安全孤儿分块垃圾回收 API (`POST /api/storage/clean`)**：
     - 接收可选 JSON 参数 `{"orphaned_only": bool}`（缺省为 `true`）；
     - 返回结构体 `StorageCleanReport`：包含 `ok`、`freed_bytes`、`deleted_files_count` 与 `cleared_resume_entries`；
     - 活跃传输任务白名单强隔离保护；
   - **CLI 终端同构命令群 (`tetherly` 与 `tetherly-cli`)**：
     - `storage status [--json] [--ui-port <port>]`：格式化容量单位（B / KiB / MiB / GiB）友好展示或机器可读 JSON；
     - `storage clean [--all] [--ui-port <port>]`：安全触发垃圾回收并打印释放容量；
   - **Web UI 补全 (`ui/index.html`)**：
     - 增设「存储与数据目录」面板，动态监控收件箱容量与分块占用，支持一键安全清理孤儿文件。

3. **测试覆盖与 GitHub CI 验证**：
   - 在 `tests/phase1.rs` 增设 `storage_inspection_and_partial_cleanup_via_loopback_api`：
     - 验证空目录初始化指标；
     - 验证接收文件与孤儿分块混合场景下的精确区分统计；
     - 验证调用 `POST /api/storage/clean` 仅精准清理孤儿文件，正常接收文件毫发无损；
     - 验证全部历史未决分块清理与断点续传表注销逻辑。

## 38. 节点健康检查探针、轻量级守护进程可观测性与优雅停止机制 (v1.39 / ADR-032)

1. **动机与设计准则（ADR-032）**：
   - 彻底解耦高频监控探活与开销巨大的全量 `GET /api/state` 状态快照；
   - 提供纳秒级、零额外内存分配的健康检查端点 `GET /api/health`，适应容器存活/就绪探针与前端实时心跳；
   - 统一暴露守护进程关键指标：进程 PID、持续运行时长（uptime）、操作系统平台以及五维服务端口（control 45717, file 45718, input 45719, screen 45720, ui 45716）的动态实际绑定值；
   - 彻底解决节点守护进程退出依赖外部硬杀（`kill -9`）的问题，重构 `Node::run(&self)` 与所有监听接收循环，提供受控的平滑优雅停止端点 `POST /api/daemon/shutdown` / `POST /api/daemon/stop`；
   - 提供双端同构的轻量级 CLI 工具（`tetherly` 与 `tetherly-cli`）命令群：`health`、`ping`（精确统计往返 RTT 时延）与 `stop`（优雅停止守护进程）；
   - 严格维护安全红线：127.0.0.1 绑核、Host 头校验与命令行轻量回环客户端原则。

2. **核心机制与实现**：
   - **极低开销健康探针回环端点 (`GET /api/health`)**：
     - 结构体 `HealthReport` 与 `NodePorts`：
       ```json
       {
         "ok": true,
         "status": "healthy",
         "version": "1.39.0",
         "device_id": "c0ffee...",
         "name": "desktop-node",
         "platform": "macos",
         "pid": 12345,
         "uptime_secs": 3600,
         "ports": {
           "control": 45717,
           "file": 45718,
           "input": 45719,
           "screen": 45720,
           "ui": 45716
         },
         "live_peers": 2,
         "trusted_peers": 3
       }
       ```
     - 节点启动时在 `Shared` 中记录单调时钟 `started_at: Instant`，纳秒级获取 `uptime_secs`；
   - **优雅停止与监听循环协同解耦**：
     - 重构 `Node::run(&self)`：由原先挂起的 `std::future::pending()` 改为异步等待 `watch::Receiver<bool>` 关闭信号；
     - 增设 `POST /api/daemon/shutdown` 与 `POST /api/daemon/stop` 回环端点：在后台延迟 50ms 异步触发 `node.shutdown()`，优先返回 HTTP 200 确认；
     - `uihttp::accept_loop` 监听 `node.shutdown_signal()`，在触发时退出循环并释放端口；
     - `node_accept_loop` 同样订阅关闭信号，安全跳出 accept 循环，使 `Node::run` 平滑结束并返回 `Ok(())`；
   - **CLI 终端同构命令群 (`tetherly` 与 `tetherly-cli`)**：
     - `health [--json] [--ui-port <port>]`：结构化表格呈现各项健康度量或标准 JSON 管道输出；
     - `ping [--count <n>] [--timeout-ms <ms>] [--ui-port <port>]`：客户端高精时钟测算单次及平均 loopback RTT 时延；
     - `stop [--ui-port <port>]`：向运行中的节点发起受控优雅停机；
   - **Web UI 补全 (`ui/index.html`)**：
     - 在 Web UI 中增设「守护进程与系统健康」卡片，显示健康徽标、PID、天/时/分/秒运行时长、端口绑定映射、实时 RTT 往返时延度量与受控优雅停机交互按钮。

3. **测试覆盖与 GitHub CI 验证**：
   - 在 `tests/phase1.rs` 增设 `node_health_probe_ping_and_graceful_shutdown_via_loopback_api`：
     - 验证直接 Rust 调用 `node.health_report()` 指标准确无误；
     - 验证并发启动 `node.run()` 背景任务并成功进入监听与等待态；
     - 验证通过 loopback HTTP `GET /api/health` 成功获取探针指标且往返 RTT 时延在微秒/亚毫秒级（CI 环境宽限内）；
     - 验证向 `POST /api/daemon/shutdown` 发送停机请求收到 HTTP 200 `shutting_down` 应答；
     - 验证 `node.run()` 顺畅解挂并在 3 秒内干净返回 `Ok(())`；
     - 验证 UI 端口 listener 已彻底退出，后续连接被拒绝。

## 39. 节点事件总线实时流式推送 (SSE)、实时事件可观测性与 CLI 事件监听闭环 (v1.40 / ADR-033)

1. **动机与设计准则（ADR-033）**：
   - **高频轮询开销治理**：淘汰原先依靠每 700ms 轮询全量快照 `GET /api/state` 带来的 CPU、网络与内存开销；
   - **微秒级事件响应性**：将内部 `broadcast::Sender<UiEvent>` 事件总线通过 W3C 标准 Server-Sent Events (SSE) 直推到客户端；
   - **双端 CLI 同构支持**：`tetherly events` 与 `tetherly-cli events` 提供类似 `tail -f` 的命令行事件实时监听器；
   - **安全红线铁律（Verbatim Security Red Line）**：`UiEvent` 严格禁止携带短信正文、OTP 验证码、配对码与剪贴板纯文本；所有回环长连接严格限制 `127.0.0.1` 绑核与 `Host` 头校验。

2. **核心机制与实现**：
   - **标准 Server-Sent Events 端点 (`GET /api/events` 与 `GET /api/events/stream`)**：
     - 在 `uihttp.rs` 请求解析管线中对 `GET /api/events` / `GET /api/events/stream` 进行流式分发；
     - 响应标准 HTTP 200 text/event-stream 响应头并立即 flush；
     - 发送初始握手注释行 `: connected\n\n`；
     - 在 `tokio::select!` 中并发监听：
       - `events.recv()`：序列化 `UiEvent` 为 JSON，以 `data: <json>\n\n` 格式推送；
       - `node.shutdown_signal()`：节点触发关闭信号时安全跳出循环，干净终止长连接；
       - `ping_interval`（15 秒）：推送 `: ping\n\n` 注释帧，防止中间防火墙或系统超时断流；
     - 其他非 GET 请求调用该端点严格返回 405 Method Not Allowed。
   - **客户端辅助结构与轻量回环连接器**：
     - 在 `tetherly-node` 中提供 `SseStream` 结构体，封装 `Lines<BufReader<TcpStream>>`，自动过滤注释与空行，提供 `next_event()`（原始字符串）与 `next_ui_event()`（类型化 `UiEvent`）；
     - 提供 `loopback_events_connect(port)` 异步连接助手；
     - 在解耦的 `tetherly-cli` 中内建完全自包含的 `loopback_events_connect` 与 `SseStream`，无需链入 `tetherly-node`。
   - **双端 CLI 同构事件流命令 (`tetherly events` 与 `tetherly-cli events`)**：
     - 支持 `--count <n>`：收集满 N 个事件后自动退出；
     - 支持 `--timeout-secs <s>`：指定秒数未收到后续事件或总时长达到后优雅退出；
     - 支持 `--json`：逐行输出原始 JSON，便于 jq 过滤或自动化管道处理；
     - 默认模式：终端结构化美化打印，带有类别标签（`[peer_up]`、`[file_offered]`、`[status]` 等）。
   - **Web UI 实时交互联动 (`ui/index.html`)**：
     - 接入浏览器原生 `EventSource("/api/events")`，断开自动指数退避重连；
     - 事件到达时即时触发 `tick()` 刷新界面，将原先 700ms 背景高频轮询降为 2000ms+ 安全底噪轮询；
     - 在系统健康面板增加「实时事件流 (SSE)」连接状态徽标与「最近节点事件」跑马灯。

3. **测试覆盖与 GitHub CI 验证**：
   - 在 `tests/phase1.rs` 增设 `node_events_stream_sse_via_loopback_api`：
     - 验证非法 HTTP 动作（如 `POST /api/events`）被严格阻断并返回 405 Method Not Allowed；
     - 验证通过 `loopback_events_connect` 建立流式连接；
     - 验证节点派发 `Status`、`PeerUp`、`FileOffered`、`FileDone` 事件按序被流式接收且反序列化数据完全匹配；
     - 验证多并发客户端连接与原始 JSON `next_event()` 解析；
     - 验证当守护进程调用 `node.shutdown()` 时，所有活跃 SSE 流优雅终止并返回 `Ok(None)`。

## 40. 对端加密通道往返时延 (RTT) 实时测速、网络质量度量与主动链路探测 (v1.41 / ADR-034)

1. **动机与设计准则（ADR-034）**：
   - **链路质量黑盒消除**：在之前的实现中，底层 `TYPE_PING` (0x0001) 与 `TYPE_PONG` (0x0002) 帧已在控制会话维持长连接，但接收对端 Pong 时直接在 `SessionEvent::Pong(_) => {}` 丢弃了时间戳，导致系统无法感知对端网络质量与物理时延波动；
   - **零协议额外开销**：复用现存的 8 字节大端毫秒时间戳载荷，无需引入任何新的协议帧或握手开销即可精确度量毫秒级往返时延 (RTT)；
   - **加密通道隔离**：所有 Ping/Pong 报文完全在 Noise IK 加密隧道内部传输，外层观察者绝对无法获取任何明文时间戳或网络指标；
   - **异步解耦按需探测**：通过 `oneshot::Sender<u64>` 等待派发向量队列，支持本地管理端非阻塞发起链路上行探测并安全等待响应，设置 5 秒超时保护；
   - **安全红线铁律（Verbatim Security Red Line）**：回环探测接口严格绑定 `127.0.0.1`，强校验 `Host` 请求头（非法请求返回 403）；时延事件广播与报告绝对禁止携带任何用户隐私或敏感内容。

2. **核心机制与实现**：
   - **会话级 RTT 度量与 LivePeer 扩展**：
     - `LivePeer` 结构体引入 `rtt_ms: Option<u64>` 与 `last_seen_ms: u64`；
     - 处理 `SessionEvent::Pong(ts)` 时，计算 `now_ms.saturating_sub(ts)`，原子更新 `live` 状态，触发 `UiEvent::PeerLatency` 广播，并唤醒 `ping_waiters` 队列；
     - `LivePeer::last_seen_ms` 随任意合法入站帧实时刷新；
   - **回环 HTTP 探测端点 (`POST /api/peers/ping`)**：
     - 支持请求体指定 `{"peer": "tdev_..."}`；当请求体为空或单对端连接时自动解析目标；多对端且未指定时返回明确 400 提示；
     - 返回结构化 `PeerPingReport` (`ok`, `device_id`, `rtt_ms`, `path`, `addr`)；
     - 在 `GET /api/peers` 与 `GET /api/state` 中完整输出 `rtt_ms` 与 `last_seen_ms`；
   - **实时事件流式广播 (`UiEvent::PeerLatency`)**：
     - 每次测量得到有效 RTT 时，自动向 SSE 事件总线广播 `UiEvent::PeerLatency { device_id, rtt_ms }`，驱动 Web UI 亚毫秒级更新；
   - **双端 CLI 同构测速命令 (`tetherly` 与 `tetherly-cli`)**：
     - `peers --ping [--peer <id>] [--count <n>] [--json]`：对端列表结合测速；
     - `ping --peer <id> [--count <n>] [--json]`：对端通道测速（省略 `--peer` 则测 local daemon loopback 响应）；
     - 支持 `--count <n>` 多包探测并统计 min/avg/max 时延；
   - **Web UI 实时时延徽标与交互测速 (`ui/index.html`)**：
     - 对端卡片增设动态时延徽标（绿色 <50ms、蓝色 50~150ms、黄色 >150ms、灰色未测）；
     - 增设一键「测速」按钮，点击发起 `POST /api/peers/ping` 并提供实时交互反馈。

3. **测试覆盖与 GitHub CI 验证**：
   - 在 `tests/phase1.rs` 增设 `active_peer_rtt_probing_and_latency_reporting_via_loopback_api`：
     - 验证直接 Rust 调用 `node.ping_peer(&peer_id)` 成功返回报告且 RTT 符合局域网预期；
     - 验证非法/未连接设备 ID 准确拒绝并返回 `NoConnectedPeer`；
     - 验证 loopback `POST /api/peers/ping` 指定设备与单设备自动解析；
     - 验证 `GET /api/peers` 与 `GET /api/state` 包含 `rtt_ms` 与 `last_seen_ms`；
     - 验证 SSE 事件流成功接收到 `UiEvent::PeerLatency` 广播事件。

## 41. 文件传输通道全景可观测性、轻量级传输查询与命令行同构治理 (v1.42 / ADR-035)

1. **动机与设计准则（ADR-035）**：
   - **大文件传输状态轻量解耦**：自 v1.28 起 Tetherly 支持高达 64 GiB 的流式文件传输。此前查看传输任务状态与速率必须轮询 `GET /api/state`，在存在长周期大文件传输时频繁获取完整节点快照产生严重的冗余计算与序列化开销；
   - **轻量级按需检索与状态过滤**：增设专用回环端点 `GET /api/file/transfers`（及其别名 `GET /api/transfers`），支持通过 `?id=tfl_...` 进行单任务 $O(1)$ 精准查询（不存在时标准返回 404），并支持 `?active=true`、`?terminal=true`、`?direction=inbound|outbound` 进行按需过滤；
   - **会话级度量透视**：在传输报告中同时返回各任务实时进度与会话级累计指标 (`TransferStats`：`bytes_sent`, `bytes_received`, `completed_count`, `failed_count`)，即使用户清除了已完成的历史传输，总吞吐量依然准确保留；
   - **双端 CLI 同构**：在 `tetherly` 与 `tetherly-cli` 全量增设 `transfers` 子命令，提供格式化进度条、实时自适应传输速度与会话级统计透视；
   - **安全红线铁律（Verbatim Security Red Line）**：回环接口仅监听 `127.0.0.1`，严格校验 `Host: 127.0.0.1|localhost`（不匹配返回 403）；在途文件传输（`Transferring` / `Verifying`）受底层状态机强保护，严防误删；接口禁止泄露任何超出传输元数据之外的隐私信息。

2. **核心机制与实现**：
   - **传输报告模型 (`TransfersReport`)**：
     - 在 `runtime.rs` 与 `lib.rs` 引入 `TransfersReport` DTO：
       ```rust
       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct TransfersReport {
           pub ok: bool,
           pub active_count: usize,
           pub terminal_count: usize,
           pub total_count: usize,
           pub transfers: Vec<ActiveTransfer>,
           pub stats: TransferStats,
       }
       ```
     - 扩展 `Node::transfer_by_id(&self, id: &str) -> Option<ActiveTransfer>` 与 `Node::transfers_report(&self) -> TransfersReport`；
   - **回环 HTTP 端点 (`GET /api/file/transfers` & `GET /api/transfers`)**：
     - 统一解析 URL Query 与 POST JSON 参数（支持 `id`, `active`, `terminal`, `direction`）；
     - `id` 命中返回 200 及 `ActiveTransfer`，未找到返回 HTTP 404；
     - 列表模式按启动时间倒序聚合并按指定条件过滤，返回 `TransfersReport`；
   - **双端同构命令行**：
     - `transfers [--id <id>] [--active] [--terminal] [--direction <in|out>] [--json] [--ui-port <port>]`；
     - 格式化展示已传输字节数/总大小、百分比进度、自适应传输速率 (`format_speed`) 及累计吞吐；
   - **Web UI 传输面板增强 (`ui/index.html`)**：
     - 增设独立「刷新传输」按键，调用轻量级端点获取最新状态；
     - 展示会话累计统计（总发送、总接收、完成笔数、失败笔数）；
     - 清除历史完成后自动触发轻量刷新；
   - **端到端集成测试全覆盖 (`tests/phase1.rs`)**：
     - `file_transfers_inspection_and_query_via_loopback_api`：验证初始状态、404 未找到、单任务 ID 检索、active/terminal 过滤、direction 过滤、传输完成后的度量累计与清理闭环。

## 42. 配对控制面治理、PIN 码安全生命周期管理与 CLI 拨号配对全同构 (v1.43 / ADR-036)

1. **动机与设计准则（ADR-036）**：
   - **PIN 码生命周期盲区消除**：自 v1.18 起 Tetherly 引入基于 SPAKE2 的 8 位数字 PIN 配对机制，但此前仅提供单向生成端点 `POST /api/pin`，缺乏轻量级查询端点 `GET /api/pin`。客户端与 Web UI 无法探知当前是否存在活跃 PIN、已武装 PIN 的格式化数字与剩余倒计时秒数（TTL 倒计时）；
   - **主动安全撤销机制**：PIN 具有 180 秒（`PIN_TTL`）超时窗口。在此前架构中，一旦武装 PIN 无法中途撤销，若发生误触或对端取消配对，节点将在此时间窗内持续暴露潜在的配对接收状态。v1.43 引入 `Node::clear_pin()` 与 `POST /api/pin/clear`（及 `DELETE /api/pin`）端点，支持在超时前即时作废 PIN，实现安全状态的主动废黜与幂等返回；
   - **双端 CLI 同构治理**：消除独立二进制 `bins/tetherly-cli` 与守护进程 `crates/tetherly-node` 二进制 `tetherly` 之间的异构缺陷。废黜 `tetherly-cli` 遗留的 Phase 0 临时进程随机数生成与临时 TCP 拨号，将其全面升级为与主守护进程通信的回环客户端；
   - **Web UI 状态恢复与实时感知**：页面刷新时自动调用 `GET /api/pin` 还原武装状态与倒计时，增设一键「撤销 PIN」交互，并接入 SSE `pin_state_changed` 事件实现无感响应；
   - **安全红线铁律（Verbatim Security Red Line）**：
     - 「公开 issue / 日志禁止贴短信正文、一次性码、配对码、剪贴板」；
     - 「信任库只在 Noise transport 成功后写」；
     - 「Loopback UI 45716 仅绑定 127.0.0.1，校验 Host: 127.0.0.1|localhost (403 on mismatch); 严禁绑定 0.0.0.0 / ::」；
     - **`UiEvent` 严禁携带短信正文、一次性码、配对码、剪贴板纯文本**，因此 `UiEvent::PinStateChanged` 仅携带 `active: bool`，绝对禁止在事件总线与广播载荷中出现 PIN 数字。

2. **核心机制与实现**：
   - **PIN 状态报告模型 (`PinReport`)**：
     - 在 `runtime.rs` 与 `lib.rs` 引入 `PinReport` DTO：
       ```rust
       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct PinReport {
           pub ok: bool,
           pub active: bool,
           pub pin: Option<String>,
           pub expires_in_secs: u64,
           pub ttl_secs: u64,
           pub created_at_ms: u64,
       }
       ```
     - 扩展 `Node::pin_report(&self) -> PinReport` 与 `Node::clear_pin(&self) -> bool`；
     - 在生成与设置 PIN 时广播 `UiEvent::PinStateChanged { active: true }`，在清除 PIN 时广播 `UiEvent::PinStateChanged { active: false }`；
   - **回环 HTTP 控制面端点**：
     - `GET /api/pin`：直接返回当前 PIN 的 `PinReport`；
     - `POST /api/pin/clear` 与 `DELETE /api/pin`：原子清除武装状态，返回 `{"ok": true, "cleared": bool}`；
     - `POST /api/pin`：生成并返回新 PIN 的同时，携带 `expires_in_secs` 与 `ttl_secs`；
   - **双端同构命令行**：
     - `pin [--status] [--clear] [--json] [--ui-port <port>]`：支持查询当前 PIN 状态、主动撤销武装 PIN、结构化 JSON 输出与向后兼容生成；
     - `dial --addr <ip:port> [--pin <pin>] [--ui-port <port>]`：统一向本地运行的守护进程递送配对拨号请求；
   - **Web UI 配对面板增强 (`ui/index.html`)**：
     - 新增 `#clrpin` 按键与 `#pin-ttl` 倒计时徽标；
     - 页面加载自动调用 `refreshPin()` 还原状态；
     - SSE 监听 `pin_state_changed` 自动同步最新 PIN 状态；
   - **端到端集成测试全覆盖 (`tests/phase1.rs`)**：
     - `pin_lifecycle_inspection_and_revocation_via_loopback_api`：验证初始非活跃状态、POST 生成与 SSE 广播、GET 查询与 TTL 校验、POST 清除与 SSE 广播、清除后非活跃确认以及幂等二次清除。

## 43. 对端设备别名治理、友好命名持久化与对端标识智能解析 (v1.44 / ADR-037)

1. **动机与设计准则（ADR-037）**：
   - **Base32 身份标识认知负担与别名缺失**：Tetherly 底层采用 21 位加密哈希 ID（`DeviceId`），但在交互界面和命令行中使用机器随机字符体验较差；
   - **持久化别名存储模型与冲突消解**：在节点配置目录维护 `aliases.json`，支持别名原子写盘与双向映射；
   - **智能对端解析赋能 (`Node::resolve_peer`)**：全面支持直接设备 ID、用户分配别名、mDNS 广播名及前缀/子串模糊匹配；
   - **回环控制端点与双端 CLI 全同构**：`POST /api/peers/alias` 与 `DELETE /api/peers/alias` 端点，结合 `tetherly alias` / `tetherly-cli alias` 命令群；
   - **Web UI 别名卡片与端到端测试闭环**：对端卡片内嵌别名展示与就地编辑弹窗，`tests/phase1.rs` 增设 `peer_alias_governance_and_smart_resolution_via_loopback_api`。

## 44. 节点运行配置全景透视、动态运行时特性开关与轻量级配置回环治理 (v1.45 / ADR-038)

1. **动机与设计准则（ADR-038）**：
   - **静态配置黑盒与重启割裂消除**：节点广播可见性、断线自动重连等核心特性此前仅在启动时加载，无法在运行时动态切换；
   - **轻量级配置透视与热更新模型 (`ConfigReport`, `ConfigUpdateDto`)**：实现 `Node::update_config` 与安全校验；
   - **回环 HTTP 控制面端点 (`GET /api/config`, `POST /api/config`)**：支持零停机热修改节点友好名称、局域网 mDNS 广播可见性与自动重连开关；
   - **双端同构命令行 (`tetherly config` / `tetherly-cli config`)**：支持配置查看与 `--name`、`--advertise`、`--reconnect` 动态修改；
   - **Web UI 配置面板与测试闭环**：增设配置状态卡片与实时热切换开关，`tests/phase1.rs` 增设 `node_config_inspection_and_dynamic_flags_via_loopback_api`。

## 45. 物理寻址路由表全景透视、静态路由注入与断线自动重连治理 (v1.46 / ADR-039)

1. **动机与设计准则（ADR-039）**：
   - **物理寻址路由透视模型 (`RouteEntryDto`, `RoutesReport`)**：对齐物理 IP:Port 寻址与对端元数据，聚合在线/受信任状态、广播名与友好别名；
   - **静态路由双向注入与原子落盘原语 (`remember_route`, `forget_route`)**：实现返回布尔指示状态（added vs updated，removed vs not_found）的安全写盘，彻底解决跨子网与无广播环境下对端寻址丢失难题；
   - **智能对端解析赋能 (`Node::add_route`, `Node::remove_route`)**：全面支持直接设备 ID、友好别名、广播名及模糊子串定位目标对端，严格校验物理套接字地址合法性与非零端口；
   - **轻量级回环控制面端点 (`POST /api/routes`, `DELETE /api/routes`, `POST /api/routes/delete`)**：支持标准 RESTful 与灵活查询参数/JSON 体删除，结合 `UiEvent::RouteChanged` SSE 事件总线实时派发；
   - **CLI 双端全同构升级 (`tetherly route` 与 `tetherly-cli route`)**：提供 `list`、`add` 与 `del`/`remove` 子命令群，无缝支持友好别名与机器可读 JSON 输出；
   - **Web UI 物理路由卡片与实时交互闭环**：增设物理寻址路由与静态注入面板，提供动态卡片列表、交互式弹框注入与一键删除确认；
   - **端到端集成测试全绿覆盖**：在 `tests/phase1.rs` 增设 `route_table_governance_and_static_injection_via_loopback_api` 完备验证治理全链路。

## 46. 全协议网络流量透视、多通道遥测度量与运行时可观测性闭环 (v1.47 / ADR-040)

1. **动机与设计准则（ADR-040）**：
   - **全局网络流量黑盒**：除了文件通道内部的会话级 `TransferStats` 外，守护进程缺乏对整机网络出入站总吞吐（TX/RX 字节数、数据帧包数）的统一度量与聚合；
   - **四维多通道带宽分布不可知**：控制 (45717)、文件 (45718)、输入 (45719)、屏幕 (45720) 各自消耗了多少网络带宽完全缺乏细分度量，无法定位由于大文件传输或屏幕高频推流导致的网络拥堵瓶颈；
   - **协议消息遥测计数缺失**：Ping/Pong 探活、Notification 通知、Clipboard 剪贴板、FileOffer 文件报价等核心协议帧缺乏全局调用计数，会话建连数与握手失败率处于黑盒状态；
   - **缺少轻量级遥测回环查询与基准重置端点**：外部监控或自动化运维脚本无法一键获取全局网络指标，也无法在基准性能测试中原子清零历史数据；
   - **双端命令行与 Web UI 缺乏遥测面板**：`tetherly` 与 `tetherly-cli` 缺失 `metrics`/`traffic` 命令，Web 前端缺少流量透视卡片；
   - **安全红线铁律（Verbatim Security Red Line）**：
     - 「公开 issue / 日志禁止贴短信正文、一次性码、配对码、剪贴板」；
     - 「信任库只在 Noise transport 成功后写」；
     - 「Loopback UI 45716 仅绑定 127.0.0.1，校验 Host: 127.0.0.1|localhost (403 on mismatch); 严禁绑定 0.0.0.0 / ::」；
     - **`UiEvent` 严禁携带短信正文、一次性码、配对码、剪贴板纯文本**，遥测数据必须仅包含纯数字统计，严禁包含任何载荷或用户数据。

2. **核心机制与实现**：
   - **无锁原子遥测引擎 (`TrafficTelemetry`)**：
     - 在 `crates/tetherly-node/src/telemetry.rs` 与 `lib.rs` 引入高性能度量模型：
       ```rust
       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct ChannelTrafficDto {
           pub tx_bytes: u64,
           pub rx_bytes: u64,
           pub tx_frames: u64,
           pub rx_frames: u64,
       }

       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct ProtocolMessagesDto {
           pub pings_sent: u64,
           pub pings_recv: u64,
           pub pongs_sent: u64,
           pub pongs_recv: u64,
           pub notifies_sent: u64,
           pub notifies_recv: u64,
           pub clips_sent: u64,
           pub clips_recv: u64,
           pub file_offers_sent: u64,
           pub file_offers_recv: u64,
       }

       #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
       pub struct TrafficReport {
           pub ok: bool,
           pub uptime_secs: u64,
           pub total_tx_bytes: u64,
           pub total_rx_bytes: u64,
           pub total_tx_frames: u64,
           pub total_rx_frames: u64,
           pub control: ChannelTrafficDto,
           pub file: ChannelTrafficDto,
           pub input: ChannelTrafficDto,
           pub screen: ChannelTrafficDto,
           pub messages: ProtocolMessagesDto,
           pub sessions_established: u64,
           pub sessions_closed: u64,
           pub handshakes_failed: u64,
       }
       ```
     - 采用 `AtomicU64` 与 `Ordering::Relaxed`，以 $O(1)$ 时间复杂度在网络数据收发热路径上累加出入站字节与帧数，杜绝锁争用；
     - 提供原子 `snapshot(uptime_secs)` 与 `reset()` 原语；
   - **全通道 I/O 链路埋点集成 (`runtime.rs`)**：
     - 控制通道 (45717)：出入站帧收发与协议帧类型原子累加；
     - 文件通道 (45718)：流式传输与内存分块发送/接收出入站字节累计；
     - 键鼠通道 (45719)：输入事件帧收发出入站字节与帧数累计；
     - 屏幕通道 (45720)：屏幕推流与接收字节与帧数累计；
     - 会话生命周期：`sessions_established`, `sessions_closed`, `handshakes_failed` 状态跟踪；
   - **轻量级回环控制面端点 (`uihttp.rs`)**：
     - `GET /api/metrics` 与 `GET /api/traffic`：毫秒级响应 `TrafficReport`；
     - `POST /api/metrics/reset` 与 `POST /api/traffic/reset`：原子清零各通道与协议度量，广播 `UiEvent::MetricsReset`；
   - **CLI 双端全同构命令 (`tetherly metrics` / `traffic`, `tetherly-cli metrics` / `traffic`)**：
     - 支持 `--reset`、`--json` 与 `--ui-port`；
     - 格式化终端输出：自动换算 B/KiB/MiB/GiB，绘制四通道带宽分布柱状占比，清晰呈现协议消息与会话健康度；
   - **Web UI 遥测仪表盘与交互闭环 (`ui/index.html`)**：
     - 增设「网络流量与全协议遥测」面板，集成出入站吞吐徽标、四通道带宽分布进度指示与协议核心消息芯片；
     - 提供一键重置按键与 SSE `metrics_reset` 实时局部刷新；
	   - **端到端集成测试全绿覆盖 (`tests/phase1.rs`)**：
	     - `traffic_metrics_inspection_and_telemetry_reporting_via_loopback_api`：完整断言初态查询、控制帧交互流量递增、文件流式传输度量、POST 重置及 SSE 事件响应与清零验证。

## 47. ANCS 苹果通知服务全生命周期治理、白名单规则动态增删查、状态机事件总线闭环与双端 CLI 全同构 (v1.56 / ADR-049)

1. **动机与设计准则（ADR-049）**：
   - **ANCS 状态流转事件盲区**：在 BLE GATT 链路重连、断开或退避等待（`Idle`、`Ready`、`Backoff`）期间，底层缺乏事件广播，导致上层 Web UI 与 CLI 事件总线失去实时感知能力；
   - **白名单单向不可逆缺陷**：`OpenAllowlist` 此前仅允许追加规则，缺乏根据 `app_id` 移除单条规则（`remove`）与全量清空规则（`clear`）的能力；
   - **缺少结构化强类型 DTO 抽象**：`/api/ancs` 控制面接口此前返回松散内联 JSON，缺乏统一强类型契约定义（`OpenRuleDto`、`AncsConnectReport`、`AncsDisconnectReport`、`AncsRuleReport`、`AncsClearRulesReport`）；
   - **双端 CLI 与 Web UI 全同构治理闭环**：CLI 缺乏对规则增删查与断开连接的支持，Web UI 缺少行内删除规则与一键清空按键，且缺少基于 SSE 的局部即时刷新。

2. **核心机制与实现**：
   - **核心白名单原语与安全守卫 (`allowlist.rs` & `error.rs`)**：
     - `OpenRule` 派生 `Serialize`；
     - `OpenAllowlist` 新增 `remove(&mut self, app_id: &str) -> Option<OpenRule>` 与 `clear(&mut self) -> usize`；
     - 严格校验 `is_local_scheme`，阻断非本地 Scheme（`http://`、`https://`、`file://` 等）注入；
     - 核心错误新增 `CoreError::NoAncsTransport` 变体；
   - **事件总线扩展与安全红线守卫 (`events.rs`)**：
     - 引入 `UiEvent::AncsStateChanged { state, attached, retry_at_ms }` 与 `UiEvent::AncsRuleChanged { action, app_id, url, rules_count }`；
     - 坚守 Zero-Plaintext 红线：绝对不携带任何苹果通知消息正文、标题或敏感属性；
   - **结构化 DTO 抽象与运行时控制原语 (`runtime.rs`, `lib.rs`)**：
     - 导出 `OpenRuleDto`、`AncsConnectReport`、`AncsDisconnectReport`、`AncsRuleReport`、`AncsClearRulesReport` 与增强型 `AncsReport`；
     - 实现 `ancs_connect_reported`、`ancs_disconnect_reported`、`add_open_rule_reported`、`remove_open_rule_reported`、`clear_open_rules_reported`，联动事件总线广播；
   - **轻量级回环控制面端点升级 (`uihttp.rs`)**：
     - 重构 `GET /api/ancs`，提供强类型状态与完整规则列表；
     - `POST /api/ancs/connect` 与 `POST /api/ancs/disconnect`；
     - `POST /api/ancs/allow` (及 `POST /api/ancs/rules/add`)；
     - `DELETE /api/ancs/allow` (及 `POST /api/ancs/rules/remove` 与 `DELETE /api/ancs/rules`)；
     - `POST /api/ancs/rules/clear` (及 `DELETE /api/ancs/rules/clear`)；
     - 严格守卫 `127.0.0.1` 回环绑定与 `Host` 头校验；
   - **CLI 双端全同构升级 (`tetherly-cli` 与 `tetherly`)**：
     - `ancs` 命令支持 `status`、`connect`、`disconnect`、`allow`、`remove`、`clear`，并支持 `--json` 输出；
     - SSE `events` 监听器高亮 `[ancs_state_changed]` 与 `[ancs_rule_changed]`；
   - **Web UI 实时动态刷新与状态徽标 (`ui/index.html`)**：
     - 增设「断开 ANCS」、规则行内删除标签 (`×`) 与「清空白名单」按钮；
     - 挂载 SSE 监听器实现毫秒级局部刷新。

3. **测试覆盖与验证**：
   - `tests/phase4.rs` 新增端到端集成测试 `ancs_lifecycle_governance_rule_management_and_events_via_loopback_api`：
     - 验证非回环 Host 请求（`Host: evil.attacker.com`）拦截并返回 403 Forbidden；
     - 验证 SSE `/api/events` 实时流式推送；
     - 验证初态查询、主动连线、添加规则、删除规则、清空规则、主动断开与状态/规则事件广播全链路。








