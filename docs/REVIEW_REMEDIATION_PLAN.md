# 深度 Review 修复路线图

## 速读（当前有效 · 维护于 2026-11-19）

- 当前审计基线为 workspace `v0.2.65`、review snapshot `3609ca1`；本文件是 review 实施合同，不替代 `docs/DEVELOPMENT.md` 的实现合同。
- 深度 review 已确认发布阻断项：本地权限边界、HTTP/文件资源上界、会话 teardown 原子性、Android/release/deploy 门禁均存在待修问题；在 P1 修复和定向回归完成前禁止新 tag、release、生产部署。
- 修复顺序固定为：取证基线 → P1 安全/资源/发布阻断 → P2 生命周期与可靠性 → CI/release/deploy gate → 仅以 GitHub CI 和现场证据判定可发布。
- 当前修复轮已完成 Batch 1、Batch 2 主要资源边界、Batch 3 主要状态原子性和 Batch 4 clipboard/browser/Tauri 的本地代码与定向 GREEN；未运行 GitHub CI、Android/Windows/macOS release、生产部署或真实设备验收，故本文件仍不是发布批准。
- 当前明确残余为 timeout/cancel/shutdown 后 partial/sidecar 策略的现场确认、Android 真机/Gradle 证据，以及三平台 CI、artifact provenance、现场 readiness/rollback。multi-peer 失败分支和 shutdown 已统一复用 terminal transition，清理 pending/cancel 状态并更新 outbox/stats；两条高层集成证据（inbound stalled-transfer、multi-peer queue-full 隔离）已按 RED→GREEN 落地。发布冻结继续有效。
- `Cargo.toml`、tag 和现场 health 是二进制版本依据；仓库 `DEVELOPMENT.md` 顶部、Obsidian canonical 速读和 `.codex-memory/todo.md` 存在历史版本漂移，不能互相静默覆盖。

## 1. 范围与判定口径

### 1.1 Review 范围

覆盖以下边界：

- `tetherly-core`：输入、文件、通知、状态机和协议不变量。
- `tetherly-net`：Noise session、文件通道、framing、backpressure、rekey 和 cancellation。
- `tetherly-node`：HTTP/API、token tier、文件存储/传输、screen/input/session 生命周期、平台 hook。
- `client` / Tauri / Android：SSE、quick reply、foreground service、前端构建和桌面 bundle。
- CI、release、部署和 smoke/crossing 脚本：版本、产物、readiness、rollback、API contract。

### 1.2 Finding 状态

- **confirmed**：有静态代码证据、可达触发条件和明确生产影响，进入修复批次。
- **needs confirmation**：依赖威胁模型、平台语义或运行时并发时序；先补测试/取证，不直接当作已确认漏洞。
- **capability gap**：产品能力尚未实现，不应伪装成当前 bug 修复。

每个 confirmed finding 在实现时必须补齐：

1. RED：能稳定证明缺陷的定向测试或脚本证据。
2. GREEN：修复后同一证据通过。
3. CI：对应 crate、平台矩阵或前端/Android job 通过。
4. Release/deploy：只有涉及发布或生产行为时才记录 tag、artifact、health 和回滚证据。

## 2. Findings 登记

### 2.1 P1：权限与本地副作用

| 状态 | 问题与证据 | 影响 | 修复合同 |
|---|---|---|---|
| confirmed | **Write-tier 路径发送越过本地权限边界**。`crates/tetherly-node/src/uihttp/v1.rs:1586-1646` 只检查 supplied path 是普通文件且不超过 `FILE_MAX`；`runtime/files.rs:976-1019,1853-1884` 由 daemon 读取并发送；Windows 部署以 LocalSystem 运行。 | 持有 `client.token` 的本地进程可要求 SYSTEM 读取并外传 SYSTEM 可访问文件，包括 `identity.bin`。 | 将路径限制在受控 data/export 目录，或增加显式用户授权；补 client-token、SYSTEM、paired destination 和拒绝敏感路径的回归测试。若产品明确允许任意路径，必须把该信任边界写入架构文档并接受风险。 |
| confirmed | **Read-tier GET 接口执行任意路径写入**。screen snapshot、recording snapshot、whiteboard export 的 `out` 参数位于 `uihttp/v1.rs:4405-4438,4461-4473,4686-4699`；`screen_media.rs:547-565,645-660,935-955` 创建目录并 `std::fs::write()`。 | 低权限 client token 可借助高权限 daemon 截断/覆盖其自身无权写入的文件。 | 改为 Write 级 POST；输出限制在专用 export directory；拒绝绝对路径、路径穿越和默认覆盖；补授权、路径 containment、existing-target 和 SYSTEM 回归。 |
| confirmed | **legacy SSE 绕过 v1 Read token**。`uihttp.rs:307-348` 在 token gate 前把 `/api/events` 与 `/api/events/stream` 交给 `handle_sse()`；事件 handler `:528-590` 无 token。 | 任意本地进程可读取设备、peer、文件名/大小、通知标题/app、handoff、媒体等元数据；目前未证实 OTP 数字、clipboard/body 明文泄露。 | 迁移调用方到 `/api/v1/events`，或给 legacy SSE 加 Read tier 并移除不必要 wildcard CORS；补无 token/Read token/跨 origin 测试。 |

### 2.2 P1：内存、连接和 session liveness

| 状态 | 问题与证据 | 影响 | 修复合同 |
|---|---|---|---|
| confirmed | **raw inbox download 完整读入内存**。`runtime/storage.rs:782-815` 使用 `std::fs::read()`；`uihttp/v1.rs:5880-5889` 构造完整 `UiResponse<Vec<u8>>`；`uihttp.rs:435-472` 再完整写出；`FILE_MAX` 为 64 GiB，而 `MAX_BODY` 只约束请求体。 | Read-tier 调用方可触发巨大分配、swap/OOM 或机器卡顿，尤其在慢下载客户端下 body 长时间存活。 | 采用文件句柄流式响应、独立下载上限、异步/阻塞隔离和响应写 timeout；补大文件内存上界、慢客户端和取消测试。 |
| confirmed | **文件端口 pre-auth 空闲连接无上界**。`lifecycle.rs:738-807` 在 LAN/overlay 监听；`runtime.rs:1238-1265` 每连接直接 spawn；`files.rs:224-232` 先读未认证 header；`filechan.rs:159-187`、`codec.rs:29-38` 使用无 deadline 的 `read_exact()`。 | 无需配对、token 或有效 transfer ID，连接后不发数据即可永久占用 socket、task、fd，造成文件服务 DoS。 | header 短 deadline；全局/per-peer semaphore；连接空闲、异常和 shutdown 统一回收；补空连接、半 header、并发上限和 listener shutdown 测试。 |
| confirmed | **已接受/出站文件传输无 end-to-end deadline**。`files.rs:366-407` 只与用户 cancel 竞争；`filechan.rs:264-307` chunk read/write 无 timeout；出站 `TcpStream::connect` 与 streaming 在 `files.rs:1850-1949` 无统一 deadline。 | peer 停顿可长期占用 task、socket、`.part`、active transfer、resume state，重复 stalled transfer 可耗尽状态预算。 | 分离 connect、header、chunk idle、total transfer deadline；超时进入 terminal failure 并清理 cancel sender、file handle、sidecar、active state；补 inbound/outbound stall tests。 |
| confirmed | **`recv_inner()` 通过 `flush_outbox()` 绕过写 timeout**。`session.rs:141-176` 的 `recv_inner()` 先 flush，只有 `send_bounded()` 才包 `SESSION_WRITE_TIMEOUT`；screen pump 在 `screen_stream.rs:1197-1200` 直接调用。 | 对端保持连接但停止读取时，接收 pump 可卡在 writer，阻塞入站处理、shutdown 和 teardown。 | 统一所有 writer flush 的 deadline，或拆分独立 writer task；stalled writer 必须触发 session teardown；补 recv-path stalled-writer 回归。 |

### 2.3 P1：发布和部署门禁

| 状态 | 问题与证据 | 影响 | 修复合同 |
|---|---|---|---|
| confirmed | **Android release 发布 debug APK 且版本过期**。`release.yml:173-184` 执行 `assembleDebug` 并上传 `app-debug.apk`；`android/app/build.gradle.kts:10-15` 仍为 `versionCode=1`、`versionName=0.1.0`。 | 发布物不是生产 release variant，升级/version identity 不可信。 | 使用 `assembleRelease`、正式 CI signing、单一版本注入；publish 前解析 APK manifest 并校验 variant、签名和版本。 |
| confirmed | **manual release tag 未绑定 checkout revision**。`release.yml:5-10,192-223` 接受任意 `inputs.tag`，build jobs 未按该 tag checkout，也无跨文件 version gate。 | 不同源码可被发布为任意 tag，无法可靠复现。 | dispatch 时 checkout tag/commit；校验 tag、Cargo/package/Tauri/Android 版本；不一致直接失败。 |
| confirmed | **Tauri bundle 使用未验证的 committed `client/dist`**。`tauri.conf.json:6-8` 指向 `../dist`；`release.yml:113-116` 明确不构建 frontend；CI 无 pnpm/check/test/build gate。 | source 更新后 installer 仍可能嵌入旧 JS/CSS。 | release 中固定 Node/pnpm 并构建 frontend；CI 加 `pnpm check/test/build`；若保留 committed dist，增加 source-to-dist 一致性校验。 |
| confirmed | **smoke/crossing 脚本仍使用已删除 legacy API**。`scripts/verify_cross_platform_smoke.sh`、`scripts/verify_crossing.py` 仍调用 `/api/health`、`/api/input/*`、`/api/clip/*` 等，而当前 contract 是 `/api/v1/*`。 | 验收脚本无法证明当前 release，静默 curl 还可能把旧路由失败伪装成业务失败。 | 全部迁移 `/api/v1`、加入 token tier 和严格 status 检查；把脚本纳入 release gate。 |
| confirmed | **Windows restart/deploy 无 replacement readiness/rollback**。`lifecycle.rs:964-993` spawn 成功即返回；`deploy.sh:220-238` 等待后单次 health，未确认 PID/version/start time，也不恢复旧 binary。 | 新进程初始化失败或旧进程仍响应时可误报成功，失败会留下不可用主机。 | 保留旧 binary/config；replacement 独立 readiness probe；验证 PID/version/start time；失败自动回滚并启动旧版本。 |
| confirmed | **release asset 可被静默覆盖且下载无 provenance gate**。`release.yml:215-216` 使用 `--clobber`；deploy/package 未做 checksum/signature/attestation 校验。 | rerun 可替换已发布物；错误或被替换的 artifact 可进入生产。 | 默认 immutable release；重复发布比较 digest 或显式批准；部署前验证 SHA-256、签名或 GitHub attestation。 |
| confirmed | **Android 15 `dataSync` foreground service 无 timeout/recovery**。target SDK 35、manifest `dataSync`、`ForegroundKeep.kt:13-45` 长期 `startForeground()`，无超时回调/恢复。 | 系统停止服务后通知会话可能静默消失。 | 按 Android 15 规则重新选择 service 设计；实现 timeout、重启/重连和 UI 可观测状态；补系统停止后的恢复测试。 |

### 2.4 P2：状态原子性、teardown 和可靠性

- **screen replacement teardown**：`screen_stream.rs:930-959` generation 检查后才异步清 `screen_peer/screen_out`，且 privacy flags 在 `still_active` 外无条件清零。旧 pump 可清理新 session。修复必须让所有 session-scoped state 共享 generation ownership，并覆盖 replacement race 测试。
- **screen control failure cleanup**：`screen_control.rs:95-141` 的 `screen_start` 在发送失败时可能留下 odd ticket；`:217-225` 的 `screen_stop` 在发送失败时不清 privacy flags。使用统一 guard/finally-style cleanup，补 stale Stop 和 send failure 测试。
- **inbox 同名 TOCTOU**：`files.rs:424-439,2319-2337` 先 exists-check 再 rename，Unix 并发同名 transfer 可覆盖。用独占占位/`create_new` 或原子目标预留，补并发同名测试。
- **camera detach ownership**：`camera.rs:212-219` 先 `take()` 再验证 session/peer。先只读验证所有权，再移除；补跨 peer detach 测试。
- **outbound offer rollback**：`files.rs:906-968,1029-1092` 在 `send_to()` 前发布 pending/active/outbox，发送失败不回滚。失败必须移除 payload、标记 terminal failure、清理 outbox 并发事件。
- **inbound accept rollback**：`files.rs:477-524` 先把 offer 变 Accepted 并建 active transfer，再发送 accept；失败后不能再 reject/accept。需要事务化发送或可逆状态转换，补 queue-full/peer-gone 测试。
- **Android quick reply**：`NotifyListener.kt:48-75` 只在新通知时 drain，并 destructive take 后丢弃失败项。reply arrival 触发 drain，失败项限时重试/requeue。
- **clipboard partial delivery**：`clipboard.rs:460-513` 忽略逐 peer `send_to()` 错误，先记 hash 和成功计数。改为 per-peer result、retry 或明确 partial delivery，补 queue-full 测试。
- **browser/Tauri SSE**：browser `http.ts:9,52-75` 声明 idle constant 但未使用；Tauri `events.rs:157-168` 未验证 HTTP status 就发 Established。补 half-open、401/500、reconnect 和 status gate 测试。

### 2.5 P3/低优先级

- ~~authenticated storage/open API 返回绝对 filesystem path~~：**已修复**。`GET /api/v1/storage` 的 `data_dir`/`inbox_dir` 与 inbox open 报告的 `path` 现按 tier 分级：Admin（daemon token，CLI/运维）保留真实路径；Read/Write（client token）返回 `<data-dir>`/`inbox/<name>` 相对占位。回归：`uihttp::v1::tests::inbox_open_path_is_redacted_below_admin` + `v1_file_domains_serve_and_gate` tier 断言；node lib 315/315。
- ~~macOS packaging 将 Admin token 放入 curl argv~~：**已修复**。`package-macos-app.sh` 两处 curl 改用 stdin config（`curl --config -`），token 不再出现在进程 argv；`bash -n` 通过。
- Windows smooth-wheel 子 detent 可能被 `/120` 整数除法丢弃；是否违反产品契约需先确认。

## 3. 修复批次合同

### Batch 0：发布冻结与证据基线

- 暂停新 tag、release、deploy；固定 Cargo/tag/health、当前 artifact、API 路由和 threat model。
- 迁移并运行 smoke/crossing 脚本前先记录旧路由失败证据。
- 明确 client token 的调用者身份和 Windows LocalSystem 威胁模型。

### Batch 1：权限边界与本地副作用

- 路径发送 containment/授权；导出接口动词、tier 和输出目录；legacy SSE Read auth。
- RED/GREEN：client token、绝对路径、SYSTEM readable file、existing target、无 token SSE、Read token SSE、cross-origin。
- 完成本批前不接受新的本地 API 能力扩展。

### Batch 2：内存、连接与写入生命周期上界

- raw download 流式化；file listener admission/header timeout；transfer connect/chunk/total deadline；统一 session writer timeout。
- RED/GREEN：大文件、慢客户端、空连接、半 header、stalled writer、取消、shutdown、inbound/outbound stall。
- 资源上界必须在代码中表达，不能只靠 OS backlog、`FILE_MAX` 或固定 sleep。

