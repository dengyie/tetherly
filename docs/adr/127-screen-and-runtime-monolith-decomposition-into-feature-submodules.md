# ADR-127 屏幕串流与节点运行时巨型单文件模块化分解 (Screen Streaming & Node Runtime Monolith Decomposition into Feature Submodules)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-11-01，v2.41 / v0.1.63）**——纯工程债重构，零行为变更：(1) **`crates/tetherly-core/src/screen.rs` 分解**：25611 行单文件拆分为 88 行模块根 + 35 个特性子模块（`wire` / `control` / `adapt` / `fec` / `tile` / `tile_compress` / `tile_schedule` / `tile_lod` / `sender` / `receiver` / `recording` / `playback` / `watermark` / `annotation` / `roi` / `color` / `delta` / `qoe` / `resilience` / `motion` / `probe` / `twcc` / `pacing` / `retransmit` / `squelch` / `damage` / `playout` / `profiler` / `tuner` / `audio_sync` / `cursor` / `overlay` / `sink` / `tests` / `tests_pipelines`），模块根以 `pub use <mod>::*;` 全量再导出，`tetherly_core::screen::X` 路径对全部调用方保持稳定；(2) **`crates/tetherly-node/src/runtime.rs` 分解**：33423 行单文件拆分为 2941 行模块根 + 30 个特性子模块（`trust` / `discovery` / `notify` / `session` / `config` / `overlay` / `routes` / `messages` / `audit` / `storage` / `files` / `clipboard` / `handoff` / `media` / `locate` / `actions` / `focus` / `topology` / `input` / `virtual_display` / `audio` / `camera` / `bench` / `lifecycle` / `screen_stream` / `screen_control` / `screen_media` / `screen_mode` / `screen_status` / `screen_dto`），570 个 `impl Node` 方法按特性域迁入子模块并以 `pub(super)` 暴露，模块根保留 `Node` / `Shared` / `NodeConfig` / `UserCommand` / `Hubs` 与常量，使子模块凭祖先可见性继续访问私有字段；(3) **可见性策略**：跨兄弟模块访问的顶层项、结构体字段与固有方法统一放宽为 `pub(super)`，`impl Trait for T` 的方法不加可见性限定符；(4) **导入策略**：原 89 行 `use` 块按顶层 `;` 切分，逐模块按实际引用名重新生成最小导入集，测试专用导入以逐行 `#[cfg(test)]` 标注；(5) **等价性验证**：以词法 token 流多重集比对（剔除注释、`use` 导入语句、`mod` 声明与可见性修饰符）：`runtime.rs` 缺失 0 token，多余 token 全为脚手架（1 个原 `impl Node` 头由 29 个模块级块取代、测试专用 `#[cfg(test)]` 导入属性、可见性加长引发的 rustfmt 换行）；`screen.rs` 缺失仅原内联测试模块的成对花括号（改为 `mod tests;` / `mod tests_pipelines;` 文件模块），零方法体/表达式 token 丢失，`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`（653 项全通过）三闸全绿。 |
| 日期 | 2026-11-01 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-030「屏幕串流基础帧协议」、ADR-126「屏幕串流动态空间瓦片局部运动矢量估计与跨帧位移复用流水线」 |

---

## 动机

1. **单文件规模越过可维护性阈值（Monolith Beyond Maintainability Threshold）**：
   - `crates/tetherly-core/src/screen.rs` 累积至 **25611 行**，`crates/tetherly-node/src/runtime.rs` 累积至 **33423 行**，其中单个 `impl Node` 块独占 **27152 行 / 570 个方法**（`attach_screen_client` 单方法 1907 行、`screen_send_frame` 1766 行、`attach_screen_server` 811 行、`screen_status_report` 801 行）；
   - 后果：编辑器索引与增量编译退化、代码评审 diff 噪声极大、`git blame` 与二分定位失效、多人并行开发在同一文件上高频冲突、任何一次 `cargo fmt` 都会产生数千行无关 diff。

2. **特性域边界在文件内已客观存在但未被编译器强制（Implicit Feature Boundaries）**：
   - 两个文件内部早已按特性域形成清晰分区（屏幕串流的线协议编解码、控制信封、自适应、FEC、瓦片族、录制回放、水印标注；节点运行时的信任、发现、通知、路由、消息、审计、存储、文件、剪贴板、交接、媒体、定位、动作、焦点、拓扑、输入、虚拟显示、音频、摄像头、基准、生命周期、屏幕串流族）；
   - 但这些边界仅靠注释与命名约定维持，缺少模块系统层面的强制隔离，跨域私有访问与隐式耦合无法被编译器发现。

3. **重构必须零行为变更（Zero Behavioral Change）**：
   - 项目处于 v2.40 / v0.1.62 已发布状态，屏幕串流流水线（ADR-030 ~ ADR-126）与节点运行时行为均为合同的一部分；
   - 因此本次分解只允许改变**代码组织形态**，不允许改变任何线协议、REST 端点、CLI 输出、遥测字段或运行时语义。

---

## 决定

1. **采用「模块根 + 特性子模块目录」布局（`file.rs` + `file/` 并存，无需 `mod.rs`）**：
   - `crates/tetherly-core/src/screen.rs` 保留为 88 行模块根：SPDX 头、模块级 `//!` 文档（含布局说明）、`mod <feature>;` 声明、`pub use <feature>::*;` 全量再导出、`#[cfg(test)] mod tests;` 与 `#[cfg(test)] mod tests_pipelines;`；
   - `crates/tetherly-node/src/runtime.rs` 保留为 2941 行模块根：SPDX 头、模块级 `//!` 文档、按引用裁剪后的 `use` 块、`mod <feature>;` 声明、`pub use <feature>::*;` 再导出、节点自身状态（`Node` / `Shared` / `NodeConfig` / `UserCommand` / `Hubs` / `OsRng` / 常量 / `node_accept_loop` / 自由函数 / 内联测试模块）。

