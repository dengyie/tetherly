# ADR-090 屏幕流前向纠错（FEC）与自适应冗余保护流水线（Screen Forward Error Correction & Adaptive Redundancy Protection Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v2.04）**——实现基于伽罗瓦域 $GF(2^8)$ 柯西生成矩阵（Cauchy Generator Matrix）的线速块级前向纠错引擎（`ScreenFecBlock`、`ScreenFecEncoder`、`ScreenFecDecoder`）与自适应冗余保护控制器（`ScreenFecController`）；定义 24 字节紧凑型线缆分片头（`TYPE_SCREEN_FEC = 0x0504`，魔数 `TFEC`）；支持 Auto（自适应丢包阶梯 0% ~ 30%）、Conservative（固定 10% 冗余）、Aggressive（固定 25% 冗余）和 Disabled（零开销透传）四种运行模式；与 ADR-086 4 维自主自适应矩阵协同器（`network_health()`）深度耦合；在接收端实现零往返（0-RTT）乱序抗丢重组恢复器（`ScreenFecAssembler`）；完整集成至 `ScreenSender`、`ScreenReceiver`、`tetherly-node` 守护进程运行时与回环 REST API（`GET/POST /api/screen/fec`）；实现 100% 全同构双端 CLI（`tetherly screen fec` 与 `tetherly-cli screen fec`）；新增端到端自动化测试 `m5_14_screen_adaptive_fec_protection` |
| 日期 | 2026-10-25 |
| 关联 | ADR-080「双向帧确认与自适应拥塞背压」、ADR-083「屏幕流动态帧率与自适应定步步频控制器」、ADR-084「屏幕流动态分辨率缩放与自适应下采样控制器」、ADR-086「屏幕流 4 维自主自适应协同调度器」、ADR-087「屏幕流接收端播放抖动缓冲区与自适应平滑控制器」、ADR-088「屏幕流硬件光标解耦与高频指针同步协议」、ADR-089「屏幕流脏区差分检测与静止画面自适应静噪控制器」 |

## 动机

1. **解决弱网与突发丢包下的重传风暴与交互卡顿（Stop-and-Wait ARQ Stall vs Zero-RTT Recovery）**：
   - 在远程桌面实时串流中，视频流对时延极度敏感。传统仅依靠底层传输层（如 QUIC/TCP）丢失重传（ARQ）或发送端强制 IDR 重刷的机制，受制于端到端往返时延（1 RTT ~ 数百毫秒）；
   - 在跨国公网、高丢包 Wi-Fi、蜂窝移动网络中，单包丢失会导致整个视频帧甚至后续 P 帧解码流水线阻塞冻结，造成画面严重卡顿、撕裂与橡皮筋延迟；
   - 引入前向纠错（FEC），在发端按一定比例构造奇偶校验分片（Parity Shards），接收端无需等待任何逆向 ACK/NACK 交互，即可在收到任意 $K$ 个分片时瞬时数学重构全部原始数据帧，达成 0-RTT 自愈。

2. **传统 Reed-Solomon 范德蒙矩阵（Vandermonde）计算瓶颈与奇异矩阵隐患**：
   - 经典 Vandermonde 矩阵在有限域求逆时，随着块数增加容易出现子矩阵行列式为 0（退化为非 MDS 矩阵）导致解码失败；
   - 柯西生成矩阵（Cauchy Generator Matrix）$C_{r, c} = \frac{1}{X_c \oplus Y_r}$ 天然保证任意选取的方阵其行列式均不为零，具备数学严格的「最大距离可分（Maximum Distance Separable, MDS）」特性：任意 $K$ 个到达的分片均百分之百可逆解出原始数据。

3. **动态网络质量下的按需冗余自适应（Dynamic Redundancy Scaling）**：
   - 固定 FEC 冗余率具有明显的两面性：在优质局域网（丢包 $<0.1\%$）下持续发送 20% 冗余校验包是对带宽与 CPU 的纯粹浪费；在恶劣弱网（丢包 $>10\%$）下，固定的轻量冗余又不足以覆盖连续丢包（Burst Loss）；
   - 必须将 FEC 控制器与系统的 4 维网络感知矩阵（RTT、吞吐、丢包率、网络健康度）无缝联动，实现无丢包时 0% 零开销透传，突发丢包时毫秒级阶梯爬升（10%、20%、30%）的弹性防护。

4. **双端命令行与可观测性 100% 同构要求**：
   - Tetherly 秉持「双端 CLI 全同构」架构准则，运维人员与自动化脚本无论在全功能守护节点（`tetherly`）还是轻量运维客户端（`tetherly-cli`）上，均需拥有一致的子命令、参数、JSON 输出与遥测字段。

---

## 决定

1. **有限域 $GF(2^8)$ 算术与柯西生成矩阵（Cauchy Reed-Solomon）**：
   - **有限域多项式**：采用 AES 标准不可约多项式 $P(x) = x^8 + x^4 + x^3 + x^2 + 1$（十六进制 `0x11D`）；
   - **快速乘法与求逆**：
     - 乘法基于移位异或与条件模约简（Bitwise Shift & XOR Reduction）；
     - 求逆基于费马小定理（Fermat's Little Theorem）：在 $GF(2^8)$ 中，$a^{255} = 1$，故 $a^{-1} = a^{254}$；
     - 除法运算直接实现为 $a / b = a \cdot b^{-1}$；
   - **柯西矩阵构造**：
     - 定义数据索引列 $X_c = c$（$0 \le c < K$），校验索引行 $Y_r = K + r$（$0 \le r < M$）；
     - 柯西矩阵元素 $C_{r, c} = \text{gf\_inv}(c \oplus (K + r))$；
     - 严格保证任意 $K \times K$ 子矩阵均可逆，任何接收到的 $K$ 个有效分片均可通过高斯消元法（Gaussian Elimination over $GF(2^8)$）完全重构丢失的数据分片。