### Batch 3：状态机原子性与 teardown

- screen generation/privacy cleanup、screen start/stop failure cleanup；offer/accept rollback；duplicate transfer admission；同名目标原子占位；camera ownership。
- RED/GREEN：replacement race、queue full、peer disconnect、双连接 transfer、并发同名文件、跨 peer detach。
- 每个共享 map/sidecar/cancel sender 都要有 terminal-state 或 ownership 证明。

### Batch 4：客户端可靠性

- clipboard partial retry；Android quick reply retry；Android 15 service 生命周期；browser/Tauri SSE liveness/status。
- RED/GREEN：queue-full、延迟 reply、通知消失、PendingIntent failure、half-open SSE、401/500 response。

### Batch 5：CI、release、deploy gate

- manual tag checkout/version gate；frontend build/check/test/dist consistency；Android release variant/signing/version；artifact digest/provenance；immutable assets；Windows readiness/rollback；smoke/crossing `/api/v1` 迁移。
- 只有 Rust 三平台、Android、deny、frontend/Tauri、脚本 smoke、release artifact inspection 全部通过，才允许 tag/release/deploy。

## 4. 测试矩阵与入口

### Rust/core/net

- `crates/tetherly-core/tests/{decoders.rs,fuzz_decoders.rs,state_machines.rs}`
- `crates/tetherly-net/tests/{decoders.rs,backpressure.rs,rekey.rs,screen_wire.rs}`
- `crates/tetherly-crypto/tests/limits.rs`
- `crates/tetherly-api/tests/contract.rs`
- `tests/loopback.rs`、`tests/phase1.rs` 至 `tests/phase5.rs`

### Client/Tauri/Android

- `client/package.json` 中的 build/test/check；`client/vitest.config.ts`、`client/playwright.config.ts`、`client/e2e/`。
- Android unit test、release APK manifest/signature/version inspection，以及 target SDK 35 service lifecycle evidence。
- Tauri SSE status/sequence contract 和 bundle source-to-dist consistency。

### CI/release

- Rust：`cargo fmt --all -- --check`、三平台 `cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace --locked`。
- Android：cargo-ndk、unit tests、正式 release variant；不得把 debug assemble 误当 release 验收。
- Dependency：`cargo deny check advisories licenses bans sources`。
- Release：Linux glibc floor、desktop binaries、Tauri bundles、Android artifacts、publish digest/provenance 和 immutable asset policy。

历史“全量测试全绿”只能作为背景，不能替代本路线图每项 finding 的定向 RED/GREEN 证据。

## 5. Needs confirmation 与不纳入 bugfix

### Needs confirmation

- client token 是否有意允许 daemon 以更高身份读取任意本地路径；若同机低权限进程属于威胁模型，路径发送和导出写入必须按 P1 权限漏洞处理。
- ~~macOS hook 并发启动、Windows/macOS hook shutdown 是否要求 in-process 生命周期~~：**2026-11-20 已取证并修复**。并发启动竞态为 confirmed：`ensure_input_capture` 原为对 `input_capture_active` 的 check-then-act，而该标志在 `start_input_hooks` 返回（tap 就绪阻塞期间）后才置位——1s link-watcher tick 与 tunnel accept 路径可双双通过门控重复 spawn；macOS 侧第二次 `start_input_hooks` 无任何守卫，会产生孤儿 CGEventTap（持无主 `keyboard_armed` Arc，按陈旧状态吞/放事件），Windows 侧 SENDER OnceLock 会拒绝但此前已执行无谓的 `system_cursor::restore(true)`。修复：`Shared.input_capture_spawning` 原子单飞占坑（`try_claim/release`，失败重试可再占）+ macOS 进程级 `HOOKS_STARTED` 守卫（拒绝时报 "input hooks already started"，Accessibility 未授权路径释放占坑允许后续重试）。Shutdown 定级为 confirmed design：tap 设计为进程生命周期（dock-swipe 注释明示 teardown at process exit），会话拆线只经 `set_keyboard` disarm，不做 in-process 拆除。回归：`capture_spawn_claim_admits_exactly_one_contenting_caller`（8 线程竞争恰 1 胜者 + 释放可再占）；node lib 318/318。
- ~~Windows smooth-wheel 子 detent 是否属于产品协议承诺~~：**2026-11-20 已确认并修复**。用户拍板：平滑滚轮是产品承诺。取证：Windows hook `raw / 120` 每事件独立整除，高分辨率滚轮的子 detent 增量（|raw|<120）被整体丢弃且不跨事件累积，精度滚轮/触控板滚动在远程端无响应直至单事件凑满 120。修复：`wheel_accumulate` 纯函数累积器（truncated division 保号、余数携带待发方向、饱和上界防敌意流漂移）+ `hook_wheel_state` thread-local 双轴余数（LL 滚轮事件单线程纪律同 `hook_move_state`），WM_MOUSEWHEEL/HWHEEL 均改走累积路径，零行事件不再推空帧。wire 协议不变（行单位），macOS/目标端注入不动。回归：`wheel_accumulator_emits_detents_across_sub_delta_events`（子 detent 跨事件凑格、整格直通、负向保号、正负互抵不虚构滚动、敌意流饱和）；node lib 319/319。

### Capability gap，不是本轮 bugfix

- Windows session viewer 目前是 wire-side-only 诚实 stub。
- Edge Strip、真实 Ghost Overlay、被动跨屏拖拽原生窗口层未实施。
- 多显示器输入注入当前只寻址主屏，需要独立虚拟桌面/per-display 设计。

这些能力缺口已在 `docs/DEVELOPMENT.md §171` 和 ADR-137/138 语境中登记，不应借本修复批次伪装成已完成。

## 6. Release checklist

发布前必须全部满足：

- [ ] Batch 1–4 的 P1/P2 finding 均有 RED/GREEN 链接和对应测试结果。
- [ ] Rust 三平台 CI、Android、cargo-deny 全绿。
- [ ] client build/check/test 和 Playwright/Tauri 关键路径已执行；bundle 使用当前 source 构建的 dist。
- [ ] Android artifact 是 release variant，版本、签名和 native `.so` 对齐。
- [ ] release tag checkout 与四文件版本一致；artifact digest/provenance 通过；禁止无审计 `--clobber`。
- [ ] Windows replacement readiness 已确认新 PID/version/start time；失败可回滚旧 binary。
- [ ] smoke/crossing 脚本只使用当前 `/api/v1` contract，并严格检查 HTTP status。
- [ ] Mac/Windows health、SYSTEM 身份、受保护进程和回滚锚点均有现场证据。
- [ ] 物理输入、屏幕、文件投递和 Android 长稳等 Manual-required 项单独记录，不用模拟测试替代。

## 7. 关联入口

- 仓库实现合同：[`DEVELOPMENT.md`](DEVELOPMENT.md)
- API 迁移声明：[`LEGACY_API_MAP.md`](LEGACY_API_MAP.md)
- 控制客户端架构：[`adr/139-control-client-architecture.md`](adr/139-control-client-architecture.md)
- 当前执行待办：[`../.codex-memory/todo.md`](../.codex-memory/todo.md)
- Tetherly Obsidian canonical：`Note/Project/tetherly/tetherly 开发文档.md`
- 文档治理：Obsidian `AI-DOC-ROUTER` → `routers/projects.md` → Tetherly canonical；长流水写 `00.MOC/生产变更速记.md`。

## 8. 变更与验证记录

- **2026-11-19 review baseline**：完成 v0.2.65 深度 review 汇总，确认 P1/P2 风险、批次顺序和 release gate；当时未改业务代码、未运行新的代码测试/构建、未触发 CI/release/deploy。
- **2026-11-19 remediation GREEN（本地代码与定向测试）**：
  - Batch 1：`data_dir/send` 私有 staging + canonical containment，single/multi/batch/outbox resend 统一校验；GET export 拒绝 `out`，POST snapshot/recording/whiteboard Admin-only，export 目录 real-private + `create_new` 文件句柄；legacy SSE 至少 Read，显式 v1 connector + EventEnvelope/`UiEvent::V1` facade；两套 CLI `--out` 改 POST。
  - Batch 2：raw inbox 改为已校验文件句柄 + 64 KiB 分块 HTTP 响应，定向下载回归覆盖 128 KiB 内容完整性和 `Content-Length`；file listener 共享 32 permit/header timeout；file connect/chunk/total deadline；`recv_inner`/rekey/发送路径统一 `SESSION_WRITE_TIMEOUT`。`cargo test -p tetherly-net --lib --locked` = **13 passed**；`cargo test -p tetherly-net --test backpressure --locked` = **3 passed**；node idle-header/send-path 回归通过。
  - Batch 3：screen generation/privacy/ticket failure cleanup 定向回归通过；camera detach peer+session ownership；单目标 offer/accept rollback；同源同元数据 duplicate admission；hard-link atomic inbox reservation。`cargo test -p tetherly-core --lib --locked` = **334 passed**；`cargo test -p tetherly-node --lib --locked` = **309 passed**；`cargo check -p tetherly-node --lib --tests --locked` 通过；screen failure cleanup、inbox reservation、send-path policy 定向测试通过。
  - Batch 4：clipboard preview/commit 分离 + per-peer bounded retry（3 attempts/8 seconds），clipboard delivery tests = **3 passed**、clipboard policy tests = **6 passed**；browser/Tauri SSE handshake/idle/reconnect/sequence 定向传输 = **14 passed**，Tauri Rust SSE = **10 passed**。Android quick-reply queue/foreground dataSync timeout/restart 状态机和纯 JVM 测试源码已落地，但未执行 Android 编译或 unit test。
  - Batch 5：CI frontend build gate、release tag/version prepare、Android `assembleRelease`/manifest/signature inspection、artifact checksum/immutable publish、Windows readiness/rollback、smoke/crossing/HARQ/TWCC v1 route/token migration 代码落地；workflow YAML（Ruby parser）、shell/Python syntax、`git diff --check` 通过；未触发 GitHub CI/release。
  - Existing fresh evidence retained：`cargo test -p tetherly-loopback --lib --locked` = **12 passed**；`cargo test -p tetherly-api --lib --locked` bindings + contract = **43 passed**；`cargo check -p tetherly-cli` 通过；目标 legacy-route grep 通过。
  - Residual：timeout/cancel/shutdown 后 inbound `.part`/resume sidecar 保留策略尚未做现场确认；Android Gradle/unit/NDK/真机、真实浏览器/WebView、三平台 GitHub CI、Windows ACL/junction/readiness/rollback、macOS artifact、release provenance 和生产部署均未验证。
- **文档治理**：本机固定 Obsidian `doc-lookup`/`doc-check` 路径不存在，未伪造治理通过；发布冻结继续有效。
- **2026-11-19 深度优化迭代（本地）**：
  - 前端门禁根因修复：`pnpm check` 的 13 个错误中 4 个源于 `tetherly-api` `render()` decl 列表漏登 `TransferStats`/`TrayOwner`/`DisplayModeDto`（生成的 api.ts 引用悬空），补 decl + `TETHERLY_API_BLESS=1` 重新 bless 后，临时 shim `api-contract-shims.d.ts` 已删除；其余 9 个为测试 fixture 缺新字段（`Device.group`、`System.lan_only`、`Transfers.stats`、`SettingsGeneral.tray_owner`），最小补齐。最终 `pnpm check` 0 errors / `pnpm test` 36 passed。
  - P3 storage/open 路径脱敏：storage 报告与 inbox open 报告按 tier 分级返回（Admin 真实路径，client token 相对占位）；`granted` tier 穿透两层 route。回归 `inbox_open_path_is_redacted_below_admin` + `v1_file_domains_serve_and_gate`。
  - P3 macOS token argv：`package-macos-app.sh` 两处 curl 改 stdin config。
  - Fresh tests：`cargo test -p tetherly-node --lib --locked` = **315 passed**；`cargo test -p tetherly-api --test contract --locked` 通过；bindings bless 后复验通过；`bash -n scripts/package-macos-app.sh` 通过。
- **2026-11-19/20 传输集成证据（本地）**：
  - Inbound stalled-transfer：真实 TCP peer 发送 TFL2 header + 1 个 64 KiB chunk 后停顿，测试构建注入 500ms chunk-idle 窗口（`files.rs` `#[cfg(test)]` 包装 + 惰性 `TEST_CHUNK_IDLE_MS`，生产构建整体编译消失，四个生产常量不变）；断言 Timeout、`transfer_cancels` 清理、active Failed、`.part` 与 `.resume.json` sidecar 按续传语义保留。RED（无注入 4.05s 等满 15s 生产窗口）→ GREEN 0.55s：`inbound_stall_tests::stalled_inbound_transfer_fails_and_keeps_resume_state`。
  - Multi-peer queue-full 隔离：一个 peer 的 256 深度 outbound 队列填满、另一 peer 正常，`offer_and_send_bytes_multi(TargetSelector::AllOnline)` 断言失败项 success=false、该 transfer Failed/pending 清理/outbox Failed、健康 peer 不受影响、计数正确。RED 用突变测试（临时移除 `set_transfer_phase` 调用 → 0.03s 捕获 `Transferring≠Failed`）→ GREEN：`multi_peer_file_dispatch_isolates_queue_full_failure_to_its_peer`。
  - Fresh tests：`cargo test -p tetherly-net --lib --locked` = **13 passed**；`cargo test -p tetherly-node --lib --locked` = **317 passed**；`cargo check -p tetherly-node --lib --tests --locked` 通过；rustfmt/`git diff --check` 通过。
- **2026-11-19/20 v0.2.66 发布与现场证据（全部完成）**：
  - CI：`985485b` 首轮暴露 frontend dist 契约与三平台 clippy；`903e4b5` 修 clippy+dist、windows 需修 `SUB_CONTAINERS_AND_OBJECTS_INHERIT` 导入路径（`217962e`）；loopback reject-mock 在 Windows 被 RST 掩盖（`9161d98` 补请求消费）、backpressure recv 测试对 runner 负载敏感（`95db182`→socket2 钉缓冲 `3e62b10`）。最终 CI run **44573908834 全绿**（三平台 check + android + frontend + deny）。
  - Release：bump 0.2.66（`b0aeb63`），tag 触发 release workflow；两轮修复（frontend pnpm 安装顺序 `8dc3c28`；apkanalyzer/apksigner 经 ANDROID_HOME 定位 `f56f524`；publish 剔除 frontend 辅助文件 `2bb5a50`）；Android release 签名密钥生成并写入 4 个 repo secrets（keystore 落 vault `Note/Accounts/ai/tetherly-release-signing/`）。最终 release run **44585068559 全绿**，release v0.2.66 发布 12 资产含 SHA256SUMS，APK 为 release 签名（debug 证书拒绝检查通过）。
  - Deploy：macOS 经 package-macos-app.sh（stdin-config curl 关停旧 daemon）health 0.2.66 ✓；Windows 部署修复两处现场缺陷（all-target digest 重复下载 `35a1f47`；PowerShell CRLF 破坏 SHA 锚点校验 `8dffd72`），热重启 old_pid 19764→new_pid 6104，readiness PID/version/start-time 校验通过，SYSTEM 权限确认，受保护进程 3/3 不动。
  - Smoke：三轮迭代（session 断言改为 SYSTEM owner 探针；remote_v1 强制 UTF-8 控制台编码——隔离实验证明产品链路完整传输 emoji，`??` 为脚本控制台编码；auto-accept 允许 null size；CIM 引号层级对齐 deploy.sh）。最终 **19/19 PASS（rc=0）**。
  - 文档治理：Obsidian `doc-check` PASS 4/4；canonical 速读基线更新至 v0.2.66 双端部署。
  - Manual-required 残余（不阻塞本发布）：Android 15 真机 dataSync timeout/长稳、timeout/shutdown 后 partial/sidecar 策略长期现场观察、Windows smooth-wheel detent 产品承诺确认。
