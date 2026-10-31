# ADR-118 屏幕串流动态注视点焦点感知、空间区域优先级与外围自适应抽帧调度流水线与双端 CLI 全同构 (Screen Streaming Foveated Focus & Spatial Region-Priority Scheduling Pipeline with 100% Dual CLI Isomorphism)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-29，v2.32 / v0.1.54）**——建立屏幕串流动态注视点焦点感知、空间区域优先级与外围自适应抽帧调度流水线：(1) **人眼注视点感知与分级空间优先级调度模型**（`crates/tetherly-core/src/screen.rs`）：定义 `ScreenFocusMode`（`Cursor` 光标跟踪、`Box` 静态框）、`ScreenFocusConfig` 与 `ScreenFocusReport`；将全屏幕划分为注视焦点区域（Focus Zone，优先级 0）与外围背景区域（Peripheral Zone，优先级 1）；(2) **光标几何距离与注视半径算法 (`is_tile_in_focus`)**：根据实时硬件光标物理坐标 $(cx, cy)$ 或指定矩形框判定空间脏瓦片（ADR-116/117）归属；焦点区域内瓦片享有 100% 绝对刷新保障与零抽帧投递（60 FPS 无阻滞交互）；(3) **外围背景自适应时域抽帧调度**：外围区域脏瓦片依据抽帧率 $S$（`peripheral_skip_ratio`，默认 2，即每 3 帧仅派发 1 帧，稀疏化 66.7%）进行节拍调度；(4) **数学级画布一致性守恒定理（Canvas Invariance）**：外围被抽帧跳过的脏瓦片在发送端参考画布中保持原样、不予覆写，确保后续周期被自动提取重传，双端渲染画布保持 100% 逐位像素精确还原；(5) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 45（`ControlMsg::SetScreenFocusMode { enabled: bool, mode: u8, radius: u16, peripheral_skip_ratio: u8 }`）、Tag 46（`ControlMsg::SetScreenFocusBox { box_x: u16, box_y: u16, box_w: u16, box_h: u16 }`）、Tag 47（`ControlMsg::ResetScreenFocusMode`）、Tag 48（`ControlMsg::FocusSync { focus_tiles_sent: u64 }`）与 Tag 49（`ControlMsg::FocusPeripheralSync { peripheral_skipped: u64 }`）；(6) **节点运行时状态维护与网络循环协同**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_focus_config` 及原子计数器（`focus_tiles_sent`、`peripheral_tiles_sent`、`peripheral_tiles_skipped`、`focus_saved_bytes`），在 `screen_send_frame()` 阶段动态过滤瓦片集合并精准更新发送端参考画布；(7) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/focus`（GET / POST）、`/api/screen/focus/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充注视点焦点遥测指标，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(8) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen focus [status|set|reset]` 子命令族与格式化报表输出，`screen status` 扩充 Focus 优先级流控遥测；(9) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_42_screen_foveated_focus_priority_scheduling_pipeline` 全链路验证。 |
| 日期 | 2026-10-29 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-082「屏幕串流动态速率自适应 AIMD 控制器」、ADR-083「屏幕串流动态帧率与自适应定步控制」、ADR-088「硬件光标解耦与独立流控通道」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-109「屏幕串流动态差分补丁与宏块缓存流水线」、ADR-111「屏幕串流客观 QoE 自主闭环调优流水线」、ADR-112「屏幕串流渐进式帧内平滑刷新与去脉冲流控流水线」、ADR-113「屏幕串流长期参考帧 (LTR) 锚定与增量断洞自愈流水线」、ADR-114「屏幕串流双层时域分级预测 (SVC-T) 层次化参考流水线」、ADR-115「屏幕串流窗口滚动 2D 运动矢量估计与位移补偿加速流水线」、ADR-116「屏幕串流空间分块网格划分与独立异步瓦片更新流水线」、ADR-117「屏幕串流哈希瓦片字典缓存、全局跨帧空间去重与零带宽复用流水线」 |

---

## 动机

1. **人眼视觉注意力高度聚焦（Foveated Focus & Attention Center）**：
   - 在高分辨率（1080p/2K/4K）与高瓦片网格密度（ADR-116/117）桌面串流环境下，人类操作者的视觉认知与生理注意力高度汇聚于焦点区域——即鼠标硬件光标周边（输入交互点、文字录入光标、拖拽握柄）或当前活动的单应用窗口（焦点矩形）；
   - 屏幕其余 70%~90% 的面积属于外围背景区域（低关注度边角、后台终端日志滚动、系统托盘动画、状态栏时钟跳动、侧边栏微动）。