2. **再导出保证公共路径稳定（Path Stability via Glob Re-export）**：
   - 模块根对每个子模块执行 `pub use <mod>::*;`，使 `tetherly_core::screen::X`、`crate::runtime::X` 以及 `lib.rs` 的 `pub use runtime::{...}` 与 `uihttp.rs` 的 `crate::runtime::{...}` 全部无需改动；
   - 仅含 `impl Node` 方法而无顶层项的子模块不生成 `pub use`（避免空 glob 导入告警）。

3. **可见性放宽策略（Visibility Widening）**：
   - 顶层项：原私有 `struct` / `enum` / `trait` / `type` / `const` / `static` / `fn` 放宽为 `pub(super)`；原 `pub` 保持不变；
   - 结构体字段：迁出模块的结构体私有字段放宽为 `pub(super)`，使兄弟模块可构造与读取；
   - 固有方法：`impl Node` 与其它固有 `impl` 块内的私有 `fn` / `const` 放宽为 `pub(super)`；
   - **`impl Trait for T` 的方法不加任何可见性限定符**（Rust 不允许 trait 实现方法带可见性）；
   - 依据：`pub(super)` 在 `runtime` 模块内对全部兄弟子模块与后代（含内联测试模块）可见，同时不扩大 crate 外部 API 面。

4. **祖先私有可见性用于节点状态（Ancestor Privacy for Node State）**：
   - `Node` / `Shared` / `NodeConfig` / `UserCommand` / `Hubs` 及其私有字段保留在 `runtime.rs` 模块根，不迁入子模块；
   - 子模块作为 `runtime` 的后代，凭 Rust 祖先私有可见性直接访问 `self.shared.*` 等私有字段，无需为状态结构体做任何字段放宽，最大限度缩小可见性改动面。

5. **导入最小化（Per-module Minimal Imports）**：
   - 原 89 行 `use` 块按顶层 `;` 切分为 27 条语句，解析出每条语句绑定的名字（含 `as` 别名与 `{self, ...}` 组）；
   - 每个子模块按自身正文（剔除注释后）实际引用的名字重新生成导入语句，组导入按需裁剪成员；
   - 测试专用导入以逐行 `#[cfg(test)]` 标注，避免非测试构建产生未使用导入告警。

6. **等价性验证方法（Equivalence Verification）**：
   - 以词法扫描器把拆分前后源码切分为 token 流，剔除注释、`use` 导入语句、`mod` 声明与可见性修饰符后做多重集比对；
   - 验收标准：缺失 token 数必须为 0（唯一允许项为原内联 `#[cfg(test)] mod tests { }` 改为 `mod tests;` / `mod tests_pipelines;` 文件模块所致的成对花括号），多余 token 仅允许模块脚手架（模块级 `impl Node` 头、测试专用 `#[cfg(test)]` 属性、可见性加长引发的 rustfmt 换行与尾逗号）；
   - 三闸验收：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`。

---

## 后果

1. **可维护性**：
   - 最大单文件从 33423 行降至 2941 行（`runtime.rs`）与 88 行（`screen.rs`）；最大子模块为 `screen_stream.rs` 4793 行、`tests_pipelines.rs` 5581 行；
   - 特性域边界由模块系统强制，跨域访问必须显式 `pub(super)`，隐式耦合在编译期暴露。

2. **公共 API 与行为**：
   - 零行为变更：线协议、REST 端点、CLI 输出、遥测字段、运行时语义全部不变；
   - 公共路径不变：`tetherly_core::screen::X`、`crate::runtime::X`、`lib.rs` 再导出列表、`uihttp.rs` 引用全部无需修改；
   - 可见性放宽仅限 `pub(super)`，crate 外部 API 面未扩大。

3. **工程流程**：
   - 并行开发冲突面从 2 个巨型文件收敛到 65 个特性子模块；
   - `git blame` 与二分定位恢复可用；`cargo fmt` 不再产生跨域无关 diff。

4. **遗留**：
   - `crates/tetherly-node/src/bin/tetherly.rs`（27402 行）与 `crates/tetherly-node/src/uihttp.rs`（15655 行）当时仍为巨型单文件，属后续工程债，本次不处理（**已由 ADR-128 于 v2.42 / v0.1.64 清偿**）；
   - `crates/tetherly-node/src/screen.rs`（2379 行）为节点侧屏幕辅助模块，与本次分解的 `tetherly-core` 屏幕协议模块不同，未纳入本次范围。

---

## 验证

1. **等价性**：token 流多重集比对（剔除注释、`use`、`mod` 声明与可见性修饰符）：`runtime.rs` 缺失 0 token、多余 186 个全为脚手架（1 个原 `impl Node {` 头由 29 个模块级块取代 + 10 个测试专用 `#[cfg(test)]` 导入属性 + rustfmt 换行与尾逗号）；`screen.rs` 缺失 2 token（原内联测试模块的成对花括号）、多余 8 token（2 个 `#[cfg(test)]` 属性 + 1 个尾逗号），零方法体 token 丢失。
2. **格式**：`cargo fmt --all -- --check` 通过。
3. **静态检查**：`cargo clippy --workspace --all-targets -- -D warnings` 零告警。
4. **测试**：`cargo test --workspace` 653 项全部通过、0 失败（含 `tetherly-core` 屏幕串流单元测试、`tetherly-node` 运行时内联测试模块与 Phase 5 端到端集成测试）。
5. **版本**：Workspace 版本发布为 `0.1.63`，文档版本 `2.41`。