- **发布结论**：v0.2.66 已发布并双端部署，P1/P2 修复全量上线；后续回归按常规批次记录，不再适用本路线图的发布冻结（该冻结随本次发布解除）。
- **2026-11-20 §175 hook 并发启动治理（本地定向 GREEN，随下个版本发布）**：
  - 范围：§5 Needs-confirmation 第 2 条（hook 并发启动 / 生命周期）。取证：`ensure_input_capture` check-then-act 竞态（标志在 `start_input_hooks` 就绪阻塞返回后才置位）；macOS 二次启动无守卫 → 孤儿 CGEventTap；Windows 二次启动拒绝前已有 `system_cursor::restore(true)` 副作用。
  - 修复：`runtime.rs` Shared 增 `input_capture_spawning` 原子占坑；`ensure_input_capture`（input.rs）经 `try_claim_capture_spawn` 单飞 admit，spawn 完成后 `release_capture_spawn_claim`；争用方返回 Ok(true)（armed-pending，等下个 tick 复查）。`platform.rs` macOS 增进程级 `HOOKS_STARTED` 守卫（拒绝 "input hooks already started"；Accessibility 未授权路径先释放占坑）。Windows 侧已有 SENDER OnceLock，不重复设防。Shutdown 定级 confirmed design（tap 进程生命周期，注释已锚定）。
  - RED/GREEN：新增 `capture_spawn_claim_admits_exactly_one_contenting_caller`（8 线程并发 claim 恰 1 胜者；释放后可再占）。`cargo test -p tetherly-node --lib --locked` = **318 passed**（v0.2.68 基线 317 + 1）；`cargo clippy -p tetherly-node --lib --tests --locked` 0 lint；rustfmt / `git diff --check` 通过。
  - 残余：随下个 tag 走 CI 三平台 + Android 门禁后发布；无新增 manual-required 项。
- **2026-11-20 §176 smooth-wheel 子 detent 治理（本地定向 GREEN，随下个版本发布）**：
  - 范围：§5 Needs-confirmation 第 3 条。用户拍板（2026-11-20）：平滑滚轮属产品承诺，按 P2 correctness 修复。
  - 取证：Windows hook `WM_MOUSEWHEEL`/`HWHEEL` 路径每事件 `raw / 120` 独立整除——高分辨率滚轮单事件 |raw|<120 的增量被整体丢弃且无跨事件累积；注释自称「sign-preserving round-down」与实际行为不符（丢弃即非保号语义，是静默丢失）。macOS tap 直发 CGEvent 行 delta 不受影响；wire 协议（i16 行单位）与目标端注入（×120 re-multiply / CG Line 单位）不变。
  - 修复：`platform.rs` 新增 `WHEEL_DELTA` 常量 + `wheel_accumulate` 纯函数（saturating 累积、truncated `/`/`%` 保号余数、±32 detent 钳制）+ `hook_wheel_state` thread-local 双轴余数模块；Windows hook 两处 wheel 路径改走累积器，0 行事件不推送。初版用 `div_euclid` 在负向立即溢出 -1（RED 断言捕获），改 truncated 除法后负向余数保持待发方向。
  - RED/GREEN：`wheel_accumulator_emits_detents_across_sub_delta_events` 覆盖子 detent 跨事件凑格（8×15）、整格直通、负向保号、正负互抵不虚构、100k 敌意小增量饱和。`cargo test -p tetherly-node --lib --locked` = **319 passed**（318+1）；clippy 0 lint；rustfmt / `git diff --check` 通过。
  - 残余：随下个 tag 走 CI（Windows 真机滚轮手感为 Manual-required 观察项）；§5 Needs-confirmation 三项全部销项。
- **2026-11-20 §177 前端/Tauri 层深审 + activity 通知风暴治理（本地定向 GREEN，随下个版本发布）**：
  - 范围：`client/src-tauri/src`（events/proxy/token/tray/trayui/trayowner/lib）与 `client/src/lib`（transport/stores/App.svelte）全量通读。
  - 审查结论（不修，确认为设计）：Tauri SSE 握手校验/idle 超时/last_event_id 续传/seq 去重完整（8 测试锚定）；http.ts SSE idle watchdog（reader.cancel）与 handshake timeout 正确；proxy read-to-EOF 有 daemon Content-Length+close 契约锚定；token 不进 WebView 边界干净；lib.rs ExitRequested 的同步 block_on 为退出一次性路径（2s 超时兜底）。
  - 缺陷（confirmed）：`activity.changed` 通知风暴。daemon 在任一 Activity 主题事件（消息收发/handoff 更新/candidate 过期）上都整段重发全量 Activity 资源（hub.rs `Topic::Activity → ActivityChanged{activity}`），而 tray.rs `notice_for` 对每帧无状态重算 unread——用户不读消息就每次相关变更都弹同一条系统通知，handoff 未打开同理。
  - 修复：`tray.rs` 新增 `NoticeSeen` 身份去重（HashSet：`msg:<id>` / `hand:<id>`，id 缺失时回退内容身份）；`notice_for` 增 `&mut NoticeSeen` 参数，只有**从未出现过**的 unread/un-opened 身份才计入通知正文（正文计数改为 fresh 数）；`lib.rs` SSE 转发 listener 持有跨帧 `Mutex<NoticeSeen>`。
  - RED/GREEN：新增 `activity_republish_does_not_rebuzz_known_items`（同身份重发静默、新消息只报新增数、handoff 打开后新 handoff 报新增）；旧 `unread_activity_notifies_only_when_new` 迁移到新签名。`cargo test -p tetherly-shell --lib --locked` = **33 passed**（31+2）；clippy 0 lint；`pnpm test` 36 passed；`pnpm check` 0 errors。
  - 残余：随下个 tag 走 CI（frontend/Tauri job + 三平台）。
- **2026-11-20 §178 node 支撑模块深审（纯审查，零缺陷，全部 confirmed-design）**：
  - 范围：`crates/tetherly-node/src` 支撑层 7 模块全量通读——`cred.rs`（520 行）、`apitoken.rs`（641 行）、`telemetry.rs`（307 行）、`sidecar.rs`（263 行）、`logsink.rs`（261 行）、`lan.rs`（223 行）、`ancs.rs`（88 行）。
  - 审查结论（不修，确认为设计）：
    - `cred.rs`：vault + identity.bin 双存储互为权威，缺侧幂等修复；macOS 默认文件存储（TETHERLY_VAULT=1 才进 keychain）规避弹窗；Windows SYSTEM 上下文不进 vault——均有测试锚定。
    - `apitoken.rs`：tier 数学闭环（client token Write < Admin 路径门槛）；`V1_ADMIN_PATHS.len()==8` pin 测试防新路由静默降级；`token_matches` 空 expected 永不匹配（fail closed）；api.token 每次启动轮换 + `CREATE_NEW` + protected DACL（无继承窗口）；`read_client` Unix 权限宽于 0600 即拒绝。
    - `telemetry.rs`：纯 AtomicU64 计数器，零负载红线在位；`snapshot()` 通道求和 saturating；文件通道按设计报 tx/rx 0。
    - `logsink.rs`：rename 失败原地截断兜底，硬上限 2×cap 不可无界增长；blocked-rename 场景有测试；Windows 追加句柄缺 FILE_WRITE_DATA 的 set_len 陷阱经独立写句柄绕开（注释+测试钉死）。
    - `lan.rs`：诊断报告与 binder 共用同一 `classify`（不可漂移，测试断言 report==bind set）；5s 缓存防 UI 轮询 syscall 风暴；`bind_many_optional` 拒绝 0.0.0.0 与非 LAN 地址。
    - `sidecar.rs`：EasyTier RPC/CLI 双发现路径均带 probe 超时，无阻塞探测。
    - `ancs.rs`：纯装配层，三平台 stack 编译验证；真机 iPhone 配对如实标注 Manual-required（无虚假测试声明）。
  - 定级：本批无 RED/GREEN（零代码变更）；node lib 测试基线维持 319。
  - 残余：无新增 manual-required。§175/§176/§177 三批随 v0.2.69 发布。
- 后续每个批次必须在此追加：范围、RED/GREEN、CI run、release artifact、部署 health、回滚证据和 residual/manual 项。
- **2026-11-20 §179 net 层深审（filechan/session/codec/dispatch/mdns，一处 P3 修复）**：
  - 范围：`crates/tetherly-net/src` 全 5 模块通读——`filechan.rs`（869 行）、`session.rs`（750 行）、`dispatch.rs`（269 行）、`codec.rs`（133 行）、`mdns.rs`（125 行）。
  - 审查结论（不修，确认为设计）：
    - `session.rs`：Batch 2d `flush_outbox_bounded` 覆盖 send/recv/rekey 全路径；rekey 取消安全（m1 先入 outbox、`rekey_state` 停车跨 await 恢复、responder m2+换 transport 同步原子步）；pairing fail_lock / trust 在 Noise 成功后才提交 / Noise 失败回滚 trust 全闭环；`TaggedReader` 会话侧 outbox 进度跨 cancel 保留。
    - `codec.rs`：`TaggedReader` 取消安全（header/body 部分读状态跨 select! cancel 保留）；MAX_OUTER 在 body 分配前拒绝（敌意长度不预分配）。
    - `filechan.rs`：Batch 2b/2c 上界全在位（HEADER 3s / CHUNK 15s / TOTAL 30min，均为显式 deadline）；AAD `id||chan_offset` 绑定通道位置防跨 resume 重放；`total > max_size` 在 header 即拒绝（不读不缓冲）；截断流报错而非静默短文件；`recv_chunks_to_sink` O(1) 内存 + 增量 sha256。
    - `dispatch.rs`：纯 from_payload 映射表，Unknown 透传不炸。
    - `mdns.rs`：instance 精确匹配防前缀误删（有测试）；LAN 地址过滤复用 `should_listen`。
  - 缺陷（P3，confirmed，已修）：`tetherly_core::FILE_CHUNK`（frame.rs）与 `tetherly_net::filechan::CHUNK` 是两个独立定义的 64×1024 常量——节点 resume 用前者算 `.part` 对齐，wire 层用后者算 nonce counter 与 AAD 位置，属隐式耦合。漂移不会写坏数据（receiver `start % CHUNK != 0` fail-closed 拒绝），但会造成对等方互相 Handshake 拒绝的难排查现场。修复：filechan.rs 编译期 `const _: () = assert!(CHUNK == tetherly_core::FILE_CHUNK)` + 注释锚定（零运行时开销）。
  - 验证：net crate 28 tests 全绿（13+3+7+4+1）；clippy 0 error；rustfmt / `git diff --check` 过；core 334 passed 回归不变。
  - 残余：无新增 manual-required。
- **2026-11-21 §180 core/input.rs 深审（一处 P2 修复：on_remote_leave band 内泊位）**：
  - 范围：`crates/tetherly-core/src/input.rs` 全 2886 行通读（协议常量、InputServer/InputClient 状态机、band gate、far-push 回落、30+ 测试）+ 调用方取证（node `runtime/input.rs` Leave handler、`platform.rs` warp-echo gate）。
  - 缺陷（P2，confirmed，已修）：`on_remote_leave`（target 发 Leave、controller 泵收回本地座位的唯一引擎回落路径）Top/Bottom 泊位在 y=1 / height-1——默认 24px 冗余 band 内部且在 `y<=8` into-edge 区；而 Leave handler 只 `disarm_input_capture`（仅键盘 hook），鼠标 hook 继续喂 `drive_input_delta`。泊位处任何 stray dy≤0 事件 arm 慢速 dwell，静默 120ms 后下一个 stray 事件即 re-cross——与 v0.2.26 ghost-controller 生产事故同机制。`force_local` 与 far-push 回落早已用 `band_safe_dock_y()` 泊到 band 外，唯独此路径遗漏。修复：Top/Bottom 改用 `band_safe_dock_y()` 泊位（与 force_local 完全一致），x 轴用 `denorm_x(rem_x)` 保持跨屏位置映射；Left/Right 泊位不动（水平边无 band gate）。
  - 取证（确认现场可达）：warp-echo gate 有 250ms 时限与 2 次回声配额，错过 gate 的回声/settle 事件正是 v0.2.26 生产日志记录的 stray 来源（`hook_delta_is_bogus` 只挡 >半屏 位移）；band 配置默认 24px 每事件镜像进引擎；既有回归测试 `tb_band_dock_never_recrosses_from_a_stray_quiet_event` 只覆盖 force_local 泊位。
  - RED：新测试 `on_remote_leave_dock_never_recrosses_from_a_stray_quiet_event`（Top/Bottom 双向：泊位必须 band 外 + 泊位处 stray 静默事件不得 re-cross）——修复前 y=1 断言失败。GREEN：core 335 passed（+1）；node lib 319 passed 回归不变；clippy 双形态（lib / lib+tests）0 error；rustfmt 过。
  - 残余：无新增 manual-required。
- **2026-11-21 §181 core/topology.rs 深审（两处 confirmed 修复：sanitize panic P2 + 正交映射剪切 P3）**：
  - 范围：`crates/tetherly-core/src/topology.rs` 全 750 行通读（DisplayEdge/DisplayRect/EdgeCrossingConfig/Validator/map_edge_exit_to_entry）+ wire 取证（dispatch.rs 反序列化 → lifecycle.rs:1885 → receive_topology_state:494 `frame.sanitize()`；node input.rs:3989 反向 takeover entry 映射）。
  - 缺陷 1（P2，confirmed，已修）：`TopologyStateFrame::sanitize` 对 `device_id`/`neighbor.peer_id` 直接 `String::truncate(64)`——已认证 peer 发送一个多字节 UTF-8 字符跨字节 64 边界的 frame 即触发 `is_char_boundary` assertion panic，session task 崩溃（DoS）。frame.rs 既有 `truncate_utf8`（边界回退）是全库规范写法，此处遗漏。修复：`truncate_utf8` 改 `pub(crate)` 复用；RED 测试 `sanitize_truncates_multi_byte_ids_without_panic` 修复前在生产行 panic 复现。
  - 缺陷 2（P3，confirmed，已修）：`map_edge_exit_to_entry` 主分支用 `scale()` round-to-nearest 且注释明确警告截断剪切，但 8 个正交/同边 fallback 分支仍用 `(frac * dim) as i32` 截断——正交拓扑每次往返剪切 1px（f32 精确验证：Top→Right exit_x=1918 时 trunc 2157 / round 2158）。修复：fallback 全部改 `scale()`；RED 测试 `orthogonal_entry_uses_round_to_nearest` 修复前失败。
  - 其余 confirmed-design：Validator 边界完备（band/dwell/fast-push/grace/speed 全 clamp）；`offset.clamp(±8192)` 无溢出；`is_in_corner_zone` 语义正确；serde legacy 兼容默认值有 pin 测试。
  - 验证：core 337（+2）/ node 319 / net 13 回归全绿；clippy 双形态 0 error；fmt 过。
  - 残余：无新增 manual-required。
