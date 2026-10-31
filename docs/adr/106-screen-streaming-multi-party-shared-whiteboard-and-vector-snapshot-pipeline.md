# ADR-106 屏幕串流多方协作共享画板同步、时序撤销重做与矢量快照导入导出流水线（Screen Streaming Multi-Party Shared Whiteboard Synchronization, Chronological Undo/Redo & Vector Snapshot Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-28，v2.20 / v0.1.42）**——建立屏幕串流多方协作共享画板同步、时序撤销重做与矢量快照导入导出流水线：(1) **画板时序历史引擎与多用户撤销重做模型**（`crates/tetherly-core/src/screen.rs`）：严格遵循 ADR-006 零重型外部依赖铁律，设计 `WhiteboardHistory`、`WhiteboardPolicy` 与 `WhiteboardSnapshot`；支持按作者（`author`）定向撤销/重做或全局最近撤销/重做；支持图元数量上限防内存耗尽；(2) **线协议信令定长扩充与全量专属帧**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 13（`ControlMsg::WhiteboardUndo { author_hash: u64 }`）、Tag 14（`ControlMsg::WhiteboardRedo { author_hash: u64 }`）与 Tag 15（`ControlMsg::SetWhiteboardPolicy { read_only, allow_peer_clear, max_items }`）；在专属帧 `TYPE_SCREEN_ANNOTATION = 0x050a`（魔数 `b"TANN"`）中扩充 `Remove`、`Undo`、`Redo`、`Snapshot` 与 `Policy` 动作；(3) **节点运行时画板生命周期与呈现全链路挂载**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_whiteboard: Arc<RwLock<WhiteboardHistory>>`，在呈现（`screen_sink`）、录制（ADR-101）、回放（ADR-102）与单帧快照（ADR-103）中深度合成；(4) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/whiteboard/undo`、`/api/screen/whiteboard/redo`、`/api/screen/whiteboard/export`、`/api/screen/whiteboard/import` 与 `/api/screen/whiteboard/policy` 端点；集成 Loopback 绑定、Host 头部 403 与严格路径穿越（`..`）校验；(5) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen whiteboard [status|undo|redo|export|import|policy|clear]` 子命令族与表格排版格式化输出；(6) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_30_screen_whiteboard_multi_user_sync_and_undo_redo_pipeline` 全链路验证。 |
| 日期 | 2026-10-28 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-098「屏幕隐私模式、黑屏遮罩与物理输入屏蔽安全流水线」、ADR-101「屏幕串流会话录制容器与回放流水线」、ADR-103「屏幕串流实时快照提取流水线」、ADR-104「屏幕串流动态安全水印与取证叠加层流水线」、ADR-105「屏幕串流实时协作标注与瞬态虚拟激光笔流水线」 |

---

## 动机

1. **多人协同标注中的误操作与缺乏撤销能力（Lack of Undo/Redo）**：在 ADR-105 中引入了基础折线、矩形、箭头、文本与虚拟激光笔，但若某一方绘制了错误标注，此前唯一的清理手段是 `clear`（全量清空画布），这会瞬间抹除所有参与者的所有心血。协作画板急需细粒度的时序撤销（Undo）与重做（Redo）能力；
2. **多参与者身份归属与图层隔离（Author Attribution & Multi-User Layering）**：在远程排障、代码结对评审、在线培训场景中，通常有宿主机操作员（Host）与多位远程协助者（Visitor 1, Visitor 2...）。当某一协助者执行「撤销」时，系统必须支持定向撤销该作者（`author`）最近的一笔标注，而非误撤销他人正在指引的关键标注；同时亦保留全局撤销选项；
3. **画板矢量快照持久化导出与导入（Vector Snapshot Persistence & Import/Export）**：会议与排障讨论结束后，用户需要将当前的标注矢量集以结构化 JSON 快照（`.twhiteboard` / `.tann`）导出持久化，以便存盘归档、关联到录制容器（ADR-101）中，或在后续新会话中一键导入还原现场；
4. **白板安全策略与权限治理（Whiteboard Moderation & Host Sovereignty）**：遵循 ADR-004 / ADR-098 的被控端最高主权原则，宿主机管理员可配置白板策略：
   - `read_only: bool`（只读白板：仅宿主机可画，访客端画笔被硬性阻断 HTTP 403）；
   - `allow_peer_clear: bool`（是否允许访客一键清空全量画板）；
   - `max_items: u16`（图元数量硬上限，防止恶意洪泛攻击导致内存耗尽）；
5. **零重型外部依赖铁律（Zero External Crates, ADR-006）**：严格禁止引入外部重型矢量库或图形引擎。全部撤销重做栈、快照序列化、图元重放与光栅化均由 `tetherly-core` 纯标准库与 `serde` 实现；
6. **全链路协同与双端 CLI 100% 同构**：白板控制面与数据流无缝集成到呈现渲染（`screen_sink`）、会话录制（ADR-101）、录制回放（ADR-102）、单帧快照（ADR-103）、回环 REST API 以及双端 CLI（`tetherly` 与 `tetherly-cli`）。

---

## 决定

### 1. 画板时序历史模型与核心引擎（`crates/tetherly-core/src/screen.rs`）

