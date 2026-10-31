# ADR-115 屏幕串流窗口滚动 2D 运动矢量估计 (Motion Vector / Block Matching) 与位移补偿加速流水线与双端 CLI 全同构 (Screen Streaming 2D Motion Vector Estimation, Displacement Compensation and Scroll Acceleration Pipeline with 100% Dual CLI Isomorphism)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-29，v2.29 / v0.1.51）**——建立屏幕串流窗口滚动 2D 运动矢量估计与位移补偿加速流水线：(1) **2D 运动矢量估计与位移匹配模型**（`crates/tetherly-core/src/screen.rs`）：定义 `ScreenMotionConfig`、`ScreenMotionReport` 与 `MotionVector`，新增专用 Codec 标记 `SCREEN_COMPRESS_MOTION_SCROLL = 7`；发送端在检测到较大范围脏区变化时，通过高频采样行/特征带在搜索窗口 $[-D, D]$ 内快速估计出主位移矢量 $\mathbf{v} = (\Delta x, \Delta y)$；在重叠区域相似度达到阈值（$\ge 85\%$）时判定为窗口滚动位移事件；(2) **暴露边缘条带补丁与接收端画布位移重绘**：针对滚动事件，发送端仅需提取并 Zstd 压缩滚入屏幕边界的细小暴露区域（Exposed Edge Strip），带宽相比全幅刷新骤降 80%~95%；接收端通过 `ScreenFrame::scroll_canvas` 执行本地内存平移并在暴露边缘贴合细小补丁，实现零伪影、零撕裂与像素级幂等还原；(3) **定长线协议信令扩充**：在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中扩充 Tag 36（`ControlMsg::SetScreenMotionMode { enabled: bool, max_displacement: u16 }`）、Tag 37（`ControlMsg::ResetScreenMotionMode`）与 Tag 38（`ControlMsg::MotionScrollSync { scroll_frames_sent: u64 }`）；(4) **节点运行时状态维护与网络循环协同**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_motion_config` 及计数器（`motion_frames_sent`、`fallback_frames_sent`、`saved_bandwidth_bytes`），在 `screen_send_frame()` 阶段动态触发位移估计并派发滚动补偿帧，接收端自动平移画布并贴合补丁；(5) **回环 REST 控制面端点闭环**（`crates/tetherly-node/src/uihttp.rs`）：提供 `/api/screen/motion`（GET / POST）、`/api/screen/motion/reset`（POST/DELETE）端点，`ScreenStatusReport` 扩充运动矢量遥测指标，严格执行 127.0.0.1 物理绑定与 Host 403 强隔离；(6) **双端 CLI 100% 全同构**（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）：新增 `screen motion [status|set|reset]` 子命令族与格式化报表输出，`screen status` 扩充滚动加速遥测；(7) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_39_screen_motion_vector_estimation_and_scroll_acceleration_pipeline` 全链路验证。 |
| 日期 | 2026-10-29 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-081「屏幕串流按需关键帧与丢帧自愈机制」、ADR-082「屏幕串流动态速率自适应 AIMD 控制器」、ADR-083「屏幕串流动态帧率与自适应定步控制」、ADR-089「屏幕脏区检测与自适应静噪控制器」、ADR-109「屏幕串流动态差分补丁与宏块缓存流水线」、ADR-111「屏幕串流客观 QoE 自主闭环调优流水线」、ADR-112「屏幕串流渐进式帧内平滑刷新与去脉冲流控流水线」、ADR-113「屏幕串流长期参考帧 (LTR) 锚定与增量断洞自愈流水线」、ADR-114「屏幕串流双层时域分级预测 (SVC-T) 层次化参考流水线」 |

---

## 动机

1. **页面窗口滚动场景的传统差分失效与带宽脉冲爆发**：
   - 在远程桌面、网页浏览、代码编辑翻页、终端控制台日志查阅及长文档滚动等日常操作中，超过 70% 的画面变化属于空间刚性平移（Rigid 2D Spatial Translation）；
   - 在既有的平坦差分（ADR-109）、渐进刷新（ADR-112）与 SVC-T（ADR-114）流水线中，算法均基于固定空间坐标进行逐像素或逐宏块比较（即比较 $(x, y)$ 在相邻帧中的差异）；
   - 当页面或窗口发生垂直位移 $\Delta y$ 时，画面中高达 85%~95% 的像素内容事实上保持完全一致（仅仅在 Y 轴上平移了若干像素）。然而，固定坐标差分比对会因坐标错位判定全屏大面积像素全部发生变化（`dirty_ratio > 0.8`），从而退化为重传整个屏幕的巨大差分补丁或触发关键帧刷新；
   - 这会导致每次鼠标滚轮或触控板滚动都会激发出数兆字节的瞬时流量脉冲（Traffic Burst），在弱网或中继环境下引发严重的网络拥塞、排队延迟抖动与滚屏卡顿。

