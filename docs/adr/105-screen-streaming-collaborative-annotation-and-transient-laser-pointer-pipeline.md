# ADR-105 屏幕串流实时协作标注、画板叠加层与瞬态虚拟激光笔流水线（Screen Streaming Collaborative Annotation, Whiteboard Overlay & Transient Laser Pointer Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-28，v2.19 / v0.1.41）**——建立屏幕串流实时协作标注、画板叠加层与瞬态虚拟激光笔流水线：(1) **零依赖光栅化几何渲染器与激光晕染引擎**（`crates/tetherly-core/src/screen.rs`）：严格遵循 ADR-006，零重型图形库依赖纯手工实现定点整数布雷森汉姆直线算法（带粗线段多像素填充）、中点圆光斑算法（带定点平方反比高斯衰减发光外晕）、30度箭头几何渲染与 8x8 ASCII 点阵文字标注；定点 Alpha 混合器自适应适配 `PixelFormat::Bgra8` 与 `Rgba8`；(2) **结构化协作标注与激光笔数据模型**（`AnnotationItem`, `AnnotationKind`, `LaserPointerState`, `ScreenAnnotationReport`）：支持自由折线（`Stroke`）、矩形框选（`Rect`）、指引箭头（`Arrow`）与点阵文本（`Text`），支持毫秒级生存周期（TTL Auto-Fadeout）自动过期与一键全量清空；(3) **线协议信令定长扩充与专属帧**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 11（`ControlMsg::SetLaserPointer { x, y, active, color_rgb }`）与 Tag 12（`ControlMsg::ClearAnnotations`）；引入专属帧类型 `TYPE_SCREEN_ANNOTATION = 0x050a`（魔数 `b"TANN"`）承载结构化矢量图元增量与全量同步；(4) **节点运行时全链路挂载与快照联动**（`crates/tetherly-node/src/runtime.rs`）：在接收端呈现管线（`screen_sink.present`）、会话录制（ADR-101）、录制回放（`screen_playback`）与静态快照（`screen_snapshot`）中自动叠加标注与激光笔，隐私黑屏（`privacy_blanked`）模式下优先绝对遮蔽；(5) **回环 REST 控制面闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `GET /api/screen/annotation`、`POST /api/screen/annotation`、`POST /api/screen/annotation/clear`、`DELETE /api/screen/annotation`、`GET /api/screen/laser` 与 `POST /api/screen/laser` 端点；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen annotate [list|add|clear]` 与 `screen laser [coords|status]` 子命令族；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_29_screen_collaborative_annotation_and_laser_pointer_pipeline` 全链路验证。 |
| 日期 | 2026-10-28 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-072「空间输入隧道」、ADR-088「硬件光标解耦与独立信道定步」、ADR-098「屏幕隐私模式、黑屏遮罩与物理输入屏蔽安全流水线」、ADR-101「屏幕串流会话录制容器与回放流水线」、ADR-103「屏幕串流实时快照提取流水线」、ADR-104「屏幕串流动态安全水印与取证叠加层流水线」 |

---

## 动机

1. **远程协助中的物理控制冲突与指引脱节**：在远程技术支持、外包协同开发、代码评审（Pair Programming）与培训演练中，协助者（Visitor）往往需要精准指出被控端（Host）屏幕上的特定代码行、配置项或故障按钮。以往若直接使用物理输入隧道抢夺鼠标光标，极易与被控端操作者发生「光标争抢」，甚至产生误触破坏生产系统的风险；
2. **轻量非侵入式协同表达通道（Virtual Laser Pointer & Whiteboard Overlay）**：协助者需要一种「只动笔不动手、只指引不争抢」的非侵入式视觉引导手段：
   - **瞬态虚拟激光笔**：协助者移动指针时以鲜艳高亮光斑及径向光晕呈现位置，按住时常亮，释放后自动消隐；
   - **几何标注与白板圈注**：支持自由线条、矩形框选、指示箭头以及基于点阵字模的短文本批注；
   - **生命周期衰减（TTL Auto-Fadeout）与一键清空**：支持设置临时标注存活毫秒数，超时后自动消除，杜绝遗留冗余视觉杂讯；双方随时可一键清空画布；
3. **零重型外部依赖铁律（Zero External Crates, ADR-006）**：严格禁止引入 `cairo`、`skia`、`tiny-skia` 等重型矢量图形依赖。在 `tetherly-core` 内部实现纯算法光栅化直线扫描、中点圆光晕与箭头几何计算；
4. **定长线协议与低延迟传输**：高频激光笔坐标（如 60Hz 采样）必须通过零内存分配的 14 字节定长信封（Tag 11）高效穿透，避免传统矢量网络协议引发的内存抖动与时延累积。

---

## 决定

### 1. 核心光栅化几何渲染算法与数据模型（`crates/tetherly-core/src/screen.rs`）

