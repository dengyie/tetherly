# ADR-129 tetherly-cli 巨型单文件模块化分解（tetherly-cli Monolith Decomposition into Feature Submodules，三阶段收尾）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-11-01，v2.43 / v0.1.65）**——纯工程债重构，零行为变更；ADR-128 已清空 `tetherly-node` 侧的两处巨型单文件，本轮收口最后一处：`bins/tetherly-cli/src/main.rs` 由 **27903 行**拆为 **113 行 crate 根 + 36 个子模块（共 37 文件 29256 行）**：(1) **CLI 表面 5 个模块**——`cli.rs`（847 行，`Cli` + `Cmd` 树）、`cli_tools.rs`（1027 行）、`cli_screen.rs`（2228 行，`ScreenCmd` 与全部 `screen` 子枚举）、`cli_device.rs`（1072 行）、`cli_ops.rs`（580 行），共 68 个子命令枚举（69 个 `Cmd` 变体 + 59 个 `ScreenCmd` 变体）逐字迁移，仅加 `pub(crate)` 可见性；(2) **`dispatch.rs`（418 行）**——`pub(crate) async fn dispatch(cli: Cli) -> Result<()>`，`match cli.cmd { … }` **68 条 arm 全部保留原位**，每臂 body 替换为 `{ mod::handler(args...).await?; }`（模块限定，避免 arm 绑定名 `action`/`alias` 遮蔽同名处理器触发 `E0618`），派发表一处可读可 grep；(3) **`cmd_screen.rs`（70 行）**——`Cmd::Screen` 委派到 `screen(action: ScreenCmd)` 二级派发器，59 个 `ScreenCmd` 变体全覆盖（6 对 or-pattern 别名 `TilePredict|Predict`、`TilePresent|Present`、`TileNack|Nack`、`TileSaliency|Saliency`、`TileLod|Lod`、`TileBatch|Batch` 各拆为两行、指向同一处理器）；(4) **29 个 `cmd_*.rs` 处理器模块 + `util.rs`**——120 个 arm 处理器（67 个顶层 + 53 个 `ScreenCmd`）与 178 个自由函数原样迁入，共 **300 个 `pub(crate)` 函数**（2 个派发器 + 120 个处理器 + 178 个助手），同构于 node 侧 `bin/tetherly`（唯一差异：`Run` 变体被 `Listen` 取代）；(5) **等价性验证（词法 token 流多重集比对，剔除注释、`use` 导入、`mod` 声明与可见性修饰符，并把 `match Cli::parse().cmd` 与 `let cli = Cli::parse(); match cli.cmd` 视为同一构造）**：缺失 **6** token，仅 6 对 or-pattern 别名合并产生的 `|`（功能不变，别名对共享同一处理器、对应 token 全等）；多余 6890 全为脚手架（`pub(crate)`/`async fn` 签名、121 个 `Ok(())` 收尾、`mod::fn` 限定调用与导入）；`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（653 项全通过）三闸全绿；**冒烟以递归遍历 clap 命令树比对通过**：拆前 / 拆后二进制各遍历 **545 条命令路径**（顶层命令 + 全部子命令 + `screen` 子子命令及别名）的 `--help` stdout 与退出码，**差异 0**。 |
| 日期 | 2026-11-01 |
| 关联 | ADR-128「CLI 与回环 HTTP 巨型单文件模块化分解」、ADR-127「屏幕串流与节点运行时巨型单文件模块化分解」、ADR-006「许可证与自包含依赖边界」 |

---

## 动机

1. **工程债清单的最后一处（The Last Monolith）**：
   - ADR-127 拆解了 `tetherly-core/src/screen.rs`（25611 行）与 `tetherly-node/src/runtime.rs`（33423 行）；ADR-128 拆解了 `tetherly-node/src/bin/tetherly.rs`（27402 行）与 `tetherly-node/src/uihttp.rs`（15655 行）；
   - `bins/tetherly-cli/src/main.rs`（**27903 行**）是剩余唯一超过两万行的单文件，症状与前几批完全一致：编辑器索引与增量编译退化、评审 diff 噪声极大、`git blame`/二分定位失效、并行开发高频冲突、任意一次 `cargo fmt` 产生数千行无关 diff。