- **2026-11-21 §182 core/frame.rs 深审（纯审查，零缺陷修复，全部 confirmed-design）**：
  - 范围：`crates/tetherly-core/src/frame.rs` 全 710 行 + 下游取证（clip.rs `apply_remote`、filexfer.rs `offer_resumable`、notify.rs `ingest_push`、dedup.rs limiter/deduper、node notify/files/lifecycle 接收路径）。
  - 审查结论（不修，确认为设计）：
    - `INNER_PAYLOAD_MAX`（65535-16-12）是唯一预算源，crypto `tests/limits.rs` 钉死；`decode` 在 payload 分配后但加密前校验上限；MUST_UNDERSTAND fail-closed 且覆盖 input/screen 两命名空间；未知类型不带旗标保留（前向兼容）。
    - `truncate_utf8` 字节边界回退是全库 String 截断规范（§181 已让 topology 复用）；`NotifyPush::truncate` cap title/body 各 4KiB + 剥离 URL actions，多字节 pin 测试在位。
    - wire 层无 cap 的结构下游防线完备：`ClipSet` → `apply_remote`（CLIP_TEXT_MAX + sha256 绑定 + 4/s limiter）；`FileOffer` → `offer_resumable`（transfer_id 字符集 + 每文件 FILE_MAX 64GiB + sha256 格式 + INCOMING_MAX=256 表上限）；`TextMessage`/`ResourceHandoffFrame` from_payload 自带 cap。
  - P4 观察项（不行动，登记备查）：`NotifyPush` 的 `uid`/`app_id`/`app_name` 无长度 cap，且 notify hub 的 `evict_expired` 仅在 UI 拉 activity/candidates 时触发（无周期任务）。Rate limiter（2/s 持续 + burst 10，per source）与 120s candidate TTL / 10min dedup 窗口约束下增长有界；正常 peer 字段为真实通知标识符（几十字节），仅理论最坏受 64KiB 帧上限。
- **2026-11-21 §183 core/clip.rs 深审（一处 P3 修复：AWS key 扫描字节切片 panic）**：
  - 范围：`crates/tetherly-core/src/clip.rs` 全 668 行（ClipHub 状态机、ClipSyncMode、SensitiveContentGuard、历史环）+ 可达性取证（node clipboard.rs:754 inspect 调用点在 poll_clipboard_auto_sync 自动同步路径）。
  - 缺陷（P3，confirmed，已修）：`SensitiveContentGuard::is_aws_key_at` 用裸字节切片 `&remainder[..20]` / `&key_candidate[4..20]` 提取 AWS key 窗口——多字节 UTF-8 字符跨字节 20 边界时 panic（独立 rustc 程序证明）。可达路径非 wire 而是本地系统剪贴板：复制一段含 "AKIA"/"ASIA" + 中文/emoji 的普通文本（日志片段、OCR 输出），周期性 auto-sync 轮询即 panic 崩溃剪贴板同步任务。修复：按字符边界回退取窗口（`is_char_boundary` 循环），短窗口只可能 under-match 不可能误检；20 字节 ASCII 真 key 检测不变（pin 断言）。
  - 其余 confirmed-design：preview/commit 分离（全 peer 入队才提交 hash/seq）；echo 防环 hash 相等即 Echo；apply_remote 检查顺序 limiter→echo→text→size→hash 绑定；seq 单调 max 合并；MAX_CLIP_HISTORY=32 环形有界；其余 guard 检测（PEM/GitHub/OpenAI/Slack/JWT/密码管理器）全部 contains/split 无切片风险。
  - 验证：core 338（+1）/ node 319 回归全绿；clippy 双形态 0 error；fmt 过。
- **2026-11-21 §200 core/ 交互与 AV 控制面深审（drag/action/camera/media，一处 P3 修复：MediaCommandFrame 解码直通）**：
  - 范围：`crates/tetherly-core/src/drag.rs`（499 行，漫游拖拽 FSM + JSON 控制面）、`action.rs`（474 行，动作广告/触发/结果四帧 + ActionValidator）、`camera.rs`（533 行，虚拟相机 announce/attach/detach/chunk + CameraValidator）、`media.rs`（477 行，跨设备媒体控制/状态）全读 + 下游取证（dispatch.rs:135/219/228/262 解码点、node runtime/media.rs:447/630 ingest、runtime/drag.rs:50 拖拽路由、runtime/camera.rs:96/244 相机 ingest、events.rs:916 UiEvent 构造、uihttp.rs:679 legacy SSE 广播）。
  - 缺陷（P3，confirmed，已修）：`MediaCommandFrame::from_payload` 为裸 `serde_json::from_slice` 直通——无 sanitize、无字段 cap，与同文件 `MediaStateFrame::from_payload`（解码即 sanitize）、`TextMessage::from_payload`（TEXT_MESSAGE_MAX cap）、`DragControlMsg::from_payload`（1KiB 帧预算）纪律不一致。攻击链：dispatch.rs:135 解码任意已连接对端命令帧 → node `receive_media_command` 有速率限制/策略门但**不消毒** `frame.id`/`frame.target_player` → `UiEvent::media_command_received`（runtime/media.rs:591）→ legacy `/api/events` SSE 原样广播（uihttp.rs:679 `serde_json::to_string(&ev)`）。恶意对端可注入 256KB 级（MAX_OUTER 预算内）含控制字符的 id/target_player 污染本地 UI 事件流与 CLI watch 输出。修复：`from_payload` 解码即 `sanitize()`（id/target_player 走 `MediaValidator::sanitize_text` 剥控制字符 + MAX_MEDIA_PLAYER_ID_LEN cap），新增回归测试 `test_media_command_frame_decode_sanitizes_hostile_fields`（控制字符剥离 + 超长 cap + 合法帧不变三断言）。RED 证据：git HEAD 原文 `let frame: Self = serde_json::from_slice(payload)?; Ok(frame)` 直通在案。
  - 关键正面论证（drag）：1KiB 双向帧预算（encode/decode 都查）；坐标全 clamp [0,1]（`normalize_coord` 除零用 max(1)；denormalize 防御性再 clamp）；FSM 单活动会话、move/drop-confirm 严格 drag_id + phase 匹配、10s 超时 prune、cancel 即清；node 侧 `pending_drag_drops` 30s TTL purge 防孤儿 claim，MOTW 归属走 peer_alias 白名单化标签（runtime/drag.rs 模块头注明 shell 层消毒）。
  - 关键正面论证（action）：ADR-008 不变量——命令字符串/参数仅本地持有不上 wire，wire 只带 id/name/desc/icon；`from_payload` 全部解码即 sanitize；`validate_action_id` 白名单 `[a-zA-Z0-9_-]` fail-closed；`sanitize_text` 剥 is_control + UTF-8 边界截断；output_summary 专用通道（保留 \n\r\t，1024 cap）。
  - 关键正面论证（camera）：`validate_attach` 长度/字符集/分辨率 320..7680×240..4320/fps 1..60/zoom NaN 防护 0.5..20 全链；`validate_chunk` total_chunks≥1 + chunk_seq<total + MAX_WIRE_CHUNK_BYTES cap；node ingest `validate_chunk_frame` 前置 + session_id/peer 双匹配（§188 ownership 模型一致）；attach 入口 frame.sanitize + validate 双道、拒绝路径回发 detach + 审计。
  - 关键正面论证（media）：SetVolume 服务端 policy.clamp_volume 二次钳制；seek 策略门独立；§190 media_limiter 速率限制在位（command/state 双路径）；`MediaValidator::sanitize_text` 逐字符剥 is_control + is_char_boundary 回退截断。
  - 验证：core 342（+1）/ lib 全绿；clippy 双形态 0 error；fmt 过。
- **2026-11-21 §207 node 非 runtime 模块深审（小模块面/store/platform/tray/screen/ancs/apiv1 全家/model/uihttp 全家/viewer，~15.8k 行，发现并修复 2 处 P3 + 2 处 P4 观察）**：
  - 范围：小模块面（clipboard/notify/profiles/presets/updates/lan_ip/file_links/network 等 ≈6.4k 行）、`store.rs`（4615）、`platform.rs`（4704，P3-1 applescript 转义已修 76c28fb）、`tray.rs`（927）+ `cursor_shield.rs`（228）、`screen.rs`（2379）、`screen/{macos 353, x11 546}`、`ancs/{macos 623, bluez 537, win_gatt 518}`、`apiv1/{coalesce 112, journal 161, legacy 155, hub 405, project 297, apiv1.rs 13}`、`model.rs`（4078，P3-2b expect 已修）、`uihttp.rs`（2060）、`uihttp/{v1.rs 7484（P3-2a 已修）, next.rs 107, util.rs 75, dto.rs 46}`、`bin/tetherly_viewer.rs`（286）全读。至此 tetherly-node crate 全模块深审收官。
  - P3-2a（confirmed，已修 9899e50）：`POST /screen/whiteboard/import` 的 `file` 分支 `std::fs::read(&file)` 先执行、`..` 校验后置，且绝对路径从不拒绝——客户端 JSON 直供路径可先触碰任意可读文件（读后丢弃，泄露面窄但校验顺序纪律违反）。修复：校验前置（`contains("..")` + `is_absolute()` 双拒）后才读盘，新增回归测试 `v1_whiteboard_import_rejects_bad_paths_before_read`（/etc/passwd、../、sub/../../ 三路径 400 "path traversal rejected"，读前拒绝）。
  - P3-2b（confirmed，已修 9899e50）：`model.rs` 4 处 `.expect("validated ... policy")`——前提错误：runtime `set_*_policy` 经 `policy.sanitize()`（仅 clamp，从不 panic）后仍可因 `save_action_policy` 持久化失败（只读 data dir/磁盘满）返回 `CoreError::File`，旧代码使 HTTP handler task panic（连接静默断开、无响应）。修复：4 个函数签名改 `Result<Dto, CoreError>`，v1.rs 4 个 PATCH 调用点 `.map_err(core_error)?`——真故障类正确映射 500 Internal，全库 grep 确认无其它调用方。
  - 关键正面论证：①uihttp.rs loopback-only bind+accept + MAX_HEADER 16KB/MAX_BODY 2MB + 10s 读写超时 + fail-closed token + Host 精确匹配 + Origin/Sec-Fetch-Site CSRF 双门（loopback origin 必须 = ui_port，无 Origin 的跨站仍被 Sec-Fetch-Site 拒）+ 408 stalled-request + drain_request_body 先于拒绝；②v1.rs `allowed(path)` 静态方法表驱动 404/405+Allow，parse_body_strict 非 Default 全字段，core_error 客户端错映射 legacy 类（File("not found")→404、client-state→409、FileTooLarge→413）、真故障→500；create_screen_export 单文件名 + canonicalize containment + create_new + 0o600；storage 绝对路径 Admin-only + 占位符防泄漏；SSE PING 5s 喂 10s TRAY_TAKEOVER_RECLAIM_MS 窗口（§147 测试钉死）；③store.rs sanitize 纪律（ActionPolicy::sanitize 仅 clamp_timeout，白板 policy 全 clamp）+ 原子写；④platform.rs applescript 输出经命令层转义（P3-1 后）；⑤tetherly_viewer.rs stdin 12 字节 BE 头 + 8MiB 预算 + `len != w*h*4` 几何校验 + 损坏即 break，AppKit 窗口 EOF 后 orderOut；⑥next.rs 固定名 allowlist 404、util.rs percent_decode 越界保字面量。
  - P4 观察项（不行动，登记备查）：①`screen.rs` AdbSource raw 模式按不可信设备头 `w*h*4` 分配（u16 理论 ~17GB），对照 PNG 64MB cap——接受：TETHERLY_ADB_RAW=1 显式 opt-in 且 adb 设备属管理员信任域；②model.rs 4 处 expect 原为 P4 观察项，本轮升级确认 P3-2b 并已修复。
  - 验证：clippy 双形态（--lib / --all-targets，-D warnings）0 error；`uihttp` 25 + `apiv1::model` 4 + 新增回归 1 全绿；fmt 过；CI 76c28fb=44874306386 success、9899e50=44877084905 failure（windows 上回归测试的 "/etc/passwd" 不带盘符不算绝对路径 → 改 temp_dir 平台无关绝对路径）、6c81f02=44878772315 rerun success（首跑 windows phase3 m3_5 session-register 时序 flake，与改动无关，重跑绿——同 m3_2 家族）。
