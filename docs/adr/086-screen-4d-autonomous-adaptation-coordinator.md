# ADR-086 屏幕流 4 维自主自适应协同调度器（Screen Streaming 4D Autonomous Adaptation Coordinator）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v2.00）**——统一带宽码率（ADR-082）、时间帧率（ADR-083）、空间分辨率（ADR-084）与语义画质/QP（ADR-085）4 维流控矩阵，实现闭环自主协同调度器 `ScreenMatrixCoordinator`、四级网络健康评估模型 `NetworkHealth`（Healthy / Warning / Congested / Critical）、跨层降阶梯队与迟滞恢复升阶模型、三种调度模式（Auto / Conservative / Manual）、回环 REST API 与 CLI 动态控制 |
| 日期 | 2026-10-25 |
| 关联 | ADR-080「双向帧确认与自适应拥塞背压」、ADR-081「按需关键帧请求与序列断洞自愈恢复」、ADR-082「屏幕流动态速率自适应与 AIMD 码率控制器」、ADR-083「屏幕流动态帧率与自适应定步步频控制器」、ADR-084「屏幕流动态分辨率自适应降采样控制器」、ADR-085「屏幕流自适应内容模式与量化参数 QP 质量控制器」 |

## 动机

1. **孤立多维流控维度的失步与局部最优陷阱**：
   - 在先前的里程碑中，Tetherly 陆续构建了四大维度的独立控制器：
     - **带宽/码率（ADR-082）**：`ScreenRateController`（AIMD 200 kbps .. 20 Mbps 动态控码）；
     - **时间/帧率（ADR-083）**：`ScreenFpsPacer`（5 .. 120 FPS 微秒定步阻断）；
     - **空间/分辨率（ADR-084）**：`ScreenScaleController`（25% .. 100% 整数双线性下采样）；
     - **语义/画质（ADR-085）**：`ScreenQualityController`（Text / Motion 模式联动与 0..=51 QP 边界钳制）。
   - 如果各个控制器各自行事：
     - 当网络突发严重拥塞时，若仅依靠码率乘法递减（MD），由于套接字发送缓冲区依然以高帧率（如 60 FPS）持续推包，小包报头开销与拥塞丢包将导致在途队列持续恶化；
     - 若同时盲目骤降分辨率并触发编码器重构，可能在偶发单帧抖动时产生不必要的画面模糊和 IDR 瞬时爆码；
     - 在网络恢复阶段，若并发提升分辨率和帧率，容易引起二次网络拥塞震荡。
2. **需要全局跨层闭环调度核心（Matrix Coordinator）**：
   - 必须在传输层（Ack、RTT、丢包、在途深度）与多媒体层（Bitrate、FPS、Scale、Profile）之间建立一个中心仲裁协调器：
     $$\text{Coordination}(t) = \mathcal{M}(\text{RTT}(t), \text{InFlight}(t), \text{LossRate}(t), \text{PacingState}(t))$$
   - 依据清晰的四级网络健康状态评估，有序执行非对称的「快速分阶降级」与「保守迟滞升阶」梯队。
3. **确定性工作模式满足不同用户偏好**：
   - 提供 `Auto`（全自动 4D 闭环自适应）、`Conservative`（保守模式：锁定 100% 分辨率，仅动态调优码率和 FPS，保障桌面文字绝对不失真）、`Manual`（手动模式：完全由操作者通过 API/CLI 指定各维度参数）。

---

## 决定

1. **网络健康四级分级评估（`NetworkHealth`）**：
   - 基于端到端实时反馈的 RTT、在途未确认帧数（`in_flight`）与发送端主动丢包（`consecutive_drops`）综合研判：
     - **Healthy (0)**：$\text{RTT} < 120\,\text{ms} \land \text{in\_flight} == 0 \land \text{drops} == 0$。网络极度通畅。
     - **Warning (1)**：$120\,\text{ms} \le \text{RTT} < 250\,\text{ms} \lor 1 \le \text{in\_flight} \le 3$。排队延迟开始显现。
     - **Congested (2)**：$250\,\text{ms} \le \text{RTT} < 500\,\text{ms} \lor 4 \le \text{in\_flight} < 8 \lor \text{drops} == 1$。信道明显拥堵，出现缓冲区积压或初次丢包。
     - **Critical (3)**：$\text{RTT} \ge 500\,\text{ms} \lor \text{in\_flight} \ge 8 \lor \text{drops} \ge 2$。严重弱网或链路断崖，需采取激进降载措施。

