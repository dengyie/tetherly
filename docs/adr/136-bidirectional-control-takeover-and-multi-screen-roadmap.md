# ADR-136: 双向控制接管治理与屏幕延展/多屏控制路线图

状态: 已实施（双向接管部分） / 规划（延展与多屏部分）
版本: v0.1.71 / v2.52
日期: 2026-11-04

## 1. 背景

用户生产场景：Mac（主控）↔ Windows 工作站，双向键鼠漫游 + 屏幕延展 + 多屏控制。
v0.1.70 引入 Target 越境守卫后暴露了方向治理缺陷：Windows 无法反向控制 Mac。

## 2. 已实施：方向治理（v0.1.71）

### 2.1 不变式
- **全局至多一条出站输入通道**：任意时刻两台机器之间只有一个 controller。
- **座位切换权唯一**：controller 判定所有越境与返回；Target 引擎不做越境评估。
- **方向翻转只由显式 attach 发起**，被接管方优雅降级（polite takeover）。

### 2.2 机制
1. `receive_input_tunnel_request`：接受入站请求前，若本机身为 Controller 先 `input_stop()` 释放（审计 `input_tunnel_takeover_released`）。
2. `Shared.input_client_task`：新入站 accept 中止旧客户端任务，杜绝堆叠会话竞写注入 sink。
3. `link_watcher` 方向守卫：`active_tunnel_role() == Target` 时跳过自动 attach。
4. 空闲态方向：两端均配置 edge 时由 watcher 竞速决定（稳定后不漂移）；生产推荐仅主控端配置 edge。

### 2.3 用户操作语义
- UI「激活漫游通道」= 本机成为 controller（对方自动降级为 target）。
- UI「释放回本机」= 退出控制；若对端配置了 edge，对端 watcher 秒级自动接管。

## 3. 规划：屏幕延展（Screen Extension）

目标：Mac 将 Windows 当作**扩展显示器**（不是镜像），Windows 屏幕呈现 Mac 的第二桌面。

阶段 A — 虚拟显示器管线（复用既有设施）：
1. Windows 端启用 IddCx 间接显示驱动（或 spashtop 式虚拟帧缓冲），产出额外显示模式；
2. macOS 端 `CGVirtualDisplay`/`CGConfigureDisplayWithDisplayMode` 注册远端屏；
3. 帧通道复用 45720 screen wire（H264/RAW 自适应，ADR-079 编码层已具备）；
4. 光标与输入沿用 ADR-133 四向拓扑（延展屏即拓扑中的一个邻居节点）。

阶段 B — 质量与治理：
- 帧率/码率由 ADR-079 的 QoE 闭环接管；窗口拖拽跨屏由 Tier 网格调度保障。

验收：Mac 桌面拖动窗口到 Windows 屏 < 100ms 延迟、可全屏视频播放（≥30fps @1080p）。

## 4. 规划：多屏控制（3+ 屏拓扑）

1. **拓扑图数据结构**：`topology_layout.neighbors` 已是 per-edge 邻接（top/bottom/left/right），扩展为多节点图（每边可挂不同 peer + offset 沿边滑动分段映射）；
2. **控制端角色不变**：主控机仍是唯一 controller；越境目标屏由图查询决定（当前 peer 表已支持 resolve）；
3. **屏幕延展节点**是"无输入屏"——只接收显示流、不参与 input tunnel；
4. **UI**：拓扑编辑器拖拽摆放（现有 Deskflow 拓扑看板升级为图编辑器）；
5. **测试矩阵**：Mac+2 Windows、Mac+Win+延展屏、三屏环绕（wrap_around 已有字段）。

## 5. 已知限制（后续 ADR 候选）

- watcher 自动 attach 首轮与对端 release 存在时序竞态（失败静默，已加 warn 日志——本轮修复）；
- 被控端本地物理输入未锁定（Deskflow "lock to screen" 语义未实现）；
- 方向翻转需手动发起；自动翻转（对端主动接管信令）留待 ADR-137。

## 6. 增补（v0.1.72）：推边自动接管 —— 双向漫游零点击

用户提问"为什么反向必须点击？默认打开有什么问题？"的落地答案：

**朴素默认双向的问题**：两端 watcher 盲目自动 attach = 双控制器互驱（v0.1.70 前的乱飘根因）；被控端旁闲置的物理鼠标停在共享边时，一碰就抢方向。

**安全默认双向（已实现）**：把切换权交给"受控端对本地物理光标的观测"——受控端物理光标**持续推过本机声明的共享边**（topology delay_ms 驻留，默认 100ms）时，自动发起 polite takeover（复用既有优雅接管路径，全局单出站通道不变式保持）。四重防误触闸：
1. 本机策略必须声明 edge（方向先声明才可触发）；
2. 触发前 2 秒内必须有本地物理输入（闲置鼠标永久停在边上也不可能抢方向）；
3. 推边驻留 ≥ delay_ms（扫过/轻碰不翻转）；
4. 自动翻转 3 秒冷却（防风暴翻转）。