- **数据模型**：
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub struct WhiteboardPolicy {
      #[serde(default)]
      pub read_only: bool,
      #[serde(default = "default_allow_peer_clear")]
      pub allow_peer_clear: bool,
      #[serde(default = "default_max_items")]
      pub max_items: u16,
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct WhiteboardSnapshot {
      pub version: u32,
      pub created_at_ms: u64,
      pub session_id: String,
      pub active_items: Vec<AnnotationItem>,
      pub total_added_counter: u64,
  }

  #[derive(Debug, Clone, Default)]
  pub struct WhiteboardHistory {
      active: Vec<AnnotationItem>,
      undo_stack: Vec<AnnotationItem>,
      redo_stack: Vec<AnnotationItem>,
      policy: WhiteboardPolicy,
      total_added: u64,
  }
  ```
- **核心操作与撤销重做状态机**：
  - `add_item(item)`：检查 `policy.read_only` 与 `policy.max_items`，推入 `active`，清空 `redo_stack`；
  - `undo(author: Option<&str>)`：若指定 `author`，逆序寻找该作者最近一项，移出 `active` 并推入 `redo_stack`；若未指定则弹出最近一项；
  - `redo(author: Option<&str>)`：若指定 `author`，逆序寻找 `redo_stack` 中该作者最近一项，移回 `active`；
  - `export_snapshot(session_id, now_ms)`：导出非过期图元与元数据；
  - `import_snapshot(snapshot, replace)`：安全批量导入，重新分配 ID 与时序；
  - `clear()`：清空活跃图元并记录 undo 历史。

### 2. 线协议信令定长扩充与全量专属帧

- **14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）**：
  - **Tag 13 (`ControlMsg::WhiteboardUndo { author_hash: u64 }`)**：
    - 64 位负载容纳作者标识 FNV-1a 紧凑哈希（0 为全局）；
  - **Tag 14 (`ControlMsg::WhiteboardRedo { author_hash: u64 }`)**：
    - 64 位负载容纳作者标识 FNV-1a 紧凑哈希（0 为全局）；
  - **Tag 15 (`ControlMsg::SetWhiteboardPolicy { read_only: bool, allow_peer_clear: bool, max_items: u16 }`)**：
    - 紧凑位打包：bit 0 表示 `read_only`，bit 1 表示 `allow_peer_clear`，bits 16..32 表示 `max_items`。
- **专属数据帧 `TYPE_SCREEN_ANNOTATION = 0x050a`（魔数 `b"TANN"`）**：
  - `ScreenAnnotationMsg` 扩充：
    - `Add(AnnotationItem)`
    - `Remove { id: u32 }`
    - `Undo { author: String }`
    - `Redo { author: String }`
    - `Clear`
    - `Snapshot(WhiteboardSnapshot)`
    - `Policy(WhiteboardPolicy)`

### 3. 节点运行时管线与全生命周期挂接（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载 `screen_whiteboard: Arc<RwLock<WhiteboardHistory>>` 与 `screen_whiteboard_policy: Arc<RwLock<WhiteboardPolicy>>`；
- 双向信令与帧路由处理：
  - 收到 Tag 13/14 时调用本地 undo/redo 并刷新呈现画面；
  - 收到 Tag 15 时更新白板策略；
  - 收到专属帧时派发对应图元动作；
- 在接收端呈现管线（`screen_sink.present`）、录制（`screen_playback`）与单帧快照（`screen_snapshot_data`）中统一集成。

### 4. 回环 REST 控制面与双端 CLI 全同构

- **REST API**：
  - `POST /api/screen/whiteboard/undo`：执行撤销；
  - `POST /api/screen/whiteboard/redo`：执行重做；
  - `GET /api/screen/whiteboard/export`：导出快照；
  - `POST /api/screen/whiteboard/import`：导入快照；
  - `GET /api/screen/whiteboard/policy` 与 `POST /api/screen/whiteboard/policy`：查询与设置策略；
- **双端 CLI 同构命令**：
  - `screen whiteboard status [--json] [--ui-port <port>]`
  - `screen whiteboard undo [--author <name>] [--json] [--ui-port <port>]`
  - `screen whiteboard redo [--author <name>] [--json] [--ui-port <port>]`
  - `screen whiteboard export [--out <path>] [--json] [--ui-port <port>]`
  - `screen whiteboard import <path_or_json> [--replace] [--json] [--ui-port <port>]`
  - `screen whiteboard policy [--read-only <bool>] [--allow-clear <bool>] [--max-items <num>] [--json] [--ui-port <port>]`
  - `screen whiteboard clear [--json] [--ui-port <port>]`

---

## 验证与测试准则

1. 核心单测：`crates/tetherly-core/src/screen.rs` 验证 `WhiteboardHistory` 撤销/重做栈操作、按作者定向撤销、快照导出导入序列化、Tag 13/14/15 编解码 roundtrip；
2. REST 单测：`crates/tetherly-node/src/uihttp.rs` 验证 `/api/screen/whiteboard/*` 端点（撤销、重做、导入、导出、策略门控与 Host 403 严格隔离）；
3. E2E 集成测试：`tests/phase5.rs::m5_30_screen_whiteboard_multi_user_sync_and_undo_redo_pipeline` 验证双节点配对下，多作者协作绘制、撤销重做网络同步、只读策略硬性阻断与矢量快照导入导出还原。