- **2026-11-21 §206 node runtime 收尾深审（lifecycle/discovery/audio/routes/topology/locate/focus/remote_session/drag 三件/shell_reveal/overlay，~10k 行，纯审查，零代码缺陷，无代码变更）**：
  - 范围：`runtime/lifecycle.rs`（3001 行 run/listeners/shutdown/attach/handle_event 全事件分发/lan promotion/restart_daemon + 测试）、`discovery.rs`（1567 行 mDNS advertise/browse/解析）、`audio.rs`（1399 行 audio routing + mic 会话）、`routes.rs`（708 行路由收敛）、`topology.rs`（562 行）、`locate.rs`（571 行）、`remote_session.rs`（469 行）、`focus.rs`（466 行）、`drag.rs`（477 行）+ `drag_controller.rs`（386 行）+ `drag_target.rs`（303 行）、`shell_reveal.rs`（231 行）、`overlay.rs`（132 行）全读。node runtime/ 层 35 模块深审收官（§204 会话面 + §205 屏幕面 + §206 收尾三批）。
  - 关键正面论证：①`lifecycle.rs` attach/release 单会话不变量（ownership = channel 身份而非 peer 地址，register/release 持同一 `sessions` 锁双写两 map；keepalive 仅持 weak sender——强克隆即僵尸会话，测试钉死）；peer down teardown 顺序完整：reconnect wake → ping_waiters → input_seat → pending_drag_drops 按 peer 清扫 + active drag_id 归属复核才 reset → PeerDown，stale 拖拽注册不会错误认领后续 FileOffer（MOTW/reveal 误施加防护）；②`restart_daemon` 单飞门（RESTART_IN_PROGRESS CAS）+ 替换二进制强制位于 update staging 目录（canonicalize + starts_with，防 loopback 调用方指任意可执行）+ Windows rename 链带失败回滚 + `--help` preflight（坏构建在旧 daemon 存活时暴露）+ 退出窗口 clamp ≤1000ms 对齐新进程 bind 重试预算；③lan promotion：`lanips` 是 peer 自报，只有握手证明的身份算数（identity mismatch 即弃）+ blocked peer 双重检查 + 连接延迟确定性抖动（lan_promotion_delay）+ try_begin 单飞 + 200ms 探测超时 + promotion 握手用 RECONNECT_CONNECT_TIMEOUT 而非 20s session 超时（生产事故回归测试在位）；④file listener admission 全局信号量（FILE_HANDLER_LIMIT）跨 LAN/overlay 监听共享，shutdown drain 全部 cancel sender 并释放全部 permit（测试断言 permits 归零）；⑤`attach` LAN 优先：已有 LAN 会话时 overlay attach 直接丢弃不覆盖；superseded 会话宽限期后关闭而非立即（对端先挂上替代会话）；⑥`handle_event` 全事件路由表 + telemetry 记字节数（明文不入遥测）；FileReject 清 pending_out 缓冲体（防拒绝后整文件常驻内存）+ transfer_cancels + forget + Failed 相位全链；FileOffer ref 被 hubs.files.offer_resumable 拒绝属 peer 协议错误，log-and-drop 不杀会话；auto-accept 双门（policy.enabled + is_trusted + profile eligibility）；⑦`discovery.rs` mDNS 解析对 TXT 记录逐字段校验长度/字符集，anti-spoof 以握手身份为准；⑧`locate.rs` 10s per-peer anti-harassment 冷却（§206 前批已证）；⑨`audio.rs` mic chunk Zero-Plaintext（元数据 only）+ 隐私指示灯 fail-closed；⑩drag 三件套 wire roundtrip/settlement 回归/superseded-cancel/confirm-then-abort retryable 测试完备；⑪`remote_session.rs` 单槽不变量 + optimistic 注册，m6_7 生产竞态注释在位。
  - P4 观察项（不行动，登记备查）：①`restart_daemon_inner` Unix 分支 `0o755` 写死权限——若原二进制为更收紧权限（组/其他无执行）会被放宽，实际部署形态（单用户 daemon）下无风险；②`shutdown()` 中 `sessions.try_lock()` 失败即跳过清理（锁竞争窗口），但 cleanup 语义由 cancel sender drain + transfer Failed 转换兜底，不留假活；③`handle_event` 庞大 match 路由遥测字节数为估算常量而非实测 wire 长度（telemetry 精度权衡，非缺陷）。
  - 验证：纯审查批次，零代码变更，无 CI 门禁需求；node lib 325 基线未动。
- **2026-11-21 §205 node runtime 屏幕面深审（screen_stream/screen_control/screen_mode/screen_media/screen_status/screen_dto，~11k 行，发现并修复 1 处 P3）**：
  - 范围：`runtime/screen_dto.rs`（793 行 ~200 字段遥测 DTO/有界 interval tracker/p95 ceil 索引）、`screen_status.rs`（882 行纯采样 report，checked_div 防除零，无 wire decode 面）、`screen_control.rs`（1615 行 allow/revoke 唯一同意入口 + start 奇偶票据协议 + stop 隐私 fail-closed）、`screen_media.rs`（1002 行录制/回放/快照/白板/激光笔）、`screen_mode.rs`（1811 行纯配置面 ~20 个 setter 全部 clamp + sync 回发）、`screen_stream.rs`（4979 行 attach 双端 pump 编解码全管线 + send_frame 编码分发 + FEC/pacing/背压整帧丢弃）全读。
  - 关键正面论证：①generation-owned teardown（registration lock 内 claim + still_active/still_newest_client 复核）贯穿 server/client 两端 pump，teardown 才清 session-scoped 隐私/水印/录制态；②viewer 端 ordering gate `sf.seq <= last_seq` 先于一切解码丢弃 stale，防画布回卷；所有 tile 解码失败路径 → KeyframeRequest 或 TileNackRequest（degraded commit 保 back-buffer）；③send_frame 分发层次清晰（tile-cache/batch/grid → LTR recovery → PIR → SVC-T1 → delta → raw），每个分支 dirty/saved_bytes 遥测完备，LTR/SVC T0 anchor 更新严格；④背压 `tx.capacity() < frags.len()` 整帧丢弃 + 不投半帧（assembler 不会 mid-frame 悬挂）+ force_idr + 4D 适配；⑤start 票据 odd=open window/refusal latch 仅 odd 期间锁存、stop 先本地隐私收敛（force even + 清 latch）再 send_result——send 失败不泄露隐私态；⑥PIR band 轮转 + OnDemand sweep 完成即停；水印 disable 被 host_enforced 拦截；快照在 privacy_active 时 bytes.fill(0) 先于水印合成（隐私优先）。
  - 修复（P3，commit 11525b7）：`screen_recording_replay`（screen_media.rs:153）是三个按调用方路径打开 TREC 文件的入口中唯一缺 `file_name().is_none() || ParentDir → UnsafeFileName` 守卫的（`screen_playback_start`/`screen_recording_snapshot_data` 均有），经 `GET /api/v1/screen/recording/replay?path=` Read tier 可达——错误信息差异构成文件存在性探测面且绕过 Batch 1 受控读取策略精神。修复：镜像兄弟守卫 + 相对路径 data_dir join + report 返回 resolved 路径。测试 `uihttp.rs::v1_screen_recording_replay_rejects_traversal`（三种 traversal 断言 unsafe file name + 正常相对路径断言 open recording 失败）；RED（stash 守卫后测试失败）→ GREEN → clippy 双形态 0 error → CI（run 44869937409）。
  - P4 观察项（不行动，登记备查）：①screen_stream.rs tile 编码三分支（cache/grid）存在大段近似重复的 palette/predict/raw fallback 链（~700 行×2），功能正确但可抽 helper 缩 ~40%；②send_screen_frames 中 batch chunk 睡眠 pacing 逐 chunk await，pacing_interval_us 大时拖慢 pump——现状默认值下无碍。
  - 验证：node lib 325 全绿（+1）；clippy 双形态 0 error；fmt 过；CI 门禁 run 44869937409。
- **2026-11-21 §204 node runtime 会话面深审（trust/clipboard/notify/actions/messages/audit/session/config，~5.5k 行，纯审查，零代码缺陷，无代码变更）**：
  - 范围：`runtime/trust.rs`（907 行 PIN 生命周期/resolve_peer/admit_channel/block_peer）、`clipboard.rs`（1085 行有界投递状态机 + 自动同步轮询）、`notify.rs`（995 行通知横幅抑制链 + ANCS 泵）、`actions.rs`（902 行远程动作执行）、`messages.rs`（869 行点对点消息 + 发件箱）、`audit.rs`（298 行审计环形缓冲）、`session.rs`（67 行会话 accept/send）、`config.rs`（266 行节点名/tray 保活）全读。节点 runtime/ 层 35 模块深审第一批收官。
  - 关键正面论证：①`trust.rs` admit_channel 对 side channel（文件/媒体等）执行 blocklist + expected-device 双门——被 block 设备仍处信任态，side channel 必须独立拦截（注释明示）；`block_peer` reason ≤128 字节 + 控制字符拒绝；resolve_peer 精确 DeviceId → 精确别名/名 → 子串三段式，歧义即报错列出候选（不静默择一）；forget/disconnect 全清 session/live/input_seat/screen_peer/input_peer 并发 PeerDown，无状态残留。②`clipboard.rs` 有界投递状态机（3 次尝试 / 800+2400ms 重试 / 8s max age），仅全部 peer 接受后 commit_local hash——测试钉死 `delivery_retries_only_failed_peer_and_commits_only_after_all_success`、`delivery_stops_after_bounded_attempts`、`clip_preview_does_not_commit_hash_until_delivery_finishes`；poll 链五重去重（policy → change_token → pending/terminal/last_sync hash）+ SensitiveContentGuard 前置 + max_payload_bytes 封顶；OTP 复制仅在剪贴板未被换掉时延迟清除（不变更检测）。③`notify.rs` 横幅抑制链有序：banners_enabled → DND（All 拦截除非 OTP+allow_otp_in_dnd）→ ADR-070 专注模式（OTP 突破受 policy 门）→ OTP 掩码（mask_otp 默认 true）；ANCS 泵跑专用线程（BLE 调用阻塞，current-thread runtime 隔离）；Removed → dismiss_uid → CandidateDismissed 闭环。④`actions.rs` receive_action_trigger 5s per-peer 冷却 + Disabled/eligibility/PromptOnly 策略门 + 本地 enable 查验；timeout = min(action, policy.max_timeout).clamp(1,30)；命令进程 `kill_on_drop(true)`（§49 v0.2.49 修复，测试 `timed_out_action_command_is_killed` 在位）；输出经 `ActionValidator::sanitize_output`；结果缓存 VecDeque cap 100。⑤`messages.rs` 发送侧 TEXT_MESSAGE_MAX + 空 trim 双验；audit/事件仅记长度不记明文（Zero-Plaintext）；MessageEngine/OutboxEngine 双有界环（max(10) / clamp(10,1000)）；outbox resend 走 `validate_send_source_path` 复检（Batch 1 受控目录策略在位）；memory-only 转发明确拒绝而非静默失败。⑥`audit.rs` 环形缓冲 push-at-capacity 弹 front（max_capacity.max(10)）；query severity ≥ 过滤 + limit 截断；clear 自身记 Notice 审计。⑦`session.rs` accept_one 在 handshake 后查 blocklist（被 block 的信任设备侧信道照样拦）再 persist/attach；send_to try_send Full/Closed 转可读错误不 panic。⑧`config.rs` 节点名空/64 字节/控制字符全拒；tray owner 切换 Client 启动 keepalive 窗口 / Daemon 清保活 + tray_sse_seen 复位（TRAY_TAKEOVER_RECLAIM_MS）。
  - P4 观察项（不行动，登记备查）：①`messages.rs` `get_by_id` 支持 `starts_with` 前缀匹配——便利 API 但前缀撞车时返回最近一条，属 CLI 便利性权衡，非安全面；②`notify.rs` ANCS 泵线程 panic 仅终止泵线程（BLE 环境异常场景），通知功能降级但主进程不受影响，无自动重启——可观测性可再加强，属增强项非缺陷。
  - 验证：纯审查无 diff，无需 CI 门禁。
- **2026-11-21 §203 tetherly-crypto 全 crate 深审（noise/pairbind/identity/spake/aead/kdf/vectors/error/lib，959 行，纯审查，零代码缺陷，无代码变更）**：
  - 范围：`noise.rs`（235 行 Noise_IK_25519_ChaChaPoly_BLAKE2s 封装）、`pairbind.rs`（166 行配对绑定签名/加密）、`identity.rs`（143 行 Ed25519+X25519 双静态身份）、`spake.rs`（124 行 SPAKE2-Ed25519 角色封装）、`aead.rs`（62 行文件通道 ChaCha20-Poly1305）、`kdf.rs`（52 行 Argon2id/HKDF）、`vectors.rs`（102 行冻结测试向量）、`error.rs`、`lib.rs` 全读。至此 tetherly-crypto 全 crate 审毕，深度 review 迭代全仓库核心四 crate（core/net/node UI 面/crypto）收官。
  - 关键正面论证：①IK prologue 双方 Hello canonical 字节哈希绑定（名字篡改即握手失败，测试钉死）；`remote_static` 显式暴露并带文档警示——responder 必须比对信任静态键，防「重放受信 Hello + 自有静态」顶替攻击（`responder_learns_the_initiator_static` 测试完整复现攻击者路径）；②rekey 2^16 msgs / 3600s 双阈值 + `started_unix > 0` 未设防（不用 0 值误判 3600s）；模块头明令「勿自造 rekey」；③PairBind 方向分离 nonce（`tetherlybndI`/`tetherlybndR`，RFC 8439 (key,nonce) 重用禁令），decode 严格等长校验（`129 + id_len`），`verify_self` = DeviceId 重导出绑定 + `verify_strict` 签名（ed25519 malleability 加固）；④Identity Ed25519/X25519 标量独立不混用（模块头明令），`ZeroizeOnDrop` 且 pk/device_id skip 不误清公钥，`verify_strict` 全库唯一签名验证口；SPAKE2 密码/outbound Drop zeroize；⑤AEAD counter_nonce 4 字节定长前缀 + 64 位大端计数器，per-session key 由握手哈希派生（跨会话不重用）；`base64_len`/MAX_WIRE_CHUNK_BYTES 预算算术有测试钉死；⑥Argon2id m=64MiB/t=3/p=1 冻结，pairing_salt 双键字典序排序保证顺序不变性（测试钉死）；HKDF expand `[u8;32]` 输出长度恒成立（expect 仅 >8160 字节才触发）；⑦vectors.rs `#[cfg(test)]` 门控——冻结种子不进发布库（lib.rs 注释明示）；lib.rs `#![forbid(unsafe_code)]`。
  - P4 观察项（不行动，登记备查）：①`aead.rs` seal/open 失败统一映射 `CryptoError::PairBind`——语义名不副实（实为文件通道 AEAD 失败），仅影响错误分类显示，不影响安全（decrypt 失败本来就 fail-closed）；②`NoiseTransport::handshake_hash` 在 `into_transport` 时缓存而非实时读 `get_handshake_hash`——snow 0.9 transport 模式下该 API 行为一致，属防御性快照，无行为差异。
  - 验证：纯审查无 diff，无需 CI 门禁。
