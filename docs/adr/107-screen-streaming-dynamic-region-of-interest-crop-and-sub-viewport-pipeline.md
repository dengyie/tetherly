# ADR-107 屏幕串流动态感兴趣区域 (ROI) 裁剪串流、局部缩放聚焦与视口漫游流水线（Screen Streaming Dynamic Region-of-Interest (ROI) Cropping, Sub-Viewport Zoom & Pan Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-28，v2.21 / v0.1.43）**——建立屏幕串流动态感兴趣区域 (ROI) 裁剪串流、局部缩放聚焦与视口漫游流水线：(1) **ROI 数据模型与行级行距裁剪算法**（`crates/tetherly-core/src/screen.rs`）：严格遵循 ADR-006 零重型外部依赖铁律，设计 `ScreenRoi` 与 `ScreenRoiReport`；实现边界安全夹取（Clamp）、偶数对齐（multiple of 2，适配 H.264/YUV 编解码）与极速 $O(\text{roi\_h} \times \text{roi\_w} \times 4)$ 内存切片；(2) **双向坐标映射与协作图层对齐**：实现 `map_screen_to_roi` 与 `map_roi_to_screen`，确保在视口裁剪激活状态下，激光笔（ADR-105）与白板标注（ADR-106）与局部视口像素精准重合对齐；(3) **线协议信令定长扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 16（`ControlMsg::SetScreenRoi { x, y, w, h }`）、Tag 17（`ControlMsg::ResetScreenRoi`）与 Tag 18（`ControlMsg::SetScreenRoiZoom { zoom_percent }`）；(4) **节点运行时采集流控与网络分发挂载**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_roi: Arc<RwLock<ScreenRoi>>`，在 `screen_send_frame` 帧抓取后直接对未压缩原始帧切片裁剪后送入编码器，节约高达 75% 以上传输带宽与编码算力；(5) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/roi`（GET / POST）、`/api/screen/roi/reset`（POST/DELETE）、`/api/screen/roi/zoom`（POST）端点；集成 Loopback 绑定与 Host 403 严格隔离；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen roi [status|set|reset|zoom]` 子命令族与表格排版格式化输出；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_31_screen_dynamic_roi_crop_and_viewport_zoom_pipeline` 全链路验证。 |
| 日期 | 2026-10-28 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-084「屏幕串流动态分辨率降采样与流控自适应流水线」、ADR-097「屏幕串流多显示器枚举与动态切屏流水线」、ADR-104「屏幕串流动态安全水印与取证叠加层流水线」、ADR-105「屏幕串流实时协作标注与瞬态虚拟激光笔流水线」、ADR-106「屏幕串流多方协作共享画板与矢量快照导入导出流水线」 |

---

## 动机

1. **高分屏与小屏幕端的显示鸿沟（High-DPI / Multi-K Host vs Compact Visitor Viewport Mismatch）**：在远程协助、远程开发、结对编程（Pair Programming）与技术支持中，被控端（Host）通常采用 4K（3840x2160）、2K（2560x1440）或带鱼屏，而访客端（Visitor）可能在 1080p 笔记本电脑、平板甚至移动端上运行。若将整个 4K 画面下采样推流，代码行、终端字符和细小控件将严重模糊难以辨识；
2. **带宽消耗与算力浪费**：整屏高分辨率编码不仅消耗 15~40 Mbps 的高额带宽，更对 CPU/GPU 带来巨大的编码压力；
3. **局部感兴趣区域（Region of Interest, ROI）动态裁剪与 1:1 无损聚焦（Selective Viewport Focus）**：用户需要能够指定或动态拖拽视口矩形（`x, y, w, h`），使串流管线仅裁剪并编码该 ROI 子区域，以 1:1 原生无损物理像素传输关键代码与调试控制台，同时支持视口缩放（Zoom 50%~800%）与平移漫游（Pan）；
4. **协作图层视口坐标自动对齐**：在视口裁剪激活时，虚拟激光笔（ADR-105）和多方画板（ADR-106）必须精准在视口内投影，不能因裁剪而产生坐标错位；
5. **零外部重型图形依赖（Zero External Crates, ADR-006）**：严格禁止引入 OpenCV、ImageMagick 等重型依赖。在 `tetherly-core` 内以纯标准库整数边界安全夹取（Clamp to screen width/height，偶数对齐适配 H.264/YUV）与行级跨度提取完成高效 $O(\text{roi\_h} \times \text{roi\_w} \times 4)$ 内存切片；
6. **全链路协同与双端 CLI 100% 同构**：ROI 视口控制无缝集成到采集流水线（`screen_source`）、呈现渲染（`screen_sink`）、回放管线（`screen_playback`）、单帧快照（`screen_snapshot`）、回环 REST API 以及双端 CLI（`tetherly` 与 `tetherly-cli`）。

