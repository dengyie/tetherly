# ADR-130 回环客户端与格式化助手的共享化（Shared Loopback Client & Text Helpers，含超时策略与 CLI 命令树守护测试）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-11-01，v2.44 / v0.1.66）**——根因修复而非打补丁：把两份拷贝的回环 HTTP/SSE 客户端与三份拷贝的格式化/编码助手各自收敛到单一实现，并为客户端补上此前完全没有的超时；同时删除一个从未被读取的 CLI 参数，收回过度宽松的 lint 抑制，并把「命令树完整性」从人工冒烟固化为测试。 |
| 日期 | 2026-11-01 |
| 关联 | ADR-129（`bins/tetherly-cli` 分解）、ADR-128（`uihttp.rs` / node bin 分解）、ADR-127（`screen.rs` / `runtime.rs` 分解）、ADR-006（许可证与自包含依赖边界） |

---

## 动机

一次面向生产环境的严格评审（9 维：正确性 / 结构 / 架构 / 扩展性 / 可维护性 / 可靠性安全 / 性能 / 测试 / 重复兼容废弃）报出 6 项 P3 级问题与 2 项 Suggestion，全部为**真实缺陷或结构性问题**，而非风格偏好。它们有一个共同点：**同一个事实在多个地方各存一份，且没有一处是权威的**。

1. **回环 HTTP/SSE 客户端存在两份实现，且两份都没有超时**（F5/F6/S-1）
   - `crates/tetherly-node/src/uihttp.rs`（`loopback_get/post/put/delete` + `parse_http_response` + `SseStream`）与 `bins/tetherly-cli/src/util.rs`（同样的 HTTP 四件套 + `parse_http_response`）各有一份手写实现，CLI 的 `SseStream` 则内联在 `src/main.rs`；
   - 两者都用 `TcpStream::connect(("127.0.0.1", port)).await?` + 裸 `read`/`write_all`/`read_line`，**没有任何超时**。触发条件极其常见：端口无人监听时 connect 会阻塞到内核 SYN 超时（本机回环通常快速 RST，但在容器 / 半开 socket / 被防火墙 DROP 的沙箱里可长达数十秒）；更严重的是 **SSE 长连接**——`--watch` 类命令一旦接上就不再返回；若对端进程被 `SIGSTOP`、挂起或被 `kill -STOP` 打断，`read_line` 永不返回，CLI 永久悬挂且无法 Ctrl+C 之外的任何自愈；
   - 两份实现已经发生可见漂移：`SseStream::next_event` 在 node 侧是 `pub` 且带文档注释、在 CLI 侧是私有；`loopback_events_connect` 在 node 侧把失败包装成 `io::Error::other("SSE HTTP handshake failed: ...")`，CLI 侧同样如此但**没有 `next_ui_event`**（因为 `UiEvent` 类型只在 node 侧存在）；HTTP 四件套的差异仅剩格式化与可见性（`pub` vs `pub(crate)`），这正是"逻辑同源却各自演化"的临界状态。
2. **格式化与百分号编码助手存在三份实现**（F7）
   - `format_uptime` / `format_bytes` / `format_speed` / `format_timestamp_ago` / `url_encode` / `urlencode_path` 同时存在于 `bins/tetherly-cli/src/util.rs`、`crates/tetherly-node/src/bin/tetherly/util.rs`，编码逻辑还第三次出现在 `tests/phase1.rs` 的 `tetherly_cli_test_url_encode`；
   - 后果是可验证的漂移：`format_timestamp_ago` 的两份实现都把 `SystemTime::now()` **读在函数体内**，因此无法在测试里固定"现在"，所有相对时间输出都是不可测的；`tests/phase1.rs` 那份编码器虽然当前字符集与生产实现一致，但它是一份**随手的复刻**——生产实现改动时没有任何机制会让它同步，而它却被用来断言生产行为（用一把可能被改过的尺子量生产）。
