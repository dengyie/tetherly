# ADR-133 跨端四向屏幕边缘漫游与 Deskflow 空间拓扑对齐 (Four-Directional Screen Roaming and Deskflow Topology Alignment)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-11-02，v2.47 / v0.1.69）**——将 Tetherly 边缘跨屏模型从单一水平向（Right 独占）全面升级为二维四向（Left / Right / Top / Bottom）空间几何漫游体系，完美对齐 Deskflow / Barrier / Synergy 的屏幕相对关系摆放（上下堆叠、左右并列），打通 `InputTunnelPolicy` 持久化、2D 拓扑布局自动联动与双端 CLI/REST 命令行配置，全工作区测试与端到端集成测试全绿。 |
| 日期 | 2026-11-02 |
| 关联 | ADR-071（输入通道安全策略与物理锁定）、ADR-072（跨端屏幕边缘漫游与光标状态机）、ADR-132（macOS 原生键鼠捕获/注入体系与跨平台键位重映射引擎） |

---

## 动机与背景

在 Deskflow（Barrier / Synergy）的典型日常多端桌面使用中，两台物理设备的显示器摆放具有多样化的空间关系：
1. **上下堆叠摆放**：笔记本（如 MacBook）放置在桌面下方作为副屏，大尺寸桌面显示器（Windows 工作站）放置在正上方；用户期望光标从笔记本顶部边缘滑出，直接进入上方显示器的底部。
2. **左右任意摆放**：副屏置于左侧，主屏置于右侧；用户期望向左滑动穿越屏幕，而不是强制从右侧穿越。

此前 Tetherly 的虚拟输入引擎（`InputServer` / `InputClient`）硬编码了右侧单向跨越模型：仅当 `x >= width` 时触发跨屏出境，且客户端固定从 `x = 0` 接收光标。这使得上下排列或左侧排列的工作台无法自然使用。

用户的明确需求：
> *"需要跟 deskflow 一样，有调整屏幕左右关系、上下关系的功能，不能默认左滑右滑。"*

为此，必须在保持生产机非侵入、零编译负载、零线协议（`TIN1`）二进制突变的约束下，将输入漫游引擎彻底泛化为支持自由配置上下左右关系的四向空间漫游体系。

---

## 架构与技术实现

### 1. 纯协议层二维四向空间几何模型 (`crates/tetherly-core/src/input.rs`)