- **2026-11-21 §202 core/ 根目录剩余小模块收官（ports/bench/virtual_display/locate/keymap/focus/recommend/overlay/battery/dedup/device_id/error/insertion/persist/replay/screen/screen_font/text/wire，纯审查，零代码缺陷，无代码变更）**：
  - 范围：`ports.rs`（419 行 Clock/Rng/TrustStore/Opener 等 trait 端口 + NotificationPayload）、`bench.rs`（399 行吞吐基准四帧 + BenchValidator）、`virtual_display.rs`（382 行虚拟副屏 announce/attach/detach/ack）、`locate.rs`（320 行设备寻回铃声）、`keymap.rs`（287 行 Win32 VK 修饰键重映射）、`focus.rs`（265 行专注模式帧）、`recommend.rs`（250 行带宽分档推荐）、`overlay.rs`（238 行 EasyTier sidecar 发现）、`battery.rs`（233 行电量同步）、`dedup.rs`（209 行 NotifyDeduper + 五类 TokenBucket limiter）、`screen_font.rs`（207 行 8x8 点阵字库）、`device_id.rs`（94 行 tdev_ base32 身份）、`error.rs`（127 行）、`insertion.rs`（130 行验证码保守写入计划）、`persist.rs`（83 行 TrustFile DTO）、`replay.rs`（72 行 ReplayGuard）、`screen.rs`（88 行模块清单）、`text.rs`（155 行 UI 格式化/百分号编码）、`wire.rs`（89 行 base64 chunk 预算）全读；handoff.rs 已由 §186 覆盖不重读。至此 tetherly-core 全 crate 审毕。
  - 关键正面论证：①ReplayGuard 全 wrap-safe（`saturating_add` 防攻击者 u64::MAX panic，`next > 1024` 门槛防初始窗口误杀，高水位 + 1024 滑窗双检，u64::MAX 极端用例钉死）；②dedup TokenBucket `saturating_sub` 防时钟回拨膨胀 token、evict-only-on-写路径有界（NotifyDeduper 10min 窗、TcpLimiter 30/IP/min per spec §7.3、Clip 4/s / Handoff 2/s+5 / Media 5/s+10 与 §186/§190 一致）；③bench/virtual_display 解码直通（`from_payload` 无 sanitize）但下游防线完备：node `receive_virtual_display_attach` 解码即 `frame.sanitize()` + `validate_session_id` fail-closed（runtime/virtual_display.rs:116-120），detach 同款（:230），FrameAck session_id/peer 双匹配（:278，与 §188/§200 ownership 模型一致），bench 两条 ingest 入口 `validate_request`/`validate_data_frame`/`validate_session_id` 前置（runtime/bench.rs:122/352/375），且 BenchData 载荷不进事件/日志/audit（Zero-Plaintext 注释在案）；④BenchValidator clamp/validate 双轨、MAX_CHUNK_BYTES 共享 wire.rs 预算（32768 = base64 后恰好装进 INNER_PAYLOAD_MAX，测试钉死旧 65536 cap 必爆预算）；⑤device_id base32 字符集白名单 fail-closed；keymap 纯查表无越界；locate/battery/focus 全 decode 即 sanitize + clamp；overlay CIDR 解析 prefix 1..=32 且 push_ip 排除 loopback/multicast/broadcast；text.rs 百分号编码逐字节（UTF-8 字节级转义）；recommend 非有限/负值降级 minimal 档 + `validate()` 全档经 owning validator 复检。
  - P4 观察项（不行动，登记备查）：①`VirtualDisplayAnnounceFrame` 无 sanitize（display_name/device_id 无 cap）——但 node ingest 仅 debug! 日志消费（runtime/virtual_display.rs:98-103），不进 UiEvent/audit，peer_str 来自本地 DeviceId；②`FocusStateFrame::from_payload` 直通 `serde_json::from_slice` 后仅消毒 reason——`request_id` 不存在于该帧（属 SyncReq 帧，其解码后 `validate_request_id` fail-closed），实际无暴露面；③overlay `iface_looks_easytier("eth" 前缀排除)` 对 `ether` 类非 TUN 接口可能误报（仅影响路径偏好启发，CIDR 判定是权威源）。
  - 验证：纯审查无 diff，无需 CI 门禁。
- **2026-11-21 §201 core/ 信任与会话面深审（session/hello/bind/pin/otp/allowlist/liveness，纯审查，零代码缺陷，无代码变更）**：
  - 范围：`crates/tetherly-core/src/session.rs`（311 行 LockoutTable + dispose_hello/commit_trust）、`hello.rs`（142 行 HELLO_MAX_BYTES=4096 + device_id 绑定）、`bind.rs`（198 行 BindClass 地址分类）、`pin.rs`（80 行 8 位 PIN 生成）、`otp.rs`（268 行冻结短信验证码提取器）、`allowlist.rs`（168 行 OpenAllowlist 开放规则）、`liveness.rs`（135 行心跳/死线）全读 + allowlist 下游取证（clipboard.rs:302-313 open_candidate → platform.rs:228 SystemOpener；规则入口 apiv1/model.rs:1868 ancs_allow → add_open_rule → push → from_rules 校验；allowlist 非持久化，lifecycle.rs:523 启动为空）。
  - 关键正面论证：①LockoutTable 5 次失败锁 15min，sweep 仅在 fail() 写路径以 64 表下限触发（无锁下清表竞态）；`dispose_hello` 四分支互斥（None→Pair / 已撤销→Err / key 变更→StaticKeyChanged / 其余→ResumeNoise）；②hello 解码 4096 cap + proto 校验 + base64 key 解码失败 fail-closed + `DeviceId::from_id_pk` 绑定（DeviceIdMismatch 防身份顶替）；③bind.rs classify_v4 守卫子句序：仅 loopback canonical、拒绝 0.0.0.0/::、EasyTier overlay 10.144.144.0/24 显式跳过、CGNAT 100.64/10 单列；④pin.rs Rng 错误 fail-closed 不降级弱随机；⑤otp 提取器冻结（sms-pop-rs 评分规则 4-8 位数字串 + 关键词窗口 48/24 + 跳过 11 位手机号 + 丢弃 1900-2099 年份）；⑥liveness `saturating_sub` 抗时钟回拨，心跳 15s/死线 45s 动态可调。
  - P4 观察项（不行动，登记备查）：①otp `find_keywords` 先 `text.to_lowercase()` 再取字节偏移——Unicode 小写化可改变字节长度，关键词位置相对原文可能错位（仅影响评分质量，无 panic；提取器模块文档明令冻结规则，不"改进"）；②allowlist `from_rules`/`push` 的 scheme 检查形如 `contains("://") && !is_local_scheme`——无 "://" 的 URL（如 `javascript:`）绕过两道检查（fail-open 形态）。取证结论：**非 wire 可达**——规则仅经本地 Admin API/UI（v1.rs:894 `POST /api/v1/ancs/allow`）录入，URL 从不出自 wire；`open_candidate` 以 app_id 为查表键，URL 全部来自本地 allowlist；allowlist 启动为空且不持久化。属管理员本地输入健壮性观察，非可利用路径。
  - 验证：纯审查无 diff，无需 CI 门禁。
- **2026-11-21 §199 screen/ 小模块收官（recording/motion/resilience/squelch/damage/watermark/color/roi/delta + overlay 补读，纯审查，零缺陷，无代码变更）**：
  - 范围：`recording.rs`（305 行 TREC 容器读写）、`motion.rs`（299 行滚动运动估计）、`resilience.rs`（290 行 PIR/LTR/SVC-T 配置与带矩形）、`squelch.rs`（210 行静止抑制）、`damage.rs`（206 行脏区网格）、`watermark.rs`（129 行）、`color.rs`（122 行）、`roi.rs`（104 行）、`delta.rs`（63 行）、`overlay.rs`（351 行 2D 光栅原语，§193 已审但本轮复核确认）全读；至此 `core/src/screen/` 33 个模块（25,904 行）全部审毕，目录级收官。
  - 关键正面论证：①TREC reader `next_record` 首字节单独 read 区分干净 EOF 与截断（截断 read_exact 硬错）；`scan_summary`/`extract_frame` 用 seek(Current(len)) 免分配跳过，len 以 i64 承接防 u32→isize 回绕；decode 产物统一进 `ScreenFrame::decode_payload`（31B header + 8MiB cap）；②motion `estimate_scroll_motion` 全部采样点 `c_off + bpp <= len && p_off + bpp <= prev.len()` 双侧边界检查，d_max 双重 clamp（配置域 + `h.saturating_sub(2)/2`），压缩帧/尺寸/格式不一致短路 None；③PIR `compute_pir_band_rect` bands clamp 2..32 且 `.min(height.max(1))`，末带 saturating_sub 吸收余数严格覆盖全高；④squelch 状态机 `last_change_us > 0` 防零值伪激活、keepalive 2.0s/5.0s 保证静止画面保活，force_wake 清零静态计数；⑤damage `evaluate_rgba` 总像素零/长度不等/长度不足三重前置 fail-safe（全脏兜底非 panic），块比较用行切片不再逐像素索引；⑥ROI sanitize 中心对齐偶数尺寸防 codec 色度撕裂、`zoom clamp 50..800`、`map_screen_to_roi` 无下溢（先比 x/y 再减）；⑦overlay `blend_pixel` guard `offset + 3 >= pixels.len()`、Canvas::blend 负坐标拒绝、laser_spot/disk 循环域 clamp 到画布内、annotation_text scale min(4) + py/px 逐点边界 continue；⑧watermark/qoe/color/delta/report 类全部为纯配置+遥测 DTO，clamp 在位（color threshold 1..254、delta keyframe 1..600、qoe psnr 10..60）。
  - 验证：纯审查无 diff，无需 CI 门禁。
- **2026-11-21 §198 screen/ 呈现与遥测模块深审（playback/qoe/audio_sync/annotation/cursor，纯审查，零缺陷，无代码变更）**：
  - 范围：`playback.rs`（411 行回放引擎+虚拟源）、`qoe.rs`（406 行 PSNR/SSIM QoE 闭环）、`audio_sync.rs`（388 行 PTS lip-sync）、`annotation.rs`（387 行白板/标注）、`cursor.rs`（383 行光标流）全读。
  - 关键正面论证：①playback `step` 速度负值钳回 1.0、帧间等待 cap 5s（防超长 PTS 跳变阻塞）、loop 回卷 `last_pts_us=None` 防跨轮巨 delta；`ScreenRecordingSource::grab` Mutex 保护 next_record 状态、循环回卷后二次 EOF 返回 None 不死循环；②QoE evaluate 抗振荡：cooldown 500ms + Off 模式短路 + `evaluated_frames==0` 短路；action 分级阈值（ssim_deficit>0.15→UpgradeScale / psnr>8→LowerQp / 一般→BoostBitrate / psnr<20→ForceKeyframe）单调合理，`active_boosts` saturating；③audio_sync beacon 队列 VecDeque(8) 有界、`on_sync_beacon` skew 补偿 `clamp(-20ms, 20ms)` 且除 2（防过矫）、drift_ppm EWMA 0.7/0.3 且首样本直取、`elapsed_sender >= 100ms` 门槛防短窗噪声、recent_samples 16 上限；④annotation 白板 `max_items` 容量前置拒绝（add/import 双路径）、TTL `saturating_add` 无溢出、undo/redo per-author rposition 语义自洽、import 重编 id 防对端 id 冲突、JSON decode 需 magic5 字节头（HEADER_LEN=5）且 serde 错误 fail-closed；⑤cursor `decode_payload` 32 字节头 + `custom_rgba` 严格等长 `(w*h*4)` 且 ≤ CURSOR_BITMAP_MAX_BYTES 双条件、width/height u16 乘积 * 4 在 usize 下不溢出（64 位平台）且有 cap 兜底、controller 去重（同位置/形状/可见性且无新位图才丢）与 rate-limit 语义变化旁路正确。
  - P4 观察项（不行动，登记备查）：①playback `step` 为阻塞式 sleep（同步 API 设计如此，node 侧在独立线程调用，不阻塞 runtime）；②annotation `total_added as u32` 在 >42 亿条时回绕——实际受 max_items=1000 容量约束不可达。
  - 验证：纯审查无 diff，无需 CI 门禁。
- **2026-11-21 §197 screen/ 控制器三件套深审（tuner/twcc/probe/profiler，纯审查，零缺陷，无代码变更）**：
  - 范围：`tuner.rs`（619 行瓶颈调优 FSM）、`twcc.rs`（600 行传输层拥塞反馈收发）、`probe.rs`（698 行主动带宽探测）、`profiler.rs`（547 行 7 段延迟剖析）全读。
  - 关键正面论证：①tuner 同瓶颈 streak 计数 + 冷却仅作用于首个真实动作后（`last_action_us != 0`）、recovery `pop_back` 逆序弹 degraded 栈、apply_relief 按阶段映射（Capture→ReduceFps / Encode→RaiseQp≤QP_MAX-5 else ReduceScale / Pacer·Network→ReduceBitrate / Jitter→RaisePlayoutDelay / Decode·Render→ReduceScale else ReduceFps）互逆恢复均带 clamp；②twcc decode `required_len = HEADER + count*4` 严格等长、receiver arrivals `while len >= 512 pop_front` 有界、feedback span clamp 1..256、missing 用 sentinel delta 且 retain 只保留 acked span 外序列；sender in_flight `while len >= 1024 pop_front`、allocate_seq u16::MAX 回绕到 1、EWMA 全部首样本直取（rtt/loss/delivery_rate/delay_gradient）+ dt≥100us 门槛；③probe cluster_id `(id % 254)+1` 防零、packet_queue clear 前置、1.5s 超时回收 active_cluster、target_bps clamp 进 BITRATE 域、on_twcc_feedback 判定域 `wrapping_sub(start_seq) < cluster_size`（u16 回绕安全）、验证门（loss≤15% + rate≥70% target + delay_gradient≤20ms）三条件与 AND、失败需 ≥3/4 记录才置 Failed（防半反馈误判）；④profiler 环形 samples `len >= max pop_front` 有界、EWMA alpha 0.15 首帧直取、percentiles `min(len-1)` 索引钳制、瓶颈判定 35% 占比双路径（EWMA 主 + 单帧兜底）、Disabled 模式 record 短路。
  - 验证：纯审查无 diff，无需 CI 门禁。
- **2026-11-21 §196 screen/ 适配与控制面深审（adapt/control，纯审查，零缺陷，无代码变更）**：
  - 范围：`adapt.rs`（1271 行状态机/统计/速率·fps·scale·quality 控制器/矩阵协调）、`control.rs`（1170 行 ControlMsg 73 标签控制面）全读。
  - 关键正面论证：①adapt consent FSM Allowed 仅本地 allow 可达；AIMD on_congestion 80% MD + 冷却 + fast_ramp_up clamp + 连续 5 好帧且 in_flight==0 且 rtt<120ms 才小步升；RTT tracker capacity.max(8)、p95 排序索引钳制；scale_dims 偶数尺寸 min 2×2；downscale_bgra8 16.16 定点 `>>8` + `min(255)`；downscale_frame 压缩帧拒绝 + 严格 `bytes.len() == pixel_bytes`；矩阵协调 demote 冷却 400ms / promote 3s+15 健康 ack / Critical ≥2 连续丢；②control 14 字节定长信封 + magic/proto + 73 标签白名单（未知 tag → Err fail-closed）；位打包编码/解码对称（SetLaserPointer x|y<<16|active<<32|rgb<<40..56、TileNackRequest seq&0xFFFFFFFF|index<<32|retry<<48）；全部 decode 参数防御性 clamp（tile_size/max_colors/bands/radius/disp/lod 1..2/zoom 等）——恶意对端无法注入越界配置。
  - 验证：纯审查无 diff，无需 CI 门禁。
- **2026-11-21 §195 screen/ tile 族深审（tile/tile_compress/tile_lod/tile_schedule，纯审查，零缺陷，无代码变更）**：
  - 范围：`tile.rs`（519 行网格/字典/焦点）、`tile_compress.rs`（706 行调色板+预测编解码）、`tile_lod.rs`（622 行 LoD 降采样+运动向量）、`tile_schedule.rs`（709 行呈现双缓冲+重发环+显著性分级）全读。
  - 关键正面论证：①tile_compress 调色板 >16 色早退回 RAW；SOLID 5B / BILEVEL 9B+位图 / PALETTE16 2B+K*4+半字节 nibble，decompress 逐像素 `byte_idx >= len` / `idx >= count` 拒绝；RCT/paeth 断言长度 + wrapping 数学；predictive 严格 expected_len 检查 + codec.decompress_frame 结果长度复核；②tile_lod down/upsample 边界 `off+3 < len`、`saturating_sub(1<<15)`、`.get().unwrap_or(0)`、`min(255)`；chunk_dirty_tiles 14B 头预算；estimate_motion 每 4 像素采样 + ref 边界 vs canvas 域检查 + 0.98 早退 + 垂直→水平→二维对角搜索序；motion decompress ref 域按 rect 尺寸检查、receiver 画布恒 stride=w*bpp 故 effective_stride 相等（确认安全）；③tile_schedule 双缓冲维度匹配才 copy_from_slice 复用、ResendBuffer 环形容量 4、saliency `data.len() < w*h*4` 前置守卫 + 逐 2 采样 + (tier, 原序) 稳定排序、拥塞 bg-drop；④tile 字典 LRU 双映射（hash_to_id/id_to_hash）同步驱逐、extract_dirty_tiles 逐行 `p_off + tile_row_bytes <= bytes.len()` 失败即置 dirty（fail-safe 全脏）。
  - 验证：纯审查无 diff，无需 CI 门禁。