2. **紧凑型前向纠错线缆分片协议（`TYPE_SCREEN_FEC = 0x0504`）**：
   - 魔数标识：`TFEC`（`0x54, 0x46, 0x45, 0x43`）；协议版本 `proto = 1`；
   - 24 字节定长二进制头结构：
     ```text
     +-------------------+-------------------+-------------------+-------------------+
     |    magic: "TFEC"  | proto(1) |flags(1)|           reserved(2)             |
     +-------------------+-------------------+-------------------+-------------------+
     |                            frame_seq (8 字节)                             |
     +-------------------+-------------------+-------------------+-------------------+
     |    block_id (2)   |    k (1) |  m (1) | shard_idx(1)|is_parity| shard_len (2) |
     +-------------------+-------------------+-------------------+-------------------+
     |                               Shard Payload ...                               |
     +-------------------+-------------------+-------------------+-------------------+
     ```
   - 关键字段设计：
     - `frame_seq`: 对应视频大帧序号；
     - `block_id`: 帧内 FEC 编码块索引（防止超大 4K 帧分片数超过 $K=255$ 上限）；
     - `k`: 该块包含的原始数据分片数（$1 \le K \le 128$）；
     - `m`: 该块生成的冗余校验分片数（$0 \le M \le 64$）；
     - `shard_index`: 该分片在块内的逻辑索引号（$0 \le \text{idx} < K + M$）；
     - `is_parity`: 0 表示原始数据分片，1 表示柯西生成的冗余校验分片；
     - `shard_len`: 分片有效载荷长度（字节）。

3. **自适应冗余控制器（`ScreenFecController`）**：
   - 提供 4 种运行模式：
     - **`Auto (0)`（默认）**：
       - 根据网络丢包率动态分阶：
         - 丢包率 $< 0.5\%$：$M = 0$（0% 冗余，零开销直通）；
         - 丢包率 $0.5\% \sim 3.0\%$：10% 冗余（应对偶发抖动）；
         - 丢包率 $3.0\% \sim 8.0\%$：20% 冗余（应对中度拥塞）；
         - 丢包率 $\ge 8.0\%$：30% 冗余（对抗恶劣突发丢包）；
       - 深度绑定 ADR-086 4 维自主感知矩阵网络健康度评估（`Healthy` $\rightarrow 0\%$、`Warning` $\rightarrow 10\%$、`Congested` $\rightarrow 20\%$、`Critical` $\rightarrow 30\%$）；
     - **`Conservative (1)`**：固定 10% 冗余，保障常规办公；
     - **`Aggressive (2)`**：固定 25% 冗余，保障跨洋链路与高抗丢场景；
     - **`Disabled (3)`**：彻底禁用 FEC，分片直接以 `TYPE_SCREEN_FRAME` 原生发送，零计算、零额外包头开销。

4. **接收端 0-RTT 抗丢恢复器（`ScreenFecAssembler`）**：
   - 维护按 `(frame_seq, block_id)` 索引的滑动重构槽位；
   - 收到数据分片：直接暂存；若集齐全部 $K$ 个原始数据分片，无需矩阵计算，直接拼接还原整帧，零 CPU 解码耗时；
   - 收到校验分片：若发生数据分片丢失，但在接收到的数据与校验分片总数达到 $K$ 时，触发高斯消元求逆矩阵解算，重构缺失的数据分片；
   - 还原成功后立刻交付上层视频解码管线，并递增 `fec_recovered_shards` 指标；
   - 若分片接收不足且收到更高序列号帧，超时释放损坏块并递增 `fec_unrecoverable_blocks` 统计。

5. **回环 REST API 与 Dual-CLI 100% 同构**：
   - **HTTP REST 接口**：
     - `GET /api/screen/fec`：获取当前 FEC 模式、冗余度百分比、已编码块数、发出数据/校验分片数、已恢复分片数与未决破坏块数；
     - `POST /api/screen/fec`：动态修改 FEC 工作模式（支持 `mode` 或 `fec_mode` 参数）；
   - **统一 CLI 指令**：
     ```bash
     tetherly screen fec [mode] [--json] [--ui-port <port>]
     tetherly-cli screen fec [mode] [--json] [--ui-port <port>]
     ```
   - **守护进程启动标志**：
     ```bash
     tetherly run --screen-fec <auto|conservative|aggressive|disabled>
     ```
   - **状态监控扩展**：
     - `tetherly screen status` 扩展显示：`FEC Mode: auto (redundancy: 0%, recovered: 0, unrecoverable: 0)`。

---

## 验证结果

1. **有限域算术与柯西矩阵高斯消元单测**：
   - 验证 $GF(2^8)$ 乘法可逆性、非零元素求逆全覆盖、柯西 MDS 方阵求逆可解性；
   - 模拟 $K=4, M=2$ 任意丢弃 1~2 个数据分片，解码器 $100\%$ 完美恢复原始报文。
2. **端到端集成测试（`tests/phase5.rs`）**：
   - 新增 `m5_14_screen_adaptive_fec_protection` 测试用例；
   - 覆盖默认 Auto 模式、Conservative 动态切档、Aggressive 冗余计算、Disabled 旁路直通；
   - 在真实 Noise 45720 传输链路上注入大帧，验证数据与柯西校验分片的打包拆包、`ScreenFecAssembler` 重构与 `/api/screen/fec` 遥测更新。
3. **工作区测试全量通过**：
   - 执行 `cargo test --workspace`，31 套件全数通过，0 warning，0 error。