3. **`audit --interval` 是一个端到端死参数**（F1/P3-1）
   - 参数被 `Cmd::Audit` 声明、被 `dispatch` 一路透传到 `handle_audit_watch(_interval: u64)`，但该形参**从未被读取**——`--watch` 走的是服务端推送的 SSE，不是轮询。用户按帮助文本以为能调刷新频率，实际毫无效果。
4. **lint 抑制过度宽松**（F3/P3-3）
   - node bin 树内有 147 处 `#[allow(clippy::too_many_arguments)]`，其中 **113 处挂在参数不足 8 个的函数上**——lint 根本不会触发。这种「先加再说」的抑制会让真正超限的函数淹没在噪声里，属于典型的抑制债。
5. **`SystemTime::now()` 内联散落**（F8/S-2）
   - 8 处 `cmd_*.rs` 直接调用系统时钟，绕过了项目已有的可注入时钟 `tetherly_core::ports::{Clock, SystemClock, ManualClock}`，导致这些分支无法在测试里固定时间（`cmd_messages`、`cmd_files`、`cmd_audit`、`cmd_notify` 的时限/过期判定全在其中）。
6. **CLI 零测试、命令树无守护**（F2/P3-2）
   - `bins/tetherly-cli` 此前**没有任何测试**。它的 485 条命令路径（457 条规范路径 + 28 个别名，含 `screen` 子子命令）只在发布前靠人工递归跑 `--help` 冒烟，回归只能靠肉眼。
7. **文档行数漂移**（P3-4）
   - ADR-129 与 `DEVELOPMENT.md` 记录的 `dispatch.rs` 418 行、`cmd_screen.rs` 70 行，与其后 `git` 里的实际行数（463 / 341）不符。

## 决定

### 1. 新增 `tetherly-loopback` crate：唯一回环客户端，带强制超时

约束：`tetherly-core` 按设计**不含 tokio**（"no OS APIs"），`tetherly-net` 含 tokio 但语义是 P2P 传输，`tetherly-node` 拉入 arboard/png/zstd/openh264，对 CLI 太重。因此新建只依赖 `tokio` + `serde_json` 的最小 crate：

```toml
# crates/tetherly-loopback/Cargo.toml
[dependencies]
serde_json.workspace = true
tokio.workspace = true
publish = false
```

四个可覆盖常量，构成**回环调用的时间上界策略**：

| 常量 | 值 | 依据 |
|---|---|---|
| `CONNECT_TIMEOUT` | 5 s | 本机回环正常 <1 ms；5 s 已足以跨过沙箱/容器首包抖动，再长就是故障 |
| `REQUEST_TIMEOUT` | 30 s | 服务端对所有响应都带 `Connection: close`，正常应答远小于此；30 s 覆盖屏幕抓帧等最慢端点 |
| `SSE_HANDSHAKE_TIMEOUT` | 10 s | 握手期间服务端会立刻发 `: connected` |
| `SSE_IDLE_TIMEOUT` | 60 s | 服务端每 15 s 发一次 `: ping`，60 s = 连续错过 4 次心跳才判定僵死 |

API 面（`pub`）：`loopback_get/post/put/delete`、`SseStream::next_event`、`loopback_events_connect`、`loopback_events_connect_with_deadlines`（测试与特殊场景用）；`parse_http_response` 为 crate 私有 + `#[cfg(test)]` 可见。

**`SseStream` 的超时是按行计时的，不是整条流的**：`next_event` 对每一次 `read_line` 施加 `idle_timeout`，因此一条长期存活但在心跳的流不会被误杀，而对端僵死则一定在 60 s 内返回 `ErrorKind::TimedOut`。这就是「超时修复」与「长连接」的正确结合点。

### 2. `tetherly-node` 保持公共 API 不变，用 15 行 facade 承接

node crate 内近 **700 处** `loopback_*` 调用与 **40 处** `next_ui_event` 使用，改动它们既不必要也不可能一次性验证。做法：