2. **与 node 侧 `bin/tetherly.rs` 是逐 arm 同构的孪生（Isomorphic Twin）**：
   - 两者共享同一套 clap 派生表面（69 个 `Cmd` 变体、59 个 `ScreenCmd` 变体、68 条派发 arm）与同一批自由函数名（node 的 171 个助手在本文件中全部存在，另有 7 个 loopback/会话助手），唯一结构差异是顶层 "再起一个节点" 的变体在 node 侧叫 `Run`、在 CLI 侧叫 `Listen`；
   - 因此 ADR-128 阶段二验证过的 arm→模块映射（`ARM_MOD`/`SCREEN_MOD`/`FN_MOD`）可直接复用，仅把 `Run` 换成 `Listen` 并补 7 个 CLI 独有助手的落位。

3. **crate 根约束在两侧不同（Different Crate-Root Constraint）**：
   - `tetherly-node` 的 bin 根可迁到 `src/bin/tetherly/main.rs`（`Cargo.toml` 的 `[[bin]] path` 随之改）；
   - `tetherly-cli` 的 `src/main.rs` **就是** crate 根，`mod cli;` 只会解析到同级的 `src/cli.rs`，因此子模块必须**平铺**在 `bins/tetherly-cli/src/` 下（不能折叠进 `src/cli/`），`Cargo.toml` 不需要任何改动。

---

## 决定

1. **模块根 + 平铺特性子模块，零行为变更**：
   - `bins/tetherly-cli/src/main.rs` 保留 113 行：SPDX/`//!` 头、原 `use` 块逐字保留、`mod <mod>;` 声明、`pub(crate) use cli::*;`（CLI 类型再导出，供 `cmd_*` 经 `use super::*;` 引用）、`use dispatch::dispatch;`、`struct SseStream` + `impl SseStream::next_event`（loopback SSE 助手所需的类型，原样内联在根）、`#[tokio::main] async fn main()`（含 SIGPIPE 处理、`tracing_subscriber` 初始化与 `TETHERLY_INSECURE_LOG` 告警原样保留）；
   - 子模块 36 个：5 个 `cli*`（clap 表面）+ `dispatch` + `cmd_screen`（二级派发器）+ 29 个 `cmd_*`（命令域处理器）+ `util`（格式化 / URL 编码 / HTTP 解析 / loopback 客户端）。

2. **派发表保持原位可读（The Dispatch Table Stays Greppable）**：
   - 每个 `Cmd` arm 的 pattern 逐字保留，body 换为一行模块限定调用；`Cmd::Screen` 委派到 `cmd_screen::screen(action).await?`；
   - 模块限定是**必需**而非风格选择：多个 arm 的绑定名（`action`、`alias`、`tag`、`group`、`pin`、`status`…）与同名处理器函数冲突，裸调用会触发 `error[E0618]: expected function, found ...`。

3. **Or-pattern 别名拆分（Alias Arm Splitting）**：
   - 原文 6 条 `A { .. } | B { .. } => …` 合并臂，迁移后每个 `ScreenCmd` 变体各占一行、指向同一处理器；这是本轮唯一的 token 级"差异"，属既定组织变换，行为完全等价（同一个 match 分支体，同样的绑定字段）。

4. **未用参数与收尾（Unused Params & Trailer）**：
   - 处理器形参名若在 body 中不作为标识符出现，则加 `_` 前缀（注释与字符串/字符字面量先剔除再判定），避免 `unused_variables` 告警；
   - 每个处理器末尾补 `Ok(())`，除非 body 最后一条深度 0 语句已是 `return …` / `std::process::exit` / `panic!`。

5. **跨模块助手导入的最小化重建（Minimal Cross-Module Imports）**：
   - 每个 `cmd_*.rs` 只 `use super::*;` 引根，跨模块调用（如 `urlencode_path`、`loopback_post`、`now_ms`）由后处理按"该模块确实调用了该名称"（剔除注释、字符串/字符字面量与 `#[…]` 属性组后按 `(?<![\w.])name\s*\(` 判定）生成 `use super::<mod>::{…};`；
   - 名称在多模块重复定义时视为歧义并跳过（本轮无歧义名称）；`main`/`cli*` 不作为助手导出源，避免把 CLI 类型名误判为函数调用。