2. **全屏均质流控瓶颈（Uniform Scheduling Bottleneck）**：
   - 在现有空间分块（ADR-116）与瓦片字典去重（ADR-117）中，所有检测出的脏瓦片均采用同质无差别调度策略；
   - 一旦弱网或高负载引发网络队列积压或拥塞丢帧（`screen queue full; dropping frame`），全局丢帧会使得用户正在操作的核心焦点区域（例如光标拖曳与打字反馈）一同发生严重迟滞；
   - 即使网络通畅，全屏均质刷新也会在背景微动场景下持续耗费发送端 CPU 压缩与网络定步资源。

3. **核心解法：注视点焦点感知与空间区域优先级自适应调度流水线**：
   - **焦点区域（Focus Zone / 优先级 0）**：根据鼠标硬件光标实时物理坐标 $(cx, cy)$ 动态锁定注视半径 $R$（或用户配置的静态焦点框），焦点内的脏瓦片享有 100% 绝对刷新保障（如 60 FPS、零抽帧、优先编码与发送）；
   - **外围区域（Peripheral Zone / 优先级 1）**：焦点外的脏瓦片按照自适应抽帧率 $S$（`peripheral_skip_ratio`，默认 2，即每 3 帧刷新 1 次，抽帧率 66.7%）进行时域稀疏化调度；
   - **画布一致性守恒定理（Canvas Invariance）**：外围被抽帧跳过的脏瓦片在发送端参考画布中**不予更新**，确保下一次调度周期被稳定提取重传，双端画布始终保持 100% 逐位像素精确一致；
   - 弱网与高动态背景场景下，外围带宽消耗降低 60%~80%，交互焦点延迟维持在极低物理极限，彻底消除打字与鼠标拖拽迟滞。

4. **系统约束与安全红线**：
   - 严格遵循 ADR-006 零外部重型图形依赖，全部几何计算使用纯标准库基础代数；
   - 遵循 ADR-004 对端主权与安全强隔离原则，所有回环控制面端点严格绑定 127.0.0.1 并强制 Host 请求头校验，非法访问返回 403 Forbidden；
   - 严格保持双端 CLI（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）100% 同构。

---

## 决定

### 1. 焦点流控数据模型与几何判定（`crates/tetherly-core/src/screen.rs`）

- **常量与限制**：
  ```rust
  pub const SCREEN_FOCUS_DEFAULT_RADIUS: u16 = 256;
  pub const SCREEN_FOCUS_MIN_RADIUS: u16 = 64;
  pub const SCREEN_FOCUS_MAX_RADIUS: u16 = 2048;
  pub const SCREEN_FOCUS_DEFAULT_SKIP_RATIO: u8 = 2;
  pub const SCREEN_FOCUS_MAX_SKIP_RATIO: u8 = 10;
  ```

