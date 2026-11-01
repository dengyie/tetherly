# ADR-131 屏幕串流选择性 NACK 重传的「陈旧分片复活已交付帧」缺陷修复与参考画面单调性加固（Screen NACK Straggler Resurrection & Reference-Picture Monotonicity）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-11-02，v2.45 / v0.1.67）**——根因修复而非放宽断言：为 `ScreenAssembler` 补上「已交付帧水位线」以丢弃重传掉队分片，在 node 接收侧对**所有解码器之前**加装帧序单调门以保护参考画面，并修正 7 个 tile 测试「未等宿主进入 streaming 就断言下一次发送」与 36 处低于 30 FPS 节拍间隔的休眠。 |
| 日期 | 2026-11-02 |
| 关联 | ADR-093（选择性 NACK / HARQ）、ADR-102..126（屏幕瓦片串流流水线）、ADR-121（双缓冲呈现）、ADR-130（上一轮） |

---

## 动机

`ci` 工作流在 `121c704`（v0.1.66）上 `check (ubuntu-latest)` 的 `test` 步失败，`phase5` 恰好 4 例：

```
m5_40_screen_spatial_tile_grid_decomposition_and_streaming_pipeline ... FAILED
m5_44_screen_tile_predictive_compression_pipeline ... FAILED
m5_45_screen_tile_presentation_transaction_pipeline ... FAILED
m5_46_screen_tile_nack_recovery_pipeline ... FAILED
```

三个疑点必须解释清楚，否则任何修改都只是碰运气：

1. **只有 Linux 失败**，macOS / Windows 全绿——若为逻辑错误，三平台应同错；
2. **只有 4 例失败**，其余 33 例 tile/串流测试（含同样走瓦片网格的 `m5_41`..`m5_50`）全绿；
3. **重跑不一定复现**——同一提交在不同运行里表现不同。

前两点指向「依赖时序的竞态 + 统计上的小概率」；第三点要求**先把复现做成确定性的**，否则无法归因。

## 复现

在 Linux 沙盒 `cue-1`（8 核）上镜像源码并构建 `phase5` 测试二进制，用 `taskset -c 0` 钉死单核 + `--test-threads=1` 制造调度饥饿（CI 的 2~4 核 runner 在同一二进制内并行跑 55 个测试，等价于对每个测试施加持续的压力）：

```
taskset -c 0 ./target/debug/deps/phase5-<hash> m5_40 --nocapture --test-threads=1
```

在 6 次运行中第 3 次稳定复现 `m5_40` 失败——**从「偶发」变成「可复现」**，这是后续一切归因的前提。

## 根因

用两层环境变量门控的 `tidiag!` 探针（帧入站 / 各解码分支 / 画布序号 / 出站帧 / NACK 收发 / 重传）抓取一次失败用例的完整线上轨迹，因果链闭环如下：

### 根因一（产品缺陷）：掉队分片复活**已交付**帧，触发 NACK 风暴

`crates/tetherly-core/src/screen/sink.rs` 的 `ScreenAssembler` 负责把 `TMF1` 分片重组成完整帧。它此前**没有**「已经交付到哪一帧」的记忆，于是：

1. 基线关键帧被切成两个分片（65507 + 108 B）。分片 0 先行；分片 1 因故晚到，接收端发出 `NACK frame_seq=1 missing=[1]`；
2. **原始**分片 1 先补齐了重组，帧 1 完成并交付，`ScreenAssembler` 内部状态被 `reset()`；
3. 随后 NACK 触发的**重传**副本（`RETX ty=1281 len=65507`）到达。由于水位线不存在，这份分片 1 **重新开启了一次对「早已交付的帧 1」的重组**；
4. 这次重组缺的是兄弟分片 0，于是**又发一个 NACK** `NACK send frame_seq=1 missing=[0]`——于是对端把**整个 65 KB 陈旧帧**再回灌一遍。

一次丢包被放大成整帧回灌；只要链路抖动，这个循环就会重复，把**旧像素**持续推向解码器。

### 根因二（产品缺陷）：陈旧关键帧回退参考画面，其后全部增量帧永久错解

探针显示，那份回灌的陈旧帧最终以 `KEYFRAME seq=1` 抵达 node，而接收端此时已经交付并呈现过 `[1, 2, 3]`：

```
NACK send frame_seq=1 missing=[0]
RETX ty=1281 len=65507        ← 对端把整帧回灌
incoming seq=1 ... KEYFRAME seq=1
TILE_GRID seq=3 canvas_seq=Some(1)   ← 画布被回退到 1
```

`crates/tetherly-node/src/runtime/screen_stream.rs` 中，关键帧分支会执行 `*screen_receiver_canvas.write() = Some(present_frame.clone())`，而**屏幕瓦片/增量帧是以上一次解码结果为参考画面的**（tile/delta 只传脏块或残差）。此处顺序问题在于：**比对与丢弃发生在 `receiver.apply()` 内部，但画布写入发生在其之前**。于是：