将 `ScreenEdge` 枚举提升为核心协议层一等公民：
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScreenEdge {
    Left,
    Right,
    Top,
    Bottom,
}
```

实现几何对立边算子 `opposite()` 与标准化显示边 `DisplayEdge` 双向无损转换：
- `Left.opposite() == Right`，`Top.opposite() == Bottom`
- `DisplayEdge`（来自 2D 拓扑图）与 `ScreenEdge` 互转特化。

### 2. 对称出入境坐标映射与零协议突变 (`TIN1` Invariance)

为了不改变任何既有的 45719 二进制线协议格式，我们利用初次进入坐标动态推导机制：
- **出境边界判定与归一化（Server 端）**：
  - **Right 穿越**：`x >= width` 触发出境。远端入口坐标为 `(0, norm_y)`；远端从 `x <= 0` 返回时，本地光标重置至 `x = width - 1`。
  - **Left 穿越**：`x <= 0` 触发出境。远端入口坐标为 `(SCREEN_UNITS, norm_y)`（1000 逻辑刻度）；远端从 `x >= SCREEN_UNITS` 返回时，本地光标重置至 `x = 1`。
  - **Top 穿越**：`y <= 0` 触发出境。远端入口坐标为 `(norm_x, SCREEN_UNITS)`；远端从 `y >= SCREEN_UNITS` 返回时，本地光标重置至 `y = 1`。
  - **Bottom 穿越**：`y >= height` 触发出境。远端入口坐标为 `(norm_x, 0)`；远端从 `y <= 0` 返回时，本地光标重置至 `y = height - 1`。
- **自适应入口边界捕获（Client 端）**：
  `InputClient` 接收到 `InputKind::Enter` 后，在首个 `InputKind::Move { x, y }` 到达时动态捕获 `entry_edge`（若 `x <= 0` 则为 Left，`x >= 1000` 则为 Right，`y <= 0` 则为 Top，`y >= 1000` 则为 Bottom）。随后监听鼠标在远端屏幕内部的移动；一旦光标返回至该入射边界，立即合成 `InputKind::Leave` 帧归还光标。
  线协议结构保持 100% 不变。

### 3. 策略持久化与拓扑布局无缝联动

- **策略存储**：在 `crates/tetherly-node/src/store.rs` 的 `InputTunnelPolicy` 中持久化 `edge: Option<String>`（保存于 `input_policy.json`）。
- **优先级解析决策**：
  在 `drive_input` 与 `input_tunnel_attach` 中，漫游边界解析遵循自适应梯度：
  1. 若 `InputTunnelPolicy.edge` 显式配置（`top` / `bottom` / `left` / `right`），以此为最高优先级；
  2. 若未显式配置，自动检索 2D 拓扑图 `TopologyLayout.neighbors` 中对应对端设备的连接方向（如已通过 `tetherly topology link --edge top` 绑定）；
  3. 若均未配置，平滑兜底至 `ScreenEdge::Right` 保持历史兼容。

### 4. 控制面与 CLI 同构

- **HTTP REST 接口**：`/api/input/policy` 与 `/api/input/tunnel/attach` 全面支持 `edge` 字段的入参解析与输出展示。
- **CLI 命令行**：
  - `tetherly input policy --edge top`：将屏幕漫游方向切换为顶部（上屏漫游）；
  - `tetherly input policy --edge left`：将屏幕漫游方向切换为左侧；
  - `tetherly input policy` 查询回显包含：
    ```
    Virtual Peripheral & Input Tunnel Governance Policy:
      Enabled:              true
      Allow Inbound:        true
      Forward Keyboard:     true
      Forward Mouse:        true
      Forward Hotkeys:      true
      Idle Timeout (s):     300
      Keymap Profile:       mac_to_windows
      Edge Roaming:         top
    ```
  - `tetherly-cli` 与 `tetherly` 双二进制完全同构。

---

## 验证与验收

1. **协议层单元测试 (`crates/tetherly-core/src/input.rs`)**：
   - `screen_edge_from_str_and_opposite`：验证四向字符串解析与对立边变换；
   - `four_directional_edge_crossing_and_round_trip`：验证 Left、Right、Top、Bottom 四个方向的触发位置、入射归一化坐标、内部移动以及返程触发 Leave 全生命周期闭环。
2. **端到端集成测试 (`tests/phase3.rs`)**：
   - `m3_5_four_directional_edge_roaming`：验证通过策略配置 `--edge top` 后，本地光标触碰顶部边界自动出境、光标状态机迁入 Remote、受控端在底部（`y = 1000`）就绪捕获；
   - `m3_6_left_edge_roaming`：验证配置 `--edge left` 后，本地光标触碰左侧边界自动出境，受控端在右侧（`x = 1000`）就绪捕获。
3. **工作区质量门禁**：
   - `cargo fmt --all -- --check` 100% 格式对齐；
   - `cargo clippy --workspace --all-targets -- -D warnings` 零警告；
   - `cargo test --workspace` 全量 650+ 项单元测试与集成测试全部通过。

---

## 增补（2026-11-10，本地未发布）：Windows 控 Mac 漫游链路三根因修复

用户现场症状："Windows 鼠标控 Mac 部分屏幕到不了、乱跳或被拉回"。按证据驱动逐项 red→green，**Windows 条件编译代码未经真实 Windows 编译运行，物理验收仍 Manual-required**：

1. **core 位移分包敏感（`InputServer::local_move_delta`）**：逐事件 `delta * 1000 / dimension` 整除丢弃小数余量，且微小非零 delta 被强制 ±1——同一 192px 物理位移拆成 192 个 1px 事件映射 192 单位、单个 192px 事件仅映射 100 单位（red 证据），应用结果取决于事件分包方式，与"乱跳/到不了"直接相关。修复：`rem_frac_x/rem_frac_y`（i64）小数余量累加器，余量跨事件保留、随方向符号校正；`clear_stale_remote` / `force_local` / `resize` / `on_remote_leave` / `on_peer_gone` 五处状态转换重置余量，边界不积累隐形超额位移。**TIN1 `0..=1000` 绝对坐标线格式不变。** 回归：`remote_displacement_is_chunking_invariant_interior`（分包不变性）+ `remote_seat_survives_zero_delta_tail_and_resumes_on_motion`（零 delta 尾不虚构运动、真实运动恢复推进）。
2. **Mac `input_stop` 立即解除 armed**：接管降级原实现仅清引擎与 seat，macOS 侧 `arm_input_capture` 的鼠标去关联/光标隐藏/遮罩残留到下一次本地物理事件（注入远端 Move 经 magic 放行、不触发 capture worker 对齐尾）。`input_stop` 现同步调用 `disarm_input_capture()`；回归 `test_input_stop_releases_remote_capture_arming`。
3. **控制器漫游期共享边安全回中（本轮新增，Deskflow 无直接对应）**：控制器进入 Remote 后物理光标停在共享边缘（Windows 非中心停车），OS 将其夹限在 `y=height-1`（如 Bottom），LL hook 连续位置差恒为零——delta 链报告零位移，远端光标搁浅在入口边缘，直到手物理离开边界。新增纯几何 `platform::remote_roam_recenter_target`（按 `last_crossed_edge` 判定死区带、保留自由轴）与 `Node::maybe_recenter_roaming_cursor`（仅 Remote seat），capture worker 于每事件 `drive_input_delta` 后查询，命中即经 `warp_mouse_position` 回中；回中 echo 在同一 hook 线程被 pending-warp 门吞噬并重置 delta baseline（契约回放 `edge_park_recenter_keeps_delta_chain_alive`：饱和链→回中 echo 吞噬→零 delta settle→真实入内运动恢复推进）。Local / Target 永不回中。
4. **验证边界（如实记录）**：本机 macOS `cargo clippy -p tetherly-core -p tetherly-node --all-targets --locked` 零警告；`tetherly-core` 310+28 全通过；`tetherly-node --lib` 184/184（含上述新回归）。两处 warp-echo 测试共享进程级 `PENDING_WARP_*` 静态态在并行测试下偶发互偷 echo 槽，已加测试期互斥 gate。**未跑全量 workspace 测试**（部分 node/phase/cursor-shield 测试触发真实平台效果，未经隔离）；Windows hook 线程回中与 SetCursorPos echo 顺序需 Windows 原生 CI 与真机验收。已登记未修项：auto-reverse `last_tick` 不推进、双 crossing 路径未统一、滚轮 120x 单位、拖动事件类型、多显示器能力缺口（注入仅寻址主屏）等，见 `.codex-memory/todo.md`。