- **数据模型**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
  #[serde(rename_all = "snake_case")]
  pub enum ScreenFocusMode {
      #[default]
      Cursor,
      Box,
  }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenFocusConfig {
      pub enabled: bool,
      pub mode: ScreenFocusMode,
      pub radius: u16,
      pub peripheral_skip_ratio: u8,
      pub box_x: u16,
      pub box_y: u16,
      pub box_w: u16,
      pub box_h: u16,
  }

  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenFocusReport {
      pub ok: bool,
      pub enabled: bool,
      pub mode: ScreenFocusMode,
      pub radius: u16,
      pub peripheral_skip_ratio: u8,
      pub box_x: u16,
      pub box_y: u16,
      pub box_w: u16,
      pub box_h: u16,
      pub focus_tiles_sent: u64,
      pub peripheral_tiles_sent: u64,
      pub peripheral_tiles_skipped: u64,
      pub saved_bandwidth_bytes: u64,
  }
  ```

- **几何判定算法**：
  ```rust
  pub fn is_tile_in_focus(
      tile_x: u16,
      tile_y: u16,
      tile_w: u16,
      tile_h: u16,
      config: &ScreenFocusConfig,
      cursor_pos: Option<(i32, i32)>,
  ) -> bool {
      let center_x = tile_x as i32 + (tile_w as i32 / 2);
      let center_y = tile_y as i32 + (tile_h as i32 / 2);
      match config.mode {
          ScreenFocusMode::Cursor => {
              let (cx, cy) = cursor_pos.unwrap_or((0, 0));
              let dx = center_x - cx;
              let dy = center_y - cy;
              let dist_sq = (dx * dx + dy * dy) as u64;
              let r = config.radius as u64;
              dist_sq <= r * r
          }
          ScreenFocusMode::Box => {
              let bx1 = config.box_x as i32;
              let by1 = config.box_y as i32;
              let bx2 = bx1 + config.box_w as i32;
              let by2 = by1 + config.box_h as i32;
              center_x >= bx1 && center_x < bx2 && center_y >= by1 && center_y < by2
          }
      }
  }
  ```

### 2. 定长线协议信令（`ControlMsg`）

- 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中分配：
  - **Tag 45 (`ControlMsg::SetScreenFocusMode { enabled: bool, mode: u8, radius: u16, peripheral_skip_ratio: u8 }`)**：
    - `seq` 编码：`enabled (bit 0)` | `mode (bit 8..16)` | `radius (bit 16..32)` | `skip_ratio (bit 32..40)`；
  - **Tag 46 (`ControlMsg::SetScreenFocusBox { box_x: u16, box_y: u16, box_w: u16, box_h: u16 }`)**：
    - `seq` 编码：`box_x` | `(box_y << 16)` | `(box_w << 32)` | `(box_h << 48)`；
  - **Tag 47 (`ControlMsg::ResetScreenFocusMode`)**：一键重置为默认关闭状态；
  - **Tag 48 (`ControlMsg::FocusSync { focus_tiles_sent: u64 }`)**：同步焦点瓦片数；
  - **Tag 49 (`ControlMsg::FocusPeripheralSync { peripheral_skipped: u64 }`)**：同步外围抽帧数。

### 3. 节点运行时调度集成（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载 `screen_focus_config` 与相关统计原子量；
- 在 `screen_send_frame()` 阶段：
  - 提取脏瓦片后，若 `focus_cfg.enabled`，逐一调用 `is_tile_in_focus`；
  - 焦点瓦片保留加入发送队列；
  - 外围瓦片若处于非刷新节拍（`frame.seq % (skip_ratio + 1) != 0`），将其剔除出本帧发送队列，同时累加 `peripheral_tiles_skipped` 与 `saved_bytes`；
  - 参考画布局部更新：仅将本帧实际发送成功的瓦片贴入发送端参考画布 `screen_sender_tile_canvas`，被跳过的外围瓦片像素保留为旧像素，等待下一个刷新周期自动触发差分补发。

### 4. 安全回环 REST 控制面（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/focus`
- `POST /api/screen/focus`（参数：`enabled`, `mode`, `radius`, `peripheral_skip_ratio`, `box_x`, `box_y`, `box_w`, `box_h`）
- `POST/DELETE /api/screen/focus/reset`
- `ScreenStatusReport` 扩充注视点焦点遥测字段；
- 严格遵循 127.0.0.1 物理绑定与 Host 403 强隔离。

### 5. 双端 CLI 100% 全同构

- `screen focus status [--json] [--ui-port <port>]`
- `screen focus set [--enabled <bool>] [--mode <cursor|box>] [--radius <u16>] [--skip <u8>] [--box-x <u16>] [--box-y <u16>] [--box-w <u16>] [--box-h <u16>] [--json] [--ui-port <port>]`
- `screen focus reset [--json] [--ui-port <port>]`
- `screen status` 输出显示 Focus 优先级流控状态。

---

## 结论与价值

1. **零迟滞核心交互保障**：鼠标光标周边区域保持绝对全帧率刷新，即使在弱网或重度背景动效干扰下，输入和交互体验依然流畅丝滑；
2. **背景带宽大幅缩减**：外围背景按需稀疏化调度，节省 60%~80% 的无效背景带宽消耗；
3. **逐位数学精确还原**：通过参考画布差异保持机制，确保画面即使抽帧也绝不出现累积花屏或坏块，所有区域在刷新节拍到达时 100% 逐位像素精确同步。