- **2026-11-21 §194 screen/ 传输链深审（fec/retransmit/pacing，纯审查，零缺陷，无代码变更）**：
  - 范围：`crates/tetherly-core/src/screen/fec.rs`（698 行，GF(256) Cauchy RS 编解码 + FecAssembler）、`retransmit.rs`（454 行，HARQ/NACK/RetransmitBuffer）、`pacing.rs`（554 行，PacketPacer + TrendlineFilter）全读。
  - 关键正面论证：①NACK decode `required_len=36+count*2` 恰覆盖尾部 rtt 8 字节（28+count*2+8），count 循环有界，无切片 panic；②cauchy_coeff k≤32/m≤16 ⇒ y=k+r<48 且 x=c<k≤y 恒 x≠y ⇒ gf_inv(0) 不可达，Cauchy 子矩阵可逆（奇异 Err 对合法分片不可达）；③decode_block 非齐次长度/短 orig_len 全 fail-closed（Err/TruncatedFrame→node 关 viewer），重组产物二次进 decode_payload 8MiB cap；④重复分片去重后仍跑 decode_block 但高斯消元仅在 total≥k && parity≥missing 触发且该条件成立即成功交付+清空——每 block 至多一次全量消元，凑齐前重复只走廉价 Ok(None)，无重复开销热循环；⑤RetransmitBuffer record_outbound 前置驱逐（age+capacity 双条件）有界 256；handle_nack mask 展开 ≤64 索引 × 线性扫描 ≤256 条，retx_count 分档上限；⑥pacer checked_div / trendline var_x>1.0 + MIN_SAMPLES≥5 防除零、NaN loss_ratio 落最高冗余档（安全向）、i64 saturating 全程。
  - P4 观察项（不行动，登记备查）：①FecAssembler.received_shards 无显式条数 cap——去重键 (is_parity, u8 index) 天然上界 512 条 × ≤64KB ≈ 33MB/block，与对端实发字节 1:1 无放大，seq 切换/交付即释放（TMF1 标准路径 cap 8MiB，FEC 路径为松侧）；②encode/decode 的 missing_indices 实务 ≤k+m=48，encode count u16 截断分支不可达；③decode Err 关 viewer 为 session 级 fail-closed（与 §186-§190 同类，可接受）。
  - 验证：纯审查无 diff，无需 CI 门禁（§193 提交 c3bef04 CI 44845004879 已全绿 6/6）。
- **2026-11-21 §193 screen/ 协议五件套深审（wire/overlay/sender/receiver + assembler/playout，1 处 stale 注释修复，余零缺陷）**：
  - 范围：`crates/tetherly-core/src/screen/wire.rs`（1906 行）、`overlay.rs`（350 行）、`sender.rs`（1634 行）、`receiver.rs`（996 行）、`sink.rs`（349 行，ScreenAssembler+MemorySource/Sink）、`playout.rs`（308 行）全读；至此 screen/ 第一批（协议三件套 + 重组/呈现）审毕。
  - 缺陷（P4，confirmed，已修）：`decode_payload` doc 注释写 "Header is 24 bytes"，实际 header 为 31 字节（代码 `const HEADER: usize = 31` 正确，字段清单也写 31）——纯注释过期，防误导后续读者按 24 字节推导偏移。修复：注释改 31 bytes。
  - 关键正面论证（wire）：31 字节 header 最小长度 + magic/proto/codec_tag 白名单 fail-closed；`pixel_bytes` u64 加宽防乘法溢出 + decoded-size cap `> SCREEN_PAYLOAD_MAX (8MiB)` 拒绝 decode-bomb 分配；raw 帧严格等长；compressed body ≤ rect bytes（结构性 codec MOTION_SCROLL/TILE_GRID/TILE_CACHE/TILE_BATCH 除外，各自内部再 bounds-check）；全部 tile 解码器先查 `offset + N > len` 再切片；`blend_pixel` guard `offset + 3 >= pixels.len()`；`to_frames` 分片 chunk max = INNER_PAYLOAD_MAX - 24。
  - 关键正面论证（sender）：consent FSM Idle→Requested→Allowed(仅本地 allow)→Streaming；Start-before-allow = `stats.refused+1` + Stop 回复；Ack 恰好匹配 f_seq 且 `now_us >= sent_us` 才计 RTT，pop_front while f_seq ≤ seq 有界 in-flight（64）；on_peer_gone/revoke 全量复位 next_seq=1（重连不重放）；FEC k>32 回落标准分片链。
  - 关键正面论证（assembler/sink）：TMF1 重组全链条防欺骗——`total > SCREEN_PAYLOAD_MAX` / `total == 0` / `count != total.div_ceil(CHUNK_MAX)`（防短 count "完成"时空洞 body）/ `idx >= count` / checked_mul 偏移 / `off >= total` / 每片严格等长 `expected = min(total-off, CHUNK_MAX)`；delivered_seq 水位防 straggler 分片复活已交付帧（且 reset 不清水位）；旧 seq 分片不得打断新流重组；whole-frame 到达即 reset 防污染；assembled body 直接进 `decode_payload`（31B header + 8MiB cap 二次把关）。MemorySource/Sink 为测试设施，有界 trace + 注入钩子，无缺陷。
  - 关键正面论证（receiver）：`apply` seq 窗口（≤last_seq 丢弃计数，>last+65536 报 ScreenSeqJump，非零 last 才判跳变以容忍对端 next_seq=1 重启）；raw 帧 len==raw_len 本地一致性复检；trace/trace_pairs 有界（SCREEN_TRACE_MAX）且 trace_pairs 区分 sender reset 与 HARQ 重复；on_peer_gone 全复位（seq/cursor/playout/fec/nack/twcc/probe/sync/display/privacy/profiler）；NACK 去重历史 64 条 + RTT pacing ≥20ms；`generate_nack_on_fec/frag` Disabled 模式短路。playout：RFC3550 EMA jitter、目标延迟 clamp 5ms..300ms、容量 12 帧溢出丢最旧、迟帧阈值 80ms 且仅在队列 >1 时丢（保底呈现最后一帧）。
  - P4 观察项（不行动，登记备查）：①receiver `apply` 中 profiler 启用时写入硬编码 stage 值（capture 3200/encode 4100…）——真实 stage 数据由 TYPE_SCREEN_PROFILE 帧 `on_profile_frame` 摄入（node screen_stream.rs:281/1828 两端接线在位），硬编码仅作为无 profile 帧时的占位 demo 值，total_e2e 语义可用但 stage 维度失真；②apply 的 raw len 复检只覆盖 uncompressed（compressed 一致性依赖 decode_payload 内部 cap，已足）；③playout `push_frame` 的 33_333ms 名义间隔对高 fps 略偏保守（仅影响 jitter EMA 平滑度，不影响正确性）。
  - 验证：定向 `cargo test -p tetherly-core --lib screen::` 89 全绿；fmt 过。纯注释一行修复，随 CI 提交门禁。
