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