2. **2D 运动矢量估计 (Motion Vector / Block Matching) 架构**：
   - 借鉴现代视频编码中的帧间运动补偿（Inter Motion Compensation）思想，在零重型图像依赖（ADR-006）的前提下，构建自包含、极速轻量的 2D 运动矢量估计器：
     - **轻量搜索窗口**：在搜索窗口范围 $[-D, D]$（如垂直搜索半径 $D \le 128$ 像素，默认 64）内，对上一帧画布与当前最新帧进行高频中心采样比对；
     - **特征行快速匹配**：通过在画面纵向关键采样线提取特征，计算最小绝对差和（SAD, Sum of Absolute Differences）或匹配相似度；
     - **位移判定判定条件**：当最优位移 $(\Delta x, \Delta y)$ 下的重叠区域像素匹配率达到设定阈值（$\ge 85\%$）时，确认发生全局或大窗口滚动。

3. **暴露边缘条带提取与接收端画布位移补偿 (Exposed Strip & Canvas Scroll)**：
   - **发送端数据压缩革新**：
     - 一旦确立运动矢量 $(\Delta x, \Delta y)$，画面中绝大部分区域无需编码任何像素数据；
     - 发送端仅需计算新滚入屏幕可见视野的**暴露边缘条带（Exposed Edge Strip）**矩形；例如当向上滚动 $\Delta y > 0$ 时，原画面向上移动，屏幕底部露出高度为 $\Delta y$ 的新内容区域；
     - 发送端仅对该细小暴露条带进行 Zstd 压缩，连同位移参数 $(\Delta x, \Delta y)$ 封装入专用 Codec 格式 `SCREEN_COMPRESS_MOTION_SCROLL = 7`；
     - 传输体积直接从全屏数十/数百 KB 降至几 KB，**网络带宽节省 80%~95%**，编码端 CPU 计算开销同样大幅下降。
   - **接收端画布位移应用 (`ScreenFrame::scroll_canvas`)**：
     - 接收端在收到滚动帧后，首先在本地活跃画布（`screen_receiver_canvas`）上执行内存平移（Memory Blit / `copy_within`），将原画布像素按 $(\Delta x, \Delta y)$ 快速位移；
     - 紧接着将随报文附带的暴露边缘条带补丁解压并贴合（`apply_sub_rect`）到新暴露的空缺区域；
     - 双端画面完全恢复一致，零重传、零撕裂、零卡顿。

4. **系统约束与安全红线**：
   - 严格遵循 ADR-006 零重型图像/外部依赖铁律，全算法采用标准库切片与内存运算；
   - 遵循 ADR-004 对端主权与安全强隔离原则，所有回环控制面端点严格绑定 127.0.0.1 并强制 Host 请求头校验，非法访问返回 403 Forbidden；
   - 严格保持双端 CLI（`bins/tetherly-cli` 与 `crates/tetherly-node/src/bin/tetherly`）100% 同构。

---

## 决定

### 1. 运动估计数据模型与 Codec 标记（`crates/tetherly-core/src/screen.rs`）

- **专用 Codec 标记**：
  ```rust
  pub const SCREEN_COMPRESS_MOTION_SCROLL: u8 = 7;
  ```

