# ADR-128 CLI 与回环 HTTP 巨型单文件模块化分解（CLI & Loopback HTTP Monolith Decomposition into Feature Submodules）（二阶段）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-11-01，v2.42 / v0.1.64）**——纯工程债重构，零行为变更，两阶段（阶段一 `uihttp.rs`、阶段二 `bin/tetherly.rs`）；(1) **阶段一：`crates/tetherly-node/src/uihttp.rs` 分解**：15655 行单文件拆为 6557 行模块根 + 28 个子模块（26 个 `api_*` 特性域 + `dto` + `util`：`api_action`/`api_ancs`/`api_audio`/`api_audit`/`api_bench`/`api_camera`/`api_clip`/`api_core`/`api_display`/`api_files`/`api_focus`/`api_handoff`/`api_input`/`api_locate`/`api_media`/`api_messages`/`api_mic`/`api_notify`/`api_peers`/`api_power`/`api_screen_analysis`/`api_screen_overlay`/`api_screen_quality`/`api_screen_session`/`api_screen_tile`/`api_topology` + `dto` + `util`），根保留 HTTP/SSE/快照主循环与 `INDEX`（`include_str!`）等共性，子模块各含端点 Handler 与 DTO；(2) **阶段二：`crates/tetherly-node/src/bin/tetherly.rs` 分解为 crate 根 `main.rs` + 37 个子模块**：27402 行单文件拆为 85 行 `main.rs` 根 + 37 个模块（共 38 个文件 28860 行；5 个 `cli*` 表面（`cli`/`cli_tools`/`cli_screen`/`cli_device`/`cli_ops`，共 69 个 `Cmd` 变体 + 60 余个子命令枚举与 59 个 `ScreenCmd` 变体）+ `dispatch`（68 条 arm 派发表）+ 29 个 `cmd_*` 处理器 + `util`），`Cargo.toml` 的 `[[bin]] tetherly` 的 `path` 由 `src/bin/tetherly.rs` 改为 `src/bin/tetherly/main.rs`，共有 293 个 `pub(crate)` 顶项 = 67 个顶层 arm 处理器 + `dispatch`/`screen` 两个派发器 + 53 个 `ScreenCmd` 处理器 + 171 个自由函数原样迁移；(3) **等价性验证（词法 token 流多重集比对，剔除注释、`use` 导入语句、`mod` 声明与可见性修饰符）**：阶段一 `uihttp.rs` 缺失 **0** token、多余 15474 全为 Handler 签名/路由字符串/`#[derive]`/导入等脚手架（上一轮 ≠ 本轮数字，因本轮含 `#[cfg(test)]` 单测模块与运行时校验字段）；阶段二 `bin/tetherly.rs` 缺失 **6** token（仅 6 个 or-pattern 分支 `| ScreenCmd::X { .. }` 别名拆分导致的 `|`，功能不变——`TilePredict`/`Predict` 等 6 对别名各由同一处理器承担，对应 token 全等）、多余 7319 全为脚手架（120 个 `Ok(())` 收尾、Handler 签名、`mod::fn` 限定调用与 `use` 导入），`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（653 项全通过）三闸全绿。 |
| 日期 | 2026-11-01 |
| 关联 | ADR-127「屏幕串流与节点运行时巨型单文件模块化分解」、ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」 |

---

## 动机

1. **工程债清单收敛到最后两处巨型单文件（Remaining Monoliths）**：
   - ADR-127 已拆解 `tetherly-core/src/screen.rs`（25611 行）与 `tetherly-node/src/runtime.rs`（33423 行）；`crates/tetherly-node/src/bin/tetherly.rs`（**27402 行**）与 `crates/tetherly-node/src/uihttp.rs`（**15655 行**）仍是仅存的巨型单文件；
   - 与 ADR-127 相同，巨型单文件导致编辑器索引与增量编译退化、评审 diff 噪声极大、`git blame`/二分定位失效、并行开发高频冲突、任意一次 `cargo fmt` 产生数千行无关 diff。