两端都声明 edge（Mac=top、Windows=bottom）即达 Deskflow 语义：**任意方向推边即切、全程零点击**；仅一端声明时另一方向仍走 UI/CLI 显式接管。boot 竞速若落入"双 target"中性态（无人驱动、各控各的），任一端推边即恢复漫游——中性态是特性而非故障。


## 7. 增补（v0.1.73）：翻转通道重建与接替守卫（review 根因修复）

v0.1.72 的生产验证暴露了翻转链路的三个真实缺陷，本节为根因修复记录：

1. **通道僵尸复用**：takeover 后新 controller 复用了 target 时代的旧出站通道；被降级方 `input_stop` 关闭 socket → 旧通道死亡且无人重建。修复：`input_tunnel_attach` 无条件弃用旧出站通道并重新拨号（attach = 会话重置语义）；`open_input` 增加幂等（通道健康即跳过）+ `input_dialing` 单拨号闸门（并发 caller 不再产生僵尸连接）。
2. **接替清理竞态**：被取代的旧会话任务退出时无条件执行 `seat=Local + on_peer_gone`，把新会话刚建立的漫游态打回。修复：出站/入站任务退出清理加"会话仍属我"守卫（role+peer 一致性；入站任务另加代际标记）。裸 input 通道（无 tunnel 会话，CI/直连路径）保持既有清理行为。
3. **防误触闸门语义修正**：v0.1.72 的"2 秒内物理输入"闸门是恒真死代码（触发事件本身即新鲜输入）；"首次触边起算 elapsed"驻留会被"两次间隔 500ms 的孤立误碰"击穿。修复：**间隔连续性驻留**——触边事件必须持续到达（间隔 ≤150ms，等价于鼠标报点停顿 10Hz 断链线）累计满 `delay_ms` 才翻转；`TOUCH_GAP` 语义与测试（m3_11：单触不翻、双误碰 300ms 不翻、连续推翻）固化。

同批删除：`InputTunnelSuspendFrame`/`TYPE_INPUT_TUNNEL_SUSPEND`/`is_resume`（v0.1.68 引入、从未兑现的协议骨架）与 `InputServer::local_move`（生产零调用的双轨语义）。

## 8. 增补（v0.1.74）：距离武装 —— 新插入鼠标的空间维度闸门

用户场景：Windows 上插着 Mac 的鼠标（物理插拔共用一只鼠标）。插入瞬间的抖动、桌面震动、原地微碰都可能在共享边上产生触边事件——时间维度闸门（间隔连续性驻留）挡不住"恰好在边上的一次抖动"。

**空间维度武装（第四闸门）**：
- 自最近一次静止（>2s 无物理移动）以来，物理光标累计移动 **≥300px**（坐标差曼哈顿和，Windows hook 无逐事件 delta）方视为"真实使用"，才允许进入接管驻留；
- 静止重置同时**吸收插拔跳变**：重置后的首个事件不计入距离（插上后首报坐标与历史位置无关）；
- 未武装时任何推边都会清除驻留状态——距离与驻留不可互相"攒"；
- 真实使用从屏幕任意位置移动到底边必然越过 300px（约 1080p 高度 1/4），主流程无感。

至此 auto-reverse 共四道闸：**方向声明 → 距离武装（新）→ 间隔连续性驻留 → 3s 冷却**。

## 9. 增补（v0.1.75）：单调时钟域统一与累加状态生命周期重置

v0.1.74 引入距离武装后的生产代码审查暴露了时钟域与状态生命周期的边界问题：

1. **单调时钟域统一（Monotonic Clock Unification）**：
   - 原 `last_auto_reverse_ms` 与 `reverse_last_move_ms` 使用墙上时钟 `unix_ms()`，在系统时间同步（NTP 跃变/夏令时）期间存在时差负溢出或超期判断失真的风险；
   - 全面迁移为单调时钟 `std::time::Instant`（`Mutex<Option<Instant>>`），确保驻留与静止判定严格基于物理单调时间推进。
2. **累加状态生命周期重置（Lifecycle Reset Guard）**：
   - 提取 `reset_reverse_arming_state()`，在入站隧道建立（`receive_input_tunnel_req`）、隧道主动注销（`input_tunnel_release`）与入站通道退出清理（`attach_input_client`）时显式清零距离累加与驻留起点，杜绝前序会话未过 2s 窗口留存的累加距离击穿新会话防御。
3. **物理活跃心跳扩展（Physical Activity Heartbeat）**：
   - 在 `drive_input_button` 与 `drive_input_wheel` 中植入 `note_reverse_physical_activity()`，在原地点击或滚动时刷新最近物理活跃时间，防止误判为静止超时。