6. **等价性验证方法（Equivalence Verification）**：
   - 词法扫描器切 token（注释 / 普通字符串 / 生字符串 / 字符字面量 / 生命周期 / 数字 / 标识符 / 多字符运算符），剔除注释、`use` 语句、`mod x;` 声明与 `pub`/`pub(...)` 后做多重集比对；
   - 字符串 token 先做行续行规范化：`\<换行><缩进>` 是行续行，被吞掉的缩进不属于字符串值，因此 `cargo fmt` 重排多行字面量缩进不改变任何值；
   - **CLI 特有变换**额外声明并抵消：原文件的 `match Cli::parse().cmd {` 在拆分后变为 `let cli = Cli::parse(); … match cli.cmd {`（`dispatch(cli)` 需要 `Cli` 值），比对前先把两侧归一到同一形状；
   - 验收标准：缺失 token 数必须为 0（唯一例外为 6 个 or-pattern 别名的 `|`），多余 token 仅允许模块脚手架。

7. **行为冒烟以命令树全遍历为准（Recursive Help-Tree Smoke）**：
   - 拆前 / 拆后二进制递归解析各自 `--help` 的 `Commands:` 段，逐条路径执行 `<bin> <path> --help`，比对 stdout 与退出码；本轮覆盖 **545 条路径**（含 `screen` 的全部子命令与 `TilePredict`/`Predict` 等别名），差异 0 —— 这比逐个手工冒烟更强的证据，因为 clap 派生树是本轮唯一被"触碰"的对外面。

---

## 后果

1. **可维护性**：
   - 最大单文件从 27903 行降至 2228 行（`cli_screen.rs`）；`main.rs` 113 行、`dispatch.rs` 418 行、`cli.rs` 847 行、`cmd_screen.rs` 70 行；
   - 全仓库历史巨型单文件**彻底清零**（`screen.rs`/`runtime.rs`/`bin/tetherly.rs`/`uihttp.rs`/`bins/tetherly-cli/src/main.rs` 五处全部落地）；
   - `git blame` 与增量编译恢复；并行开发冲突面收敛到按命令域拆分的子模块。

2. **公共 API 与行为**：
   - 零行为变更：clap 命令树（545 条路径）、参数、帮助文本、校验语义、日志与遥测全部不变；
   - `tetherly-cli` 是独立二进制 crate，无库 API 承诺，`Cargo.toml` 未改动。

3. **工程流程**：
   - 新增一条 CLI 命令仍然只需三处改动：`cli*.rs` 加变体 → `dispatch.rs` 加 arm → `cmd_x.rs` 加处理器；
   - node 与 cli 两套孪生 CLI 现在结构完全对称，跨侧同步改动可直接对照同一文件位置。

4. **遗留**：
   - 无。全部巨型单文件清偿完毕；剩余较大文件（`cli_screen.rs` 2228 行、`cmd_screen_tile.rs` 2155 行、`cmd_files.rs` 1717 行、`cmd_screen_session.rs` 1635 行）已在可维护范围，不再需要机械拆分。

---

## 验证

1. **等价性**：`bins/tetherly-cli/src/main.rs` 27903 行 → 113 行根 + 36 子模块（37 文件 29256 行）；token 流多重集**缺失 6**（仅 6 个 or-pattern 别名的 `|`）、**多余 6890** 全为脚手架（`pub(crate)`/`async fn` 签名、121 个 `Ok(())` 收尾、`mod::fn` 限定调用、`use super::<mod>::{…}` 导入、`mod` 声明与空行）。
2. **规模对应**：300 个 `pub(crate)` 函数 = `dispatch`/`screen` 2 个派发器 + 120 个 arm 处理器（67 顶层 + 53 `ScreenCmd`）+ 178 个自由函数；68 个 `pub(crate) enum` 与 68 条派发 arm 一一对应。
3. **格式**：`cargo fmt --all -- --check` 通过。
4. **静态检查**：`cargo clippy --workspace --all-targets -- -D warnings` 零告警。
5. **测试**：`cargo test --workspace` 653 项通过 / 0 失败。
6. **冒烟**：拆前 / 拆后二进制各遍历 545 条 clap 命令路径的 `--help`，stdout 与退出码差异 **0**。
7. **版本**：Workspace 版本发布为 `0.1.65`，文档版本 `2.43`。