- `receiver.apply(&stale_keyframe)` 确实把 seq=1 判为重复、记入 `dropped++`、**不呈现**（`presented_tail=[1,2,3]` 里没有多余项）——统计上是"正确丢弃"；
- **但画布早已被那帧覆盖**，回退到了 seq=1 的像素；
- 下一个 `TILE_GRID seq=3` 把它的脏块贴到这个**错误基座**上，产生 32px×RGB = 96 字节/处的错位。

失败现场证据（测试侧逐 32px 瓦片像素差分）：

```
diff_bytes=96  tiles(idx,px)=[(0, 48), (15, 48)]   ← 恰为两处 32px×3B = 96B
recv = 原始 set_pattern() 值；want = 第 4 步预期色
```

即：`receiver` 的**统计**说"丢弃了重复帧"，但**画布**已经被那条本该被丢弃的路径污染。二者不一致，是缺陷的本质。

### 根因三（测试缺陷）：断言先于状态就绪，且休眠短于节拍间隔

`tests/phase5.rs` 的 `m5_44`..`m5_50` 在 `POST /api/screen/start` 之后**立刻**断言下一次 `screen_send_frame()` 返回 `true`。但：

- `POST /api/screen/start` 在访客侧只是**把控制消息入队**即返回 200；宿主进入 `Streaming` 要等它的屏幕通道任务读到该消息、且本地同意闸门已打开；
- 在宿主尚未 `Streaming` 时，`screen_send_frame()` 按**设计**返回 `Ok(false)`（`if !state().is_streaming() { return Ok(false); }`）。

所以这些断言在等一个**尚未发生的状态转换**，只是"碰巧"在机器够慢、状态先就绪时通过。对照组：`m5_40` 等 34 例测试**都**显式等待了 `streaming`，因此它们不因此在这一点上失败。

此外，全文件 36 处 `tokio::time::sleep(Duration::from_millis(25))` 用于「等过一个帧间隔」，但测试配置 `screen_fps = 30` ⇒ 节拍间隔 33.3 ms，`is_pacing_ready()` 会拒绝落在同一间隔内的发送。25 ms **短于**该上界，同样只在"中间代码碰巧够慢"时通过。

## 决定

三层各自根治，互不替代：

### 1. `ScreenAssembler` 增加「已交付帧水位线」，掉队分片在**触碰任何缓冲区之前**丢弃

```rust
/// Highest frame seq already handed back by `push`.
///
/// A fragment of a delivered frame is a straggler: selective-NACK recovery
/// retransmits a shard that the receiver asked for, and the original can
/// arrive first, so the copy lands *after* its frame completed. ...
pub(super) delivered_seq: u64,
```

- 整帧路径（`TMV1`）：交付后 `delivered_seq = max(delivered_seq, decoded.seq)`；
- 分片路径：解析出 `seq` 后**立即**判定

  ```rust
  if self.delivered_seq != 0 && seq <= self.delivered_seq {
      return Ok(None);
  }
  ```

  用 `delivered_seq != 0` 兜住「合法的 seq 从 0 开始」这一边界（与 `receiver.apply` 对 `last_seq` 的语义一致）；
- 流切换分支：`if seq < self.seq { return Ok(None); }`，**比在途流更旧**的分片（对端已放弃的帧）不得重启重组、不得丢弃更新的在途重组；
- 完成时先记水位线**再** `reset()`：
  `self.delivered_seq = self.delivered_seq.max(frame.seq);`
- `reset()` 的语义据此在文档注释里写明：**它清空的在途状态，但水位线属于会话级属性，必须存活**——忘记它正是"掉队分片复活已交付帧"的直接原因；
- 新增 `pub fn delivered_seq(&self) -> u64`，让不变量可被测试直接断言。

### 2. node 接收侧：帧序单调门置于**所有解码器之前**

在 `if let Some(sf) = assembled_opt {` 内、任何解码分支（关键帧写画布、瓦片网格、增量）之前：

```rust
// Ordering gate: a frame no newer than the last accepted one must not
// reach *any* decoder below: the canvas is the reference picture for
// tile and delta frames, so rebuilding it from an older frame rewinds
// it and every following delta then decodes on the wrong base.
if sf.seq <= receiver.last_seq() {
    let _ = receiver.apply(&sf);               // 仍记账 dropped++
    *node.shared.screen_rx_stats.lock()...= receiver.stats();
    continue;                                   // 不解码、不写画布、不推放
}
```

即：**丢弃决策要发生在对参考画面产生任何副作用之前**，而不是"先污染画布、再在统计里记为丢弃"。这是把根因一/二封在结构层面的关键——即便将来 assembler 又漏回一份陈旧帧，解码器也不会再被它回退。

### 3. 测试：把「等一个状态」写成语义等待，把「等一个间隔」写成常量