---

## 决定

### 1. ROI 数据模型与核心引擎（`crates/tetherly-core/src/screen.rs`）

- **数据模型**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
  pub struct ScreenRoi {
      pub active: bool,
      pub x: u16,
      pub y: u16,
      pub w: u16,
      pub h: u16,
      pub zoom_percent: u16, // 100 = 100% (1.0x), 200 = 200% (2.0x)
  }

  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub struct ScreenRoiReport {
      pub active: bool,
      pub x: u16,
      pub y: u16,
      pub w: u16,
      pub h: u16,
      pub zoom_percent: u16,
      pub source_w: u16,
      pub source_h: u16,
      pub effective_w: u16,
      pub effective_h: u16,
      pub update_count: u64,
  }
  ```
- **核心操作与映射方法**：
  - `sanitize(screen_w, screen_h)`：边界防护与偶数尺寸规范化；
  - `map_screen_to_roi(sx, sy)`：将全局屏幕坐标变换为当前视口局部坐标；
  - `map_roi_to_screen(rx, ry)`：将视口局部坐标还原为全局屏幕坐标；
  - `crop_roi(&roi)`：对未压缩 BGRA8/RGBA8 像素执行行级跨度零多余拷贝裁剪切片。

### 2. 14 字节定长信封线协议扩充（`TYPE_SCREEN_CONTROL = 0x0502`）

- **Tag 16 (`ControlMsg::SetScreenRoi { x: u16, y: u16, w: u16, h: u16 }`)**：
  - 64 位紧凑载荷：bits 0..16: `x`, bits 16..32: `y`, bits 32..48: `w`, bits 48..64: `h`；
- **Tag 17 (`ControlMsg::ResetScreenRoi`)**：
  - 64 位载荷置 0，立刻恢复全屏推流；
- **Tag 18 (`ControlMsg::SetScreenRoiZoom { zoom_percent: u16 }`)**：
  - 64 位载荷存储 `zoom_percent as u64`。

### 3. 节点运行时管线与生命周期挂接（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载 `screen_roi: Arc<RwLock<ScreenRoi>>` 与 `screen_roi_updates: Arc<AtomicU64>`；
- `screen_send_frame` 捕获到原始帧后，若 `roi.active` 判定为真，直接对原始帧调用 `frame.crop_roi(&roi)?` 进行裁切，仅将局部关键视口送入压缩与传输队列；
- `screen_server_loop` 与 `screen_client_loop` 双向路由 Tag 16、17、18。

### 4. 回环 REST 控制面与双端 CLI 全同构

- **REST API**：
  - `GET /api/screen/roi`：获取 ROI 状态画像；
  - `POST /api/screen/roi`：设置视口裁剪区域与缩放比例；
  - `POST /api/screen/roi/reset` 与 `DELETE /api/screen/roi`：重置并恢复全屏；
  - `POST /api/screen/roi/zoom`：调节缩放百分比；
- **双端 CLI 同构命令**：
  - `screen roi status [--json] [--ui-port <port>]`
  - `screen roi set --x <u16> --y <u16> --w <u16> --h <u16> [--zoom <percent>] [--json] [--ui-port <port>]`
  - `screen roi reset [--json] [--ui-port <port>]`
  - `screen roi zoom <percent> [--json] [--ui-port <port>]`

---

## 验证与测试准则

1. 核心单测：`crates/tetherly-core/src/screen.rs` 验证 `ScreenRoi` 边界规范化、正反坐标映射、帧行级裁剪切片、Tag 16/17/18 编解码 roundtrip；
2. REST 单测：`crates/tetherly-node/src/uihttp.rs` 验证 `/api/screen/roi/*` 端点（设置、重置、缩放、状态画像与 Host 403 严格隔离）；
3. E2E 集成测试：`tests/phase5.rs::m5_31_screen_dynamic_roi_crop_and_viewport_zoom_pipeline` 验证双节点配对下，动态 ROI 视口裁剪信令到端同步、宿主机裁剪 32x32 至 16x16 帧投递验证、缩放更新及复位全屏恢复。