2. **bin crate 根的模块化有一个既有约束（Crate-Root Module Resolution Constraint）**：
   - `src/bin/tetherly.rs` 是 crate 根（非 `lib.rs` 的子模块），`mod cli;` 会按 crate 根规则解析到 `src/bin/cli.rs` 而非同目录子目录 `src/bin/tetherly/cli.rs`（`E0583`）;
   - 因此 bin 的分解需要一个结构性前提：把 crate 根迁到 `src/bin/tetherly/main.rs` 并同步改 `Cargo.toml` 的 `[[bin]] path`。这与 `uihttp`（`lib.rs` 子模块）不同，不需移动根文件。

---

## 决定

1. **阶段一（`uihttp.rs`）：模块根 + `api_*` 特性子模块，零结构变更**：
   - `crates/tetherly-node/src/uihttp.rs` 保留为 6557 行模块根：SPDX 头、`//!` 文档、`use` 块、常量（`INDEX`/`MAX_HEADER`/`MAX_BODY`）、`mod <feature>;` 声明、HTTP/SSE/快照主循环；
   - `crates/tetherly-node/src/uihttp/` 子模块（`api_*` 按 REST 域、`dto`、`util`）各含端点 Handler 与 DTO，`use super::*;` 引根；根的重用项按需裁剪 `pub(crate)` 或 `pub(super)`（本 crate 内私有），crate 外 API 面不变；
   - 等价性：token 多重集缺失 **0**，多余全为脚手架；
   - 注意：`uihttp` 分解含测试——`cargo test` 653 项全通过（含 REST 单测）。

2. **阶段二（`bin/tetherly.rs`）：crate 根迁移 + `cli*`/`dispatch`/`cmd_*`/`util` 子模块**：
   - **crate 根迁移**：`crates/tetherly-node/src/bin/tetherly.rs` → `crates/tetherly-node/src/bin/tetherly/main.rs`；`Cargo.toml` `[[bin]] name = "tetherly"` 的 `path = "src/bin/tetherly.rs"` 改为 `path = "src/bin/tetherly/main.rs"`；
   - `main.rs`（85 行）只保留：SPDX 头、`use anyhow::Result` 等基本导出、`mod <mod>;` 声明 + `pub(crate) use cli::*;`…（CLI 类型再导出，供 `cmd_*` 经 `use super::*;` 引用）、`#[cfg(unix)] SIGPIPE` 处理、`tracing_subscriber` 初始化与 `Cli::parse()`；
   - `cli*.rs`（5 个）：完整 clap 派生表面——`Cli`/`Cmd`（69 变体）与 60 余个子命令枚举原样迁移，所有顶层枚举与字段改为 `pub(crate)`；`Cmd` 有 68 条派发 arm（`Metrics | Traffic` 共享一臂）；
   - `dispatch.rs`：`pub(crate) async fn dispatch(cli: Cli) -> Result<()>` —— `let mut cfg = NodeConfig::default();`… 初始化 + `match cli.cmd { … }` 68 条 arm 全部保留原位（含 `Cmd::Screen { action }` 委派到 `cmd_screen::screen(action)`），每条 arm body 替换为 `{ mod::handler(args...).await?; }`——**派发表保持可读、可 grep，一处看全**；
   - `cmd_*.rs`（29 个）：每个 `Cmd` arm body 原样迁入 `pub(crate) async fn <name>(...) -> Result<()>`（含 27 个未用参数加 `_` 前缀）；`Cmd::Screen` 拆为 `cmd_screen.rs` 的 `screen(action: ScreenCmd)` 二级派发器，59 个 `ScreenCmd` 变体原样覆盖（6 对 or-pattern 别名 `TilePredict|Predict`、`TilePresent|Present`、`TileNack|Nack`、`TileSaliency|Saliency`、`TileLod|Lod`、`TileBatch|Batch` 各自合并为一个处理器）；
   - `util.rs`：格式化与 URL 编码助手（`format_bytes`/`format_speed`/`format_uptime`/`format_timestamp_ago`/`url_encode`/`urlencode_path`）；
   - 等价性：token 多重集缺失 **6**（仅为 6 个 or-pattern 别名合并的 `|`），多余全为脚手架。