- 新增 `PACER_SETTLE = 40 ms`（> 30 FPS 的 33.3 ms 间隔），36 处 `sleep(25ms)` 全量替换；
- 新增 `wait_host_streaming(host_ui)`：轮询 `GET /api/screen` 直到 `state == "streaming"`（5 s 上限），并插入 `m5_44`..`m5_50` 共 7 例在 `start` 之后的正确位置——**断言它要断言的那个状态本身**，而不是睡一个猜出来的间隔；
- 顺带加固 `m5_10` 的步骤 6：`health` 取的是**发送侧**视角（`in_flight > 0` 或 RTT 偏慢即为 `"warning"`），而循环只等**接收侧**呈现；呈现不等于末帧的 ack 已经回来，单次采样 `health == "healthy"` 是在断言一个**合理地滞后一个 RTT** 的值，在负载高的 runner 上会输掉这场竞态。改为在 3 s 预算内轮询到 `"healthy"`（协调器的契约本就是"突发被确认后健康度回到 healthy"）。

## 结果

| 指标 | 前 | 后 |
|---|---|---|
| 掉队分片能否复活已交付帧 | **能**（每处 NACK 放大为整帧回灌） | **不能**（水位线 + 更旧分片拒绝） |
| 陈旧帧能否回退参考画面 | **能**（画布写先于丢弃判定） | **不能**（帧序单调门在所有解码器之前） |
| `phase5` 在单核饥饿下的结果 | `m5_40` 第 3/6 次失败 | 目标 8 例 **11 轮全绿** |
| `m5_44`..`m5_50` 断言前提 | 猜 25 ms 休眠 / 不等 `streaming` | `PACER_SETTLE`(40 ms) + `wait_host_streaming` |
| `m5_10` health 断言 | 单次采样（滞后一 RTT 的竞态） | 3 s 内轮询到 `healthy` |

验收：

- `cargo fmt --all -- --check` —— 通过；
- `cargo clippy --workspace --all-targets -- -D warnings` —— 通过；
- `cargo test --workspace --locked` —— 全部通过（`tetherly-core` 单元 271 + `state_machines` 20 + `decoders` 28 + `fuzz_decoders` 2 + `limits` 2 + `backpressure` 2 + `rekey` 3 + `screen_wire` 1 等，`tetherly-node` 单元 138，`tetherly-loopback` 7，`tetherly-cli` 3，`phase1` 64 / `phase2` 8 / `phase3` 4 / `phase4` 14 / `phase5` 55 / `loopback` 11）；
- 新增回归测试（`crates/tetherly-core/tests/state_machines.rs`）：`screen_assembler_ignores_stragglers_of_a_delivered_frame`（交付后回灌其分片 → `Ok(None)`、`!is_incomplete()`、`generate_nack().is_none()`、`delivered_seq()==9`）与 `screen_assembler_does_not_restart_a_newer_stream_with_an_older_fragment`（新流在途时来更旧分片 → 流序不回退、新帧仍能组装）；
- 沙盒压力复验（`cue-1`，`taskset -c 0`）：目标 8 例 **11 轮**全绿；CI 并行度下全量 55 例 **4 轮**全绿；
- **归因对照**：在同一台被 CPU 挖矿占满（743% CPU、load ≈ 10.6）的沙盒上，用 **未修改的 v0.1.66 基线**跑 `m5_10`/`m5_24`，同样复现失败（2/6 轮）——证明这两例的抖动是**环境导致**，与本轮修复无关；在干净硬件上基线与修复各自 8/8、12/12 全绿。
- Workspace 版本发布为 `0.1.67`。

## 被否决的替代方案

1. **放宽 `m5_40` 的像素断言 / 跳过测试** —— 掩盖真实缺陷（陈旧帧污染画布），且缺陷会带到生产；
2. **只在 node 侧加帧序门、不动 assembler** —— 能挡住画布回退，但 NACK 风暴（一次丢包放大为整帧回灌）仍在，浪费带宽并放大抖动；两处都要修；
3. **只在 assembler 加水位线、不在 node 加门** —— 依赖单一防线；任何未来的重组路径（如 FEC 重建）漏回陈旧帧，画布就会被静默回退。参考画面的单调性是**解码器的**不变量，应在其入口强制；
4. **在 `receiver.apply` 内部把丢弃提前（把画布写入挪进 apply 之后）** —— 需要把画布所有权穿过端口层，破坏 `tetherly-core` 无 I/O、无共享状态的边界；在 node 循环入口早退只需 6 行且零所有权改动；
5. **把 `is_congested()` 的阈值调大以让 `m5_24` 的 `assert!(sent)` 通过** —— 那是为迁就一台被挖矿占满的沙盒而放宽生产背压策略，方向相反；该抖动已用基线对照证明为环境因素；
6. **用 `#[ignore]` 标记 tile 测试** —— 把 CI 红变成 CI 静默，等价于删除覆盖；本轮之前它们**从未**在 macOS/Windows 失败，说明不是平台特异的假阳性，而是被饥饿放大的真实竞态。