- `uihttp.rs` 删除本地实现，改为 `pub use tetherly_loopback::{loopback_delete, loopback_get, loopback_post, loopback_put};`（PUT 的唯一消费者是 `tests/phase1.rs:3722`）；
- `SseStream` 保留同名同签名的 newtype 包装，唯一自有逻辑是 `next_ui_event` 把 SSE 行反序列化成 `crate::events::UiEvent`——**这个类型属于 node，所以这段逻辑必须留在 node**：
  ```rust
  pub struct SseStream(tetherly_loopback::SseStream);
  impl SseStream {
      pub async fn next_event(&mut self) -> std::io::Result<Option<String>> { self.0.next_event().await }
      pub async fn next_ui_event(&mut self) -> std::io::Result<Option<crate::events::UiEvent>> {
          while let Some(raw) = self.next_event().await? {
              if let Ok(ev) = serde_json::from_str::<crate::events::UiEvent>(&raw) { return Ok(Some(ev)); }
          }
          Ok(None)
      }
  }
  ```
- 由此全部调用点**零改动**，也不必引入 `async fn in trait`（那会需要 `#[allow(async_fn_in_trait)]`，与本轮「收敛抑制」的方向相反）。

### 3. `tetherly-core::text`：格式化与编码的唯一实现

纯函数，无时钟无 I/O，因此归 core。关键设计：**`format_timestamp_ago(ts_secs, now_secs)` 把"现在"做成参数**，而不是在函数里读系统时钟——这既消除了 F8 那一类内联 `SystemTime::now()`，也让函数本身可测。`encode_query_value` 转义 `/`，`encode_path` 保留 `/`，两者差异显式化。7 个单元测试覆盖：单位切换边界（1023/1024）、零值语义、`ts == 0`、负数方向（未来时间）、多字节 UTF-8 逐字节转义、路径分隔符保留。

配套在 `tetherly_core::ports` 增加 `now_ms()` / `now_secs()`（基于既有 `SystemClock`），让「读墙上时钟」也只有一处。

两个 bin 的 `util.rs` 退化为**纯 re-export**（CLI 19 行、node 13 行），保留 `super::util::{...}` 的既有导入形态，28 + 29 个模块无需改动。`tests/phase1.rs` 里的第三份编码器删除，改用 `tetherly_core::text::encode_query_value`——测试从此量的是生产实现。

### 4. 删除 `audit --interval`，并用测试锁死边界

从两棵 clap 树同时删除 `Cmd::Audit::interval`，`dispatch` 与 `cmd_audit` 的形参一并删除（`handle_audit_watch` 由 6 参降为 5 参）。`status` / `sys` 的 `--interval` **保留**——它们在 `cmd_core.rs:275` / `:367` 真的被用于轮询。

新增的测试把这条边界写成断言，而不是靠注释：

```rust
fn watch_interval_only_exists_where_it_is_honoured() {
    // status / sys 轮询 → 必须存在
    // audit --watch 推送 → 必须不存在，且 --interval 必须被 clap 拒绝
}
```

### 5. 抑制债按「实测需要」重收

147 处 `#[allow(clippy::too_many_arguments)]` 全部删除后重跑 clippy，按诊断逐处回填 —— node bin 树 **34** 处真的需要，CLI 树 **33** 处。判定基于 clippy 的 primary span，而非行数估计。

### 6. CLI 命令树从人工冒烟升级为测试

`bins/tetherly-cli/src/cli.rs` 新增 `#[cfg(test)] mod tests`，三个不变量：

1. `command_tree_satisfies_clap_invariants` —— `Cli::command().debug_assert()`（clap 官方建议在测试里永久保留的自检，检查参数冲突、缺失 about、非法 arg 名等）；
2. `every_command_path_resolves_help` —— 递归收集全部 485 条路径（457 条规范路径 + 28 个别名），逐条 `try_get_matches_from(argv + --help)` 并断言 `ErrorKind::DisplayHelp`。**这取代了发布前那一次人工 `--help` 冒烟**，回归从"发布前跑一次"变成"每次 `cargo test` 都跑"；同时用 `paths.len() >= 470` 兜住"整棵子树被误删"这种静默退化；
3. `watch_interval_only_exists_where_it_is_honoured` —— 见上。