3. **保持公共路径与行为稳定（Zero Public API / Behavior Change）**：
   - clap 完全派生的 CLI 表面不变 → `--help`、子命令树、参数、`tetherly screen predict` 等别名逐一冒烟验证一致；
   - REST 端点路径、Handler 语义、状态机、安全校验（127.0.0.1 物理绑定 + Host 403）逐字保持不变；
   - `uihttp` 仅为本 crate 私有模块，`bin/tetherly` 为独立二进制，均不构成对外库 API，无 crate 外路径承诺。

4. **等价性验证方法（Equivalence Verification，两阶段同法）**：
   - 以词法扫描器把拆分前后源码切分为 token 流（注释/字符串/字符/生字符串/数字/标识符/多字符运算符），剔除注释、`use` 导入语句、`mod` 声明与可见性修饰符后做多重集比对；
   - 验收标准：缺失 token 数必须为 0（阶段一达成；阶段二唯一例外为 6 个 or-pattern 别名的 `|`，属既定合并）、多余 token 仅允许模块脚手架（Handler 签名、`Ok(())` 收尾、`mod::fn` 限定调用、导入与空行）；
   - 多行字符串字面量的行续行缩进在迁移中触发 `cargo fmt` 重排，其值不变（`\<换行><缩进>` 是行续行，被吞掉的缩进不属于字符串值），比对时对字符串 token 先做行续行规范化；
   - 三闸验收：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（653 项全通过）。

---

## 后果

1. **可维护性**：
   - 最大单文件从 27402 行（bin）与 15655 行（uihttp）降至 2228 行（`bin/tetherly/cli_screen.rs`）与 6557 行（`uihttp.rs` 根）或 1227 行（`uihttp/dto.rs`）；全部巨型文件清零；
   - CLI 派发表集中在 `dispatch.rs`，处理器按命令域分派到 `cmd_*`，REST 端点按域分派到 `api_*`，`git blame` 与增量编译恢复。

2. **公共 API 与行为**：
   - 零行为变更：CLI 树、REST 端点、校验语义、日志与遥测全部不变；
   - `uihttp` 是本 crate 私有模块、`bin/tetherly` 是独立二进制，无 crate 外路径承诺。

3. **工程流程**：
   - 并行开发冲突面收敛到按命令/REST 域拆分的子模块；
   - 后续新增命令只需在 `cli.rs` 加变体 + `dispatch.rs` 加 arm + `cmd_x.rs` 加处理器。

4. **遗留**：
   - 全部历史巨型单文件已清零（`screen.rs`/`runtime.rs`/`bin/tetherly.rs`/`uihttp.rs`）；剩余大文件（`bin/tetherly/cli_screen.rs` 2228 行、`cmd_screen_tile.rs` 2177 行、`cmd_core.rs` 1738 行、`api_files.rs` 1115 行）已处于可维护范围，不再需要机械拆分。

---

## 验证

1. **等价性**：
   - 阶段一 `uihttp.rs`：token 流多重集缺失 **0**、多余 15474 全为 Handler 签名/路由字符串/`#[derive]`/导入等脚手架；
   - 阶段二 `bin/tetherly.rs`：缺失 **6**（唯一允许项=6 个 or-pattern 别名 `|`），多余 7319 全为脚手架（120 个 `Ok(())` 收尾、Handler 签名、`mod::fn` 限定调用与导入）；293 个 `pub(crate)` 顶项与拆前逐 arm/逐 fn 切片一一对应。
2. **格式**：`cargo fmt --all -- --check` 通过。
3. **静态检查**：`cargo clippy --workspace --all-targets -- -D warnings` 零告警。
4. **测试**：`cargo test --workspace` 653 项全部通过、0 失败（含 `tetherly-core` 单元测试、`tetherly-node` 运行时与 uihttp 单测、Phase 1~5 端到端集成测试）。
5. **冒烟**：`tetherly --help` / `tetherly screen --help` / `tetherly screen predict --help` / `tetherly --data-dir <d> identity` 输出与拆前 clap 树一致。
6. **版本**：Workspace 版本发布为 `0.1.64`，文档版本 `2.42`。