- **矢量标注图元模型 (`AnnotationKind` & `AnnotationItem`)**：
  ```rust
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  #[serde(rename_all = "lowercase")]
  pub enum AnnotationKind {
      Stroke { points: Vec<(u16, u16)> },
      Rect { x: u16, y: u16, w: u16, h: u16 },
      Arrow { from: (u16, u16), to: (u16, u16) },
      Text { x: u16, y: u16, text: String, scale: u8 },
  }

  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct AnnotationItem {
      pub id: u32,
      pub author: String,
      pub kind: AnnotationKind,
      pub color_rgb: [u8; 3],
      pub thickness: u8,
      pub alpha: u8,
      pub ttl_ms: u32,
      pub created_at_ms: u64,
  }
  ```
- **布雷森汉姆粗线段扫描算法 (`draw_line_thick`)**：
  - 核心整数增量计算：$e_2 = 2 \times err$，通过误差累加实现无除法象限步进；
  - 针对厚度 $T > 1$，以线段中心向外围绕 $(px, py)$ 填充半径为 $T/2$ 的像素块，调用定点 Alpha 混合器写入当前像素。
- **中点圆光斑算法与发光晕染 (`draw_laser_spot`)**：
  - 激光笔数据结构：
    ```rust
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct LaserPointerState {
        pub x: u16,
        pub y: u16,
        pub active: bool,
        pub color_rgb: [u8; 3],
        pub radius: u8,
    }
    ```
  - 内核高亮区（半径 $R_{core} = \max(2, R/3)$）：以全透明度或极高透明度写入高亮色（如纯白/亮红）；
  - 外晕渐变区（$R_{core} < r \le R$）：Alpha 按定点比例衰减：
    $$\alpha_{eff} = \alpha \times \frac{(R - r)^2}{(R - R_{core})^2}$$
    形成自然的边缘激光发光质感。
- **30度指引箭头几何计算 (`draw_arrow`)**：
  - 由起点向终点绘制主干粗线；
  - 在终点向后计算 30 度角两侧延伸线段，构造鲜明的指引箭头。

### 2. 线协议控制信令扩展与专属数据帧

- **Tag 11 (`ControlMsg::SetLaserPointer`)**：
  - 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）；
  - 64 位负载布局：
    - `0..16`: `x: u16`
    - `16..32`: `y: u16`
    - `32..40`: `active: u8` (0 或 1)
    - `40..48`: `r: u8`
    - `48..56`: `g: u8`
    - `56..64`: `b: u8`
  - 零内存分配、极高频次流转，支持随动激光笔 60 FPS 顺滑指引。
- **Tag 12 (`ControlMsg::ClearAnnotations`)**：
  - 一键清空全量标注信令。
- **专属数据帧 `TYPE_SCREEN_ANNOTATION = 0x050a`（魔数 `b"TANN"`）**：
  - 承载 JSON 编码的单个图元增量新增（`AnnotationAction::Add`）或全量同步。

### 3. 节点运行时管线与全生命周期挂接（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载：
  - `screen_annotations: Arc<RwLock<Vec<AnnotationItem>>>`；
  - `screen_laser_pointer: Arc<RwLock<Option<LaserPointerState>>>`；
  - `screen_annotation_count: AtomicU64`；
  - `screen_laser_updates: AtomicU64`。
- 自动生命周期过滤：在每次呈现或快照前，按当前绝对毫秒时间戳过滤掉已过期的 `ttl_ms` 标注；
- 呈现与快照：在 `screen_sink.present`、`screen_playback` 与 `screen_snapshot` 中调用 `apply_annotations` 与 `apply_laser_pointer` 进行像素叠加；
- 隐私模式安全优先级：若 `screen_privacy_enabled` 为真，全屏全黑覆盖优先执行，绝不泄漏底层画面。

### 4. 回环 REST 控制面与双端 CLI 全同构

- **REST API**：
  - `GET /api/screen/annotation`：获取当前活跃标注列表与状态报告；
  - `POST /api/screen/annotation`：添加标注图元；
  - `POST /api/screen/annotation/clear` / `DELETE /api/screen/annotation`：一键清空标注；
  - `GET /api/screen/laser` / `POST /api/screen/laser`：查询与更新激光笔；
- **双端 CLI 同构命令**：
  - `screen annotate list [--json] [--ui-port <port>]`
  - `screen annotate add stroke|rect|arrow|text ...`
  - `screen annotate clear [--json] [--ui-port <port>]`
  - `screen laser <x> <y> [--color <hex>] [--radius <r>] [--off] [--json] [--ui-port <port>]`
  - `screen laser status [--json] [--ui-port <port>]`
  - `screen status` 包含标注与激光笔遥测数据。

---

## 验证与测试准则

1. 核心单测：`crates/tetherly-core/src/screen.rs` 验证线段、矩形、箭头、文字绘制像素偏移，验证激光笔径向光晕定点衰减，验证 Tag 11 与 Tag 12 信令 roundtrip 编解码；
2. REST 单测：`crates/tetherly-node/src/uihttp.rs` 验证标注增删查、清空、激光笔坐标更新、Host 403 严格防御；
3. E2E 集成测试：`tests/phase5.rs::m5_29_screen_collaborative_annotation_and_laser_pointer_pipeline` 验证双节点配对下，激光笔与标注跨网实时同步呈现、快照提取像素验证、TTL 自动消隐与清空复位。