- **2026-11-21 §192 core 根目录收尾三模块深审（gesture_map/microphone/audio_routing，两处 confirmed 修复：gesture 平台误判 + audio 哨兵别名）**：
  - 范围：`crates/tetherly-core/src/gesture_map.rs`（589 行）、`microphone.rs`（528 行）、`audio_routing.rs`（489 行）全读 + 下游取证（input.rs gesture handler / input_peer_platform、audio.rs 全部 sanitize 调用点、hello/platform 数据源链）。至此 core 根目录模块全部审毕；下一批 screen/ 目录（~26k 行，分批）。
  - 缺陷①（P2，confirmed，已修）：`GestureMap::sequence_for` 平台判断先 `contains("win")` 后 `contains("mac")`，而 wire platform 字符串是 `std::env::consts::OS`/hello 自报（macOS 报 "macos"，peer 表/hello override 可携带 "darwin"）——`"darwin".contains("win")` 为 true（dar**win** 子串），所有 darwin 平台对端被误路由到 Windows 后端（Alt+Tab 形状在 macOS 无意义，丢 Cmd+` 真快捷键）。修复：mac/darwin/ios 判断前置 + `to_ascii_lowercase`（store.rs `platform_family` 同款语义）；RED（旧逻辑下 darwin 得 Alt+Tab 序列）→GREEN（新回归测试 `darwin_platform_routes_to_macos_backend`）。
  - 缺陷②（P3，confirmed，已修）：`AudioRoutingAttachFrame::sanitize` / `AudioRoutingDetachFrame::sanitize` 把无效 session_id 别名为固定哨兵 `"invalid_audio_session"`——违反 remote_session.rs 文档化的 no-sentinel 不变量（两个畸形对端可共享合成 id；detach 路径哨兵直接参与 `session_id ==` 比较，恶意对端可探测/影响其它畸形会话的拆除语义）。修复：blank 模式（`session_id.clear()`，空串二次 validate 失败使 validate-first 防线保持可达；detach 的 peer-only 半开匹配不受影响）。RED→GREEN（`sanitize_blanks_invalid_session_id_not_sentinel`）。
  - microphone.rs 全 confirmed-design：MicValidator clamp 族（sample_rate 8000..192000、channels 1..2、gain/rms_db NaN→MIN 显式）、validate_session_id/validate_mic_id fail-closed（与 audio_routing 同形状）、validate_chunk 上界 MAX_WIRE_CHUNK_BYTES；node audio.rs sanitize→validate-first 全在位（mic detach 无 session 别名问题——其 sanitize 只清洗 reason）。
  - 验证：core 341（+2）/ node gesture 8 + audio_routing 2 定向回归全绿；clippy 双形态 0 error；fmt 过。CI 提交后为权威门禁。
- **2026-11-21 §191 core/ancs.rs 深审（纯审查，零缺陷，全部 confirmed-design，无代码变更）**：
  - 范围：`crates/tetherly-core/src/ancs.rs` 全 1743 行（BLE 地址解析、Notification Source 8 字节事件解析、Control Point 三个编码器、DataAssembler 字节计数重组、AncsIngress 订阅/退避/串行 CP/uid 去重状态机、MemoryAncsTransport 测试传输）+ 23 个测试全读。
  - 关键正面论证：①DataAssembler 全部边界 by construction——push 上界 8KiB 超限 reset+Err、App 分支 `nul-1` 无下溢（front 是 cmd 字节 1）、`nul > 256` 上界、无 NUL 流 `DATA_BUF_MAX/2` 防扫描膨胀、未知 cmd reset+Err 不误解析；②所有字符串 `from_utf8_lossy`，无裸字节索引 String（§181/§183 类不成立）；③`NotificationEvent::parse` 先查 len≥8 再切片；④`ancs_backoff_delay` checked_shl + shift cap；⑤有界性全 cap+测试锚定（queue 256/held 64/app_names 256/seen 4096，`queue_and_app_names_are_bounded` 在位）；⑥协议不变量测试锚定：Data Source 先订阅、value-changed 只入队不写 CP、串行单 in-flight、静默重试一次、重试前 reset assembler（防粘包，`retry_resets_the_data_source_buffer`）、PreExisting 丢弃、Removed 清 pending+held（`removed_purges_a_held_notification`）。
  - P4 观察项（不行动，登记备查）：①`pending`/`events` 无显式 cap——由 3s 组装 deadline + BLE 吞吐物理上限约束（信任已配对 iPhone 威胁模型）；②`absorb_notification_attr` 的 `received` 计数含重复/未知 attr，重复 attr 可提前凑满 4 触发完成（缺字段等价 deadline 路径，无安全影响）；③`tick` 队列弹出后 write 失败丢该请求（链路已断，`on_disconnected` 统一清理）。
  - 验证：纯审查批次，无代码变更；UTF-8 字节索引模式扫描干净；不需要 CI 门禁（无 diff）。
- **2026-11-21 §190 core 大模块深审（media/remote_session/camera/drag + node 接收链，一处 P3 修复：media ingest 无限流）**：
  - 范围：`crates/tetherly-core/src/media.rs`（430 行）+ node `runtime/media.rs` 接收链、`remote_session.rs`（523 行，core）+ node input.rs RS 三 handler、`camera.rs`（532 行）+ node `runtime/camera.rs` 接收链、`drag.rs`（498 行），共 ~1980 行 + 下游取证。
  - 缺陷（P3，confirmed，已修）：`receive_media_command` / `receive_media_state` 是 §186 修复后**仅剩的无入口限流 ingest 路径**（clip 4/s、notify 2/s+burst10、handoff 2/s+burst5 均已限流）——已认证对端全速发 MediaCommand 帧时，每帧执行本地播放状态机翻转 + 2 UiEvent（SSE 广播）+ 1 audit entry，session 泵单任务顺序被独占（与 §186 同型：ingest-path rate-limit parity）。修复：`dedup.rs` 新增 `MediaLimiter`（TokenBucket per source，5/s + burst 10，媒体命令是低频人工动作），`Shared` 增 `media_limiter`，两个 receive 入口检查前置。
  - media.rs 其余 confirmed-design：`MediaValidator::sanitize_text` 截断前显式 `is_char_boundary` 回退；`MediaStateFrame::from_payload` 解码后 sanitize（wire 不可绕过）；volume clamp 0..=100。P4 观察项（不行动）：`MediaCommandFrame::from_payload` 不 sanitize（`id` 无 cap 直通 UiEvent 展示链，最坏 64KiB 短暂存在；`target_player` 仅 `new` 时清洗）——下游仅展示无存储/路径拼接，且 frame 走 INNER_PAYLOAD_MAX 预算。
  - remote_session.rs（core + node）：全部 confirmed-design——sanitize→validate_session_id fail-closed（空串二次失败即拒）；resp 双匹配（§187 约定已在位 input.rs:1075）；close 仅 peer 匹配（§187 修复）；`screen_width.max(1)` 防 0 viewport；keymap_profile 白名单（`from_preset` 校验后存储）；close 的 `revoke()` 非 `on_peer_gone`（防 seq 重置 wedge，注释锚定）。
  - camera.rs（core + node）：全部 confirmed-design——chunk 接收 validate_chunk_frame fail-closed → session_id+peer 双匹配才更新（§187 约定一致）；detach 双重校验（Batch 3e 回归测试在位）；clamp_zoom 显式 NaN→0.5；chunk 上界 MAX_WIRE_CHUNK_BYTES。chunk 接收无限流但每帧仅 mutex 计数器更新（与 §187 heartbeat 同级，代价可忽略）。
  - drag.rs：全部 confirmed-design——1 KiB wire 预算双向强制（encode+decode 测试锚定）；坐标全 clamp(0,1) 归一化 + `screen_w.max(1)` 防 0 除；状态机 drag_id+phase 双校验 fail-closed；`on_remote_enter` 无条件替换 active session 为 ADR 语义（新拖拽取代旧，ghost overlay 本地渲染）。
  - 验证：RED（临时禁用限流 → 40 帧同 tick 全跑状态机 → timestamp 断言失败）→ GREEN；node lib 324（+1）/ core 339 回归全绿；clippy 双形态（lib+tests、all-targets）0 error；fmt / `git diff --check` 过。
  - 范围：`crates/tetherly-core/src/` 小体量支撑模块 16 个共 ~2900 行：persist(83)/replay(72)/pin(79)/screen hub(88)/error(127)/device_id(94)/hello(141)/bind(197)/battery(233)/locate(320)/focus(265)/overlay(238)/ports(419)/recommend(250)/screen_font(207)/action(473)。
  - 关键正面论证：①`overlay.rs overlay_ips_from_text` 字节扫描与 otp.rs 同型——start 只在 ASCII 数字处落点、i 只推进过 digit/`.`/`/`，`from_utf8(&bytes[start..i])` 边界 by construction（§181/§183 类不成立）；②`pin.rs parse_pin` 先 whitespace 过滤→长度==8→全 ASCII 数字校验后才字节索引；③`focus.rs sanitize_reason` 截断前显式 `is_char_boundary` 回退（规范正面样本）；④`action.rs` clean_identifier/sanitize_text 按 char 推入无裸切片；⑤`replay.rs` u64::MAX wrap-safe（saturating_add）有测试锚定、recent 有界 1024；⑥`device_id.rs` `expect("10 bytes")` 对 32 字节 sha256 截断不可达；⑦`recommend.rs` 非有限/负输入降级 minimal tier 不 panic，五 tier 全部经 owning validator fail-closed 复验；⑧`bind.rs` classify 单一决策源 + reason 全覆盖测试（diagnostic 与 binder 不可漂移）；⑨`locate.rs`/`battery.rs`/`action.rs` frame 全部 decode 后 sanitize（wire 不可绕过），request_id 白名单字符集。
  - P4 观察项（不行动，登记备查）：locate.rs sanitize_message/reason 注释写「bytes」实为 char 数（128/64 chars，非 ASCII 时内存略大但有界）；action.rs sanitize_text byte-len 检查允许单多字节字符超限 ≤3 字节（外观性，无安全影响）。
  - 验证：纯审查批次，无代码变更；UTF-8 字节索引模式扫描 16 模块均干净；不需要 CI 门禁（无 diff）。
- **2026-11-21 §188 core 批量审查（otp/insertion/liveness/keymap，纯审查，零缺陷，全部 confirmed-design，无代码变更）**：
  - 范围：`crates/tetherly-core/src/otp.rs`（267 行，OTP 提取/打分）、`insertion.rs`（130 行，粘贴写盘计划）、`liveness.rs`（134 行，会话活性策略）、`keymap.rs`（287 行，跨平台 modifier/VK 映射），共 818 行。
  - otp.rs：字节索引安全 by construction——`is_ascii_digit` 只匹配 0x30-0x39，UTF-8 续字节 ≥0x80 永不匹配，start/index 只在 ASCII 数字后推进，边界保证（§181/§183 裸 `&s[..n]` panic 类此处不成立）；`find_keywords` 用 `find()` 返回的边界偏移；输入上限由 notify.rs `ingest_push` 的 `push.truncate()`（title/body 各 4KiB）在 extract 之前钳制，组合 ≤8KiB+1；打分规则受模块头「Do not improve the rules」冻结，不重开。
  - insertion.rs：`plan()` 语义保守（empty→全写、prefix→strip_prefix 补余、complete→跳过、无关→refuse），无 panic 型字符串操作，reason 为静态中文标签（零明文纪律）。
  - liveness.rs：纯策略 + 时钟注入；`is_expired` 用 `saturating_sub`，时钟回拨读作「刚听到」而非长静默；常量 heartbeat 15s / deadline 45s 与三连丢 ping 设计注释一致；无任何 panic 路径。
  - keymap.rs：纯 VK/modifier 查表（`remap_vk` = HashMap 查找 + match fallthrough，u16 域全闭合无越界）；唯一字符串操作 `from_preset` 的 trim/to_lowercase/replace 均为自有安全操作无切片；Meta+Any→VK_LWIN 为惯例代表值（确认设计）；serde roundtrip / preset 解析 / custom-key 优先级均有测试锚定。
  - 验证：纯审查批次，无代码变更、无新测试；UTF-8 字节索引模式扫描（§181/§183 教训）四模块均干净；不需要 CI 门禁（无 diff）。
- **2026-11-21 §187 core/input_tunnel.rs 深审 + node 隧道/远程会话接收链（一处 P2 修复：resp/release/close 缺 peer 绑定，可猜 session_id 跨 peer 拆除/劫持）**：
  - 范围：`crates/tetherly-core/src/input_tunnel.rs` 全 305 行（四个 wire frame + InputTunnelValidator）+ node 接收链取证（input.rs receive_input_tunnel_request/resp/release/heartbeat、receive_remote_session_resp/close、input_tunnel_status idle sweep、session_id 生成 input.rs:1469 `tun_{now_ms}` / remote_session.rs:83 `rs_{now_ms}`）。
  - 缺陷（P2，confirmed，已修三处）：①`receive_input_tunnel_resp` 只按 session_id 匹配 armed session，不绑 sender——第三对端伪造 ACCEPTED resp（猜中 id）可提交 capture size 并把 accept 链的 `open_input(peer)` 拨向攻击者；②`receive_input_tunnel_release` 的 `session_id == frame.session_id || peer` OR 匹配——OR 让猜中 id 的任意对端通过第一臂单独授权，跨 peer 杀掉他人活跃隧道（v0.2.49 测试只盖了 stranger+异 id 臂，stranger+同 id 臂是残余洞）；③ADR-138 `receive_remote_session_close` 同型 OR 匹配同型洞。session_id 生成是 `tun_<now_ms>`/`rs_<now_ms>` 毫秒时间戳，可猜测放大了三处。修复对齐 remote_session resp（input.rs:1075）已有的 `session_id && peer_id` 双匹配约定：resp 要求双匹配；release/close 改为仅 peer 匹配授权（保留 half-open 语义——会话自己的 peer 可用过期 id 拆除，镜像 rs 侧注释的设计意图）。
  - confirmed-design（不重开）：`validate_session_id` 字符集白名单 + sanitize 失败映射空串后二次校验 fail-closed；`sanitize_reason` 控制字符过滤 + 64 字符 clamp（§186 handoff reason 同规范）；`clamp_coords` 饱和减法防 u32 下溢；heartbeat 无限流但 handler 仅两次 mutex 更新（代价可忽略）；resp 的 capture w/h 有 `>0` + `min(i32::MAX)` 防护；release 的 entry warp 双重 clamp（拓扑尺寸 + 真实 screen_size）；idle sweep 区分 roaming/Local 座位防 v0.2.27 复合收割（生产已验证）。
  - P4 观察项（不行动，登记备查）：`InputTunnelRequestFrame.client_device_id` 仅 trim 无长度 cap——handler 不使用该字段（peer 身份取自 wire 层 DeviceId），最坏 64KiB 字符串短暂存在，无存储/路径拼接；session_id 毫秒时间戳在两 peer 同毫秒发起时理论碰撞——peer 绑定后碰撞只导致握手失败（fail-closed），无需改用随机 id。
  - 验证：三个新回归测试全部先 RED（回退修复后在生产行 panic：release `left: None`、resp capture size 被提交、RS close 会话被拆）→ GREEN；node lib 323（+3）/ core 339 回归不变 / clippy all-targets 0 / fmt 过。
- **2026-11-21 §186 core/handoff.rs 深审（一处 P3 修复：入站 handoff 无限流 + 每帧 fsync 落盘）**：
  - 范围：`crates/tetherly-core/src/handoff.rs` 全 414 行（ResourceHandoffValidator、HandoffAction/Status/Entry）+ 下游取证（node runtime/handoff.rs receive_resource_handoff、store.rs HandoffInboxTable/atomic_write、platform.rs SystemOpener、allowlist.rs is_local_scheme、frame.rs ResourceHandoffFrame wire cap）。
  - 缺陷（P3，confirmed，已修）：`receive_resource_handoff` 是同层唯一没有入口限流的接收路径——clip（core `apply_remote` ClipLimiter 4/s）、notify（`ingest_push` NotifyLimiter 2/s+burst10）均有限流，handoff 没有；且每个被接受的帧都触发 `save_handoff_inbox` → `atomic_write`（tmp + `sync_all` fsync + rename + restrict）整个 inbox JSON（默认上限 100 条）。session dispatch 是单任务顺序泵，已认证对端全速发 handoff frame：每帧消耗数 ms fsync 延迟，同会话所有其他 frame 类型（input/screen/caps/file 控制帧）被排队阻塞——会话级 DoS，且磁盘 rename/tempfile churn。修复：`dedup.rs` 新增 `HandoffLimiter`（TokenBucket per source，2/s + burst 5，handoff 是低频人工动作），`Shared` 增 `handoff_limiter`，`receive_resource_handoff` 开头检查（限流在 validator 与磁盘工作之前）。
  - confirmed-design（不重开）：validator 全部字节切片安全——`:`/`/`/`]`/`@` 均为 ASCII 分隔符，UTF-8 续字节 ≥0x80 不可能伪冒（§181/§183 类 panic 模式此处不成立）；banned scheme 表 + http(s) 强制前缀匹配；userinfo `@` 拒绝；`169.254.169.254` 元数据 IP 拒绝；IPv6 括号处理；节点侧强制传 `Some(&allowlist)`（fail-closed，非默认 scheme 一律拒绝）；SystemOpener 单 argv 无 shell、URL 全量字符校验（控制字符/反斜杠/空格/CRLF）。
  - P4 观察项（不行动，登记备查）：①`ResourceHandoffFrame::from_payload` 未 cap `id`（url/title 有 cap）——id 仅作 inbox 查找 key 不进路径，最坏 64KiB×100 条 JSON 有界；②元数据封禁为单 IP 精确匹配，AWS IPv6 IMDS `fd00:ec2::254` 不在表内——云桌面防御纵深层面，浏览器自身有 IMDSv2 token 防线；③重复 id 幂等未处理（同 id 重复入箱，open 取首个匹配）——sender 只能污染自己的条目，无安全影响。
  - 验证：RED（临时禁用限流检查 → 20 帧同 tick 全收 → 断言失败）→ GREEN；core 339 / node 320（+1）/ clippy 双 crate all-targets 0 / fmt 过。
- **2026-11-21 §185 core/filexfer.rs 深审（一处 P3 修复：Accepted 条目永不过期致表饱和）**：
  - 范围：`crates/tetherly-core/src/filexfer.rs` 全 543 行（FileHub 状态机、ResumeTable、offer/accept/done/prune 生命周期）+ 全链路取证（node lifecycle.rs FileOffer/FileDone 处理与 auto-accept、files.rs accept_file 数据通道校验、runtime.rs load_resume_table sidecar 对账、filechan.rs header/chunk/total 四级超时、dispatch.rs wire 解码）。
  - 缺陷（P3，confirmed，已修）：`prune()` 无条件保留 `Accepted` 条目，其注释前提「只有 `Offered` 无需本地点击即可达」被 auto-accept 打破——信任对端 + 宽松策略（`is_eligible` 无过滤器时全通过）下，对端连发 256 个 offer 全自动 `Accepted` 且从不打开数据通道，`INCOMING_MAX=256` 永久饱和，之后所有新 offer（含正常对端）一律 `UnknownTransfer` 拒绝直到进程重启。用户手动 accept 后对端消失同样泄漏（`cancel_transfer` 不触 hub 条目，`forget` 生产仅 peer-reject 路径调用）。修复：`IncomingTransfer` 增加 `accepted_ms`（`accept_at` 落章、`rollback_accept` 清零），`prune` 对 `Accepted` 施加 `ACCEPTED_TTL_MS = 1h`（远超 filechan `FILE_TOTAL_TIMEOUT` 30min，活跃传输必然窗内 `mark_done`；`mark_done` 后转 `Done` 仍被 prune 立即驱逐）。
  - confirmed-design（不重开）：`is_safe_transfer_id` 字符集白名单防 v0.2.49 路径逃逸回归；offer 校验序 id→文件名 sanitize→sha256 64-hex→FILE_MAX→重复幂等（异源/异载荷拒绝所有权劫持）→INCOMING_MAX；resume 链完整闭环：`ResumeTable::claim` 三重校验（source+total+对齐）→ 数据通道 `start != resume_hint` 硬拒 → HKDF(Noise handshake, transfer_id) 逐块 AEAD → sidecar 与 `.part` 长度对账丢弃撕裂态；发送方完成本地闭环（不依赖对端 FileDone）；接收 `.part` 按 start 截断 + prefix 重哈希防部分伪造。
  - P4 观察项（不行动，wire 不可达）：`mark_done` 多文件 else 分支为空占位（v1 节点全单文件构造，5 处 `files: vec![...]` 均单元素）；多文件 offer 的 sum/first 口径差仅在 wire 伪造 multi-file 时导致 `accept` 的 resume 校验失败（fail-closed，无内存/越界风险）。
  - UTF-8 字节切片模式扫描（§181/§183 教训）：本模块无裸字节索引 String 的位置，干净。
  - 验证：RED 在生产 `prune` 处 panic（`abandoned accept must expire`）→ GREEN；core lib 339（+1）全绿；`state_machines` file 3 项回归绿；`cargo check -p tetherly-node` 过；clippy（lib+tests）0 error；fmt 过。
- **2026-11-21 §184 core/notify.rs + dedup.rs 深审（纯审查，零缺陷，全部 confirmed-design）**：
  - 范围：`crates/tetherly-core/src/notify.rs` 全 551 行 + `dedup.rs` 全 173 行；回访 §182 P4 观察项。
  - 审查结论（不修，确认为设计）：
    - `ingest_push` 检查序正确：truncate → rate limit（2/s+burst10 per source）→ dedup（10min 窗口 retain 驱逐）→ OTP 本地提取 → insert；`candidate_id` 含 ts 的 sha256 无碰撞注入面。
    - `TokenBucket` 数学正确（elapsed 饱和减法防时钟回拨、token 封顶 burst）；三个 limiter 的 map key 为强类型 `DeviceId`/`IpAddr`，按已认证源数有界。
    - Reply 队列无 TTL 但有界性成立：wire 路径 `NotifyReply::from_payload` 强制 truncate（text ≤ 4KiB）；`replies` keyed by `reply_id(source, app_id, uid)` 覆盖式 last-write-wins（同通知重复 reply 不增长）；take-once 语义有 pin 测试。§182 P4 观察项维持「不行动」：candidates（TTL 驱动）与 replies（覆盖式）皆有界。
    - 零明文纪律全链路：text/title/OTP 不入 tracing/Debug/ReplyView；`copy_otp` 三分支（unknown/expired/live）语义完整。
  - 无代码变更；core 测试基线 338 不变。
  - 残余：无新增 manual-required。