2. **跨层非对称阶梯调度算法（Ladder Demotion & Promotion）**：
   - **降阶流程（Demotion on Drops / Congestion）**：
     - **Level 1（码率收缩）**：触发 AIMD 乘法递减（MD），码率下降 20%，立即缩减后续帧编码字节预算。
     - **Level 2（时间降频）**：将目标 FPS 按梯度调降：$60 \to 30 \to 24 \to 15\,\text{FPS}$。在不改变帧分辨率与无重新分配帧内存的前提下，瞬间将套接字推流频次减半，立竿见影缓解网络套接字排队压力。
     - **Level 3（空间降采样）**：当网络健康陷入 `Critical` 时，启动下采样比例调节：$100\% \to 75\% \to 50\% \to 25\%$，大幅降低原始像素吞吐量与编码宏块复杂度，并强制触发一次首帧 IDR 恢复帧。
     - **防振荡保护**：设置降阶冷却期 `SCREEN_MATRIX_DEMOTE_COOLDOWN_US = 400_000`（400 ms），防止同一突发事件触发连续多级无序跳水。
   - **升阶流程（Promotion on Sustained Stability）**：
     - **门槛条件**：必须在无拥塞环境下持续累积 `SCREEN_MATRIX_HEALTHY_ACKS_REQUIRED = 15` 个连续健康帧确认，且距上次升阶经历 `SCREEN_MATRIX_PROMOTE_COOLDOWN_US = 3_000_000`（3.0 秒）严格迟滞期。
     - **升阶顺序**：
       1. **优先恢复空间分辨率**：$50\% \to 75\% \to 100\%$，优先保证桌面操作界面的文字锐度；
       2. **恢复时间帧率**：$15 \to 24 \to 30 \to 60\,\text{FPS}$，增强鼠标滑行与动态交互流畅度；
       3. **AIMD 加法探测码率**：以 $+150\,\text{kbps}$ 逐步探测可用物理带宽瓶颈。

3. **三种协同调度模式（`ScreenMatrixMode`）**：
   - `Auto (0)`：默认模式，全自主 4 维闭环协同。
   - `Conservative (1)`：分辨率强锁 100% 原生尺寸（`auto_scale = false`），只在码率和帧率维度自适应，确保代码文字绝对点对点清晰。
   - `Manual (2)`：调度器处于旁路被动观察状态，各维度参数完全由外部明确调用覆盖。

4. **系统架构与接口暴露**：
   - **`ScreenSender`**：内聚 `matrix_coordinator: ScreenMatrixCoordinator`，在接收到帧确认 `on_control_with_time` 或发送端丢帧 `record_drop_with_time` 时即时驱动状态机流转。
   - **运行时同步**：`NodeSession` 在推流循环中定期同步 `fps_target` 与 `scale_pct` 到 `ScreenSource` 与 `ScreenCodec`。
   - **REST API**：
     - `GET /api/screen/matrix`：读取当前模式、健康状态、4 维实时参数与网络度量；
     - `POST /api/screen/matrix`：支持 JSON 请求体 `{"mode": "auto" | "conservative" | "manual"}` 或查询参数 `?mode=...` 动态热切换。
   - **CLI 命令行**：
     - `tetherly screen matrix [mode]`：查询或更新 4D 调度器模式；
     - `tetherly run --screen-matrix <auto|conservative|manual>`：启动配置；
     - `tetherly screen status`：实时状态显示当前 `Matrix Mode` 与 `Net Health`。

---

## 落地验证

1. **核心单元测试**（`crates/tetherly-core/src/screen.rs`）：
   - `screen_matrix_coordinator_health_state_transitions`：验证 RTT、在途计数与连续丢包驱动的四级健康状态迁移；
   - `screen_matrix_coordinator_demotion_ladder_on_drops`：验证连续丢包时从 FPS 到 Scale 的分阶有序降级；
   - `screen_matrix_coordinator_promotion_ladder_on_sustained_healthy_acks`：验证 15 次健康确认与 3s 迟滞期的逐级平滑升阶；
   - `screen_matrix_coordinator_modes_manual_and_conservative`：验证模式切换对自适应行为的锁死与解禁；
   - `screen_sender_4d_autonomous_matrix_closed_loop`：验证 `ScreenSender` 中 4 维流控回路的闭环驱动。
2. **节点运行时测试**（`crates/tetherly-node/src/runtime.rs`）：
   - `test_screen_matrix_mode_and_reporting`：验证 API 报告结构体、模式切换与 `screen_status_report` 状态同步。
3. **Phase 5 端到端集成测试**（`tests/phase5.rs`）：
   - `m5_10_screen_4d_matrix_coordinator`：验证双节点 45720 传输握手、回环 API 热调优、H.264 编码解码、帧投递呈现与遥测联动。
4. **远程 VM 真实环境验证**：
   - 部署至 LeaseWeb 测试 VPS / 沙盒虚拟机（`mesh-nl` / `10.144.144.12`），多维度验证各项接口及推流表现。