**实现细节（踩坑记录）**：这三条测试在默认 2 MiB 的 libtest 线程栈上会 `stack overflow, aborting (SIGABRT)`——`Cli::command()` 在 debug 构建里构造并 `debug_assert` 一棵 457 路径的树，栈帧总量远超默认值（`./target/debug/tetherly-cli --help` 正常退出，说明不是命令树本身的问题）。修复**不能**依赖 `RUST_MIN_STACK`（CI 不会设它），而是让测试自己在指定栈大小的线程里跑：

```rust
fn with_deep_stack<F: FnOnce() + Send + 'static>(f: F) {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024)
        .spawn(f).expect("spawn clap-tree walker").join().expect("clap-tree walker panicked");
}
```

## 结果

| 指标 | 前 | 后 |
|---|---|---|
| 回环 HTTP/SSE 客户端实现份数 | 2 | **1**（`tetherly-loopback`） |
| 回环调用超时上界 | **无**（可无限悬挂） | connect 5 s / request 30 s / SSE 握手 10 s / SSE 空闲 60 s |
| 格式化 + 编码助手实现份数 | 3 | **1**（`tetherly_core::text`） |
| `SystemTime::now()`（非测试代码） | 8 | **0**（统一 `now_ms()`） |
| 死参数 | 1（`audit --interval`） | **0** |
| `#[allow(clippy::too_many_arguments)]`（node bin / CLI） | 147 / 33 | **34 / 33**（均为实测需要） |
| `tetherly-cli` 测试数 | 0 | **3**（485 条命令路径 + clap 不变量 + 参数边界） |
| 新增共享 crate | — | `tetherly-loopback`（397 行，7 测试） |
| 净代码变化（不含 `Cargo.lock`） | — | 72 文件，+247 / **−754** |

验收：

- `cargo fmt --all -- --check` —— 通过；
- `cargo clippy --workspace --all-targets -- -D warnings` —— 通过（CI 亦设 `RUSTFLAGS: -D warnings`）；
- `cargo test --workspace --locked` —— 全部通过，其中 `tetherly-core` 271、`tetherly-loopback` 7、`tetherly-node` 138、`tetherly-cli` 3、`phase1` 64 / `phase2` 8 / `phase3` 4 / `phase4` 14 / `phase5` 55 / `loopback` 11；
- 行为冒烟：两个二进制各递归遍历 **458 条**规范命令路径（含顶层）的 `--help`，全部退出码 0 且渲染出 `Commands:` / 选项表；别名路径由 `tetherly-cli` 的测试覆盖（合计 485 条）；
- Workspace 版本发布为 `0.1.66`。

## 被否决的替代方案

1. **把回环客户端放进 `tetherly-core`** —— 违反 core 的「无 tokio、无 OS API」边界（ADR-006）；也违背后续 `no_std` 化的可能路径。
2. **把回环客户端放进 `tetherly-net`** —— net 的语义是 P2P 传输与会话，回环 UI 客户端放进去会让 net 的依赖面和评审面同时变宽；且 CLI 不需要 net 的任何其他部分。
3. **给 CLI 加 `tetherly-node` 依赖，直接复用 `uihttp` 的实现** —— 会把 arboard/png/zstd/openh264 拖进 CLI，二进制体积与构建时间都不可接受。
4. **在 `SseStream` 上加 `async fn next_ui_event` 扩展 trait** —— 需要 `#[allow(async_fn_in_trait)]`，且闭包 41 处调用点，收益为零；newtype facade 只多 15 行。
5. **只给 `SseStream` 加整体超时（deadline）** —— 会误杀正常的长时间 watch 会话；按行计时才是心跳协议下正确的不变量。
6. **把 `audit --interval` 保留并实现它** —— `--watch` 的数据源是服务端推送，实现"刷新间隔"意味着改成轮询，反而倒退（丢事件、增负载）；删除是唯一的正确修法。
7. **保留 `tests/phase1.rs` 里的独立编码器以"验证生产实现"** —— 它已经与生产实现漂移（保留字符集不同），留着等于保留一份错误参照系；真正的验证现在是 `tetherly_core::text` 自己的单元测试。