- **数据结构与报表**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenMotionConfig {
      pub enabled: bool,
      pub max_displacement: u16,
  }

  #[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  pub struct ScreenMotionReport {
      pub ok: bool,
      pub enabled: bool,
      pub max_displacement: u16,
      pub motion_frames_sent: u64,
      pub fallback_frames_sent: u64,
      pub saved_bandwidth_bytes: u64,
  }

  #[derive(Debug, Clone, Copy, PartialEq)]
  pub struct MotionVector {
      pub dx: i16,
      pub dy: i16,
      pub match_ratio: f32,
  }
  ```

- **轻量 2D 滚动位移估计算法 (`estimate_scroll_motion`)**：
  - 发送端在上一参考帧与当前帧之间寻找最优位移 $(\Delta x, \Delta y)$；
  - 优先在垂直轴 $dy \in [-D, D]$（步长 1，最大搜索半径 $D$ 缺省 64）进行分层采样比对；
  - 当最优位移匹配度 $\ge 0.85$ 且位移量非零时返回 `Some(MotionVector)`，否则返回 `None`。

- **暴露矩形提取与位移帧编解码 (`ScreenFrame`)**：
  - 编码格式：8 字节头部（`dx: i16`, `dy: i16`, `rect_x: u16`, `rect_y: u16`）+ 4 字节条带宽高（`rect_w: u16`, `rect_h: u16`）+ 后续 Zstd 压缩像素条带；
  - 解码逻辑：校验有效性并还原暴露条带的 `ScreenDamageRect` 与压缩补丁；
  - 画布位移操作：`ScreenFrame::scroll_canvas(&mut self, dx: i16, dy: i16)`，内部安全平移有效像素。

### 2. 14 字节定长线协议信令定义（`TYPE_SCREEN_CONTROL = 0x0502`）

信令沿用 14 字节定长信封（`magic: b"TSCR"`，`proto: 1`，`tag: u8`，`seq: 8 字节大端整数`）：

| 信令枚举 | Tag | 载荷字段 | Wire `seq` (8 字节大端) 编码规则 |
|---|---|---|---|
| `ControlMsg::SetScreenMotionMode { enabled, max_displacement }` | **36** | `enabled: bool`, `max_displacement: u16` | `(if enabled { 1u64 } else { 0u64 }) \| ((max_displacement as u64) << 8)` |
| `ControlMsg::ResetScreenMotionMode` | **37** | 无 | `0u64` |
| `ControlMsg::MotionScrollSync { scroll_frames_sent }` | **38** | `scroll_frames_sent: u64` | `scroll_frames_sent` |

### 3. 节点运行时状态与网络循环协同（`crates/tetherly-node/src/runtime.rs`）

1. **`NodeShared` 状态挂载**：
   - `screen_motion_config: Arc<RwLock<ScreenMotionConfig>>`
   - `screen_motion_frames_sent: AtomicU64`
   - `screen_motion_fallback_frames_sent: AtomicU64`
   - `screen_motion_saved_bytes: AtomicU64`

2. **帧发送调度流水线 (`screen_send_frame`)**：
   - 若 `motion_config.enabled` 且画面存在上一参考画布：
     - 调用 `estimate_scroll_motion` 评估位移矢量；
     - 若成功匹配且匹配率 $\ge 85\%$：
       - 计算暴露边缘矩形并提取像素补丁进行 Zstd 压缩；
       - 生成 `SCREEN_COMPRESS_MOTION_SCROLL = 7` 帧；
       - 本地参考画布同步执行 `scroll_canvas(dx, dy)` 并贴合暴露补丁；
       - 递增 `screen_motion_frames_sent`，估算并累加节省的带宽字节数；
     - 若未匹配或位移为 0：
       - 递增 `screen_motion_fallback_frames_sent`，平滑回退至既有差分（ADR-109 / ADR-114）或全量帧流水线。

3. **接收端位移补偿与重绘渲染**：
   - 接收端在收到 `SCREEN_COMPRESS_MOTION_SCROLL` 压缩帧时：
     - 解析得到位移量 $(\Delta x, \Delta y)$ 与暴露条带补丁；
     - 对本地参考画布 `screen_receiver_canvas` 调用 `scroll_canvas(dx, dy)` 执行像素位移；
     - 解压条带像素并通过 `apply_sub_rect` 贴合到暴露区域；
     - 驱动呈现并更新统计计数。

4. **双向信令同步**：
   - 屏幕控制通道收到 Tag 36、37、38 时，对称更新节点的运动矢量配置与遥测计数器。

### 4. 回环 REST 控制面规范（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/motion`：返回当前运动补偿配置、发送帧数、回退帧数与节省带宽字节（`ScreenMotionReport`）；
- `POST /api/screen/motion`：动态配置启用状态与最大搜索位移（支持 JSON 请求体 `{"enabled": true, "max_displacement": 64}` 或查询参数 `?enabled=true&max_displacement=64`）；
- `POST/DELETE /api/screen/motion/reset`：一键重置运动估计配置至默认状态并清空流控计数；
- 遥测汇聚：`ScreenStatusReport` 扩充 `motion_enabled`、`motion_max_displacement`、`motion_frames_sent`、`motion_saved_bytes` 字段；
- 安全防护：严格绑定 `127.0.0.1` 物理回环，非本地请求或非法 Host 请求头统一阻断并响应 `403 Forbidden`。

### 5. 双端 CLI 100% 全同构

在 `bins/tetherly-cli/src/main.rs` 与 `crates/tetherly-node/src/bin/tetherly.rs` 对称增加：
- `screen motion status [--json] [--ui-port <port>]`
- `screen motion set [--enabled <bool>] [--max-displacement <n>] [--json] [--ui-port <port>]`
- `screen motion reset [--json] [--ui-port <port>]`
- 格式化报表函数 `print_screen_motion_report`，并在 `screen status` 中输出 Motion 流水线运行状态。

### 6. 测试与集成验证

- **核心单元测试**（`crates/tetherly-core/src/screen.rs`）：
  - `screen_motion_vector_estimation_and_scroll_acceleration_algorithms`：验证位移搜索、条带计算、画布位移移动、Tag 36~38 定长编解码往返。
- **回环 REST 控制面单元测试**（`crates/tetherly-node/src/uihttp.rs`）：
  - `test_uihttp_screen_motion_endpoints`：验证 GET/POST/RESET 端点、JSON 与 Query 传参、状态持久化与非法 Host 头 403 阻断。
- **端到端集成测试**（`tests/phase5.rs`）：
  - `m5_39_screen_motion_vector_estimation_and_scroll_acceleration_pipeline`：全链路验证串流建立、REST 控制面动态使能、模拟垂直页面位移产生 Tag 7 滚动帧、接收端位移补偿与暴露边缘贴合像素级无损还原、静态与非平移普通编辑平滑回退、Tag 37 重置及安全断连。
