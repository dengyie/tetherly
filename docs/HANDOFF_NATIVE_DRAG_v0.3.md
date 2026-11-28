# 阶段交接：跨端原生拖拽 / 去重 / 文件夹传输（v0.3.0 – v0.4.1）

- 更新日期：2026-11-28
- 覆盖范围：`v0.2.99..v0.4.1`（v0.4.0 = v0.3.x 里程碑版本，见 `docs/releases/v0.4.0.md`；v0.4.1 = 流式文件夹 + helper 根因修复）（Mac 控制端 → Windows 受控端 home-win）
- 用户验收：普通文件、文件夹均可从 Mac Finder 拖过边后在 Win 侧直接作为鼠标拖住的副本落到任意窗口（v0.3.4 实测通过）

---

## 1. 链路总览

```
Mac (控制端)                                         Win (受控端, home-win)
───────────────────────────────────────────────     ──────────────────────────────────────────────
Finder 拖文件/文件夹过屏幕边缘
  drag_probe 读 pasteboard → carry
  RemoteDragEnter{has_directory,…} ───────────────▶ native_drag_begin
                                                       ├ caps 闸门 native_drag_peer_ok:
                                                       │   dragpull && (!has_directory || dragdir)
                                                       │   不满足 → 旧路径（松手落 inbox + 选中）
                                                       ├ 注册 pending drop（自动接收）
                                                       ├ 拉起 `tetherly drag-source --session <nonce>` helper（控制台会话）
                                                       │   v0.4.1：每次接管新 nonce；被取代的 helper 轮询读到 cancel、结果不计
                    ◀──────────────── RemoteDragPull ─┘
drag_handoff::hand_off_drag_files_with
  stage_drag_item（spawn_blocking）
    文件：硬链/复制到 staging/drag/<rand>/
    文件夹（对端有 filestream cap，v0.4.1）：只 scan_dir，不落盘
      offer size=scan.total、sha256=占位、sha256_trailer=true
      接受后 spawn_dir_stream 边生成 tar 边发，末尾 AEAD 封装 digest trailer
      续传：重生 tar 跳过已保留前缀；接收端重哈希 .part 前缀
    文件夹（旧对端）：bundle::pack_dir → <folder>.tar
  刷新 enter（最终数量）
  offer_and_send_file_as(bundle=dir-tar) ─────────▶ lifecycle：认领 offer → native_drag_bind
                                                       固定路径 inbox/.drag/<drag_id>/<name|folder>
                                                     files::accept_file_from_local_copy
                                                       dedupe::lookup 命中 → 零字节完成（非 bundle）
                                                     files::accept_file 接收 → sha256 校验
                                                       bundle：unpack_into .<tid>.unpack → 逐文件 MOTW
                                                               → rename 到固定路径
                                                       文件：MOTW → rename 到固定路径 → dedupe::retain
                                                     helper 轮询 /api/v1/drag/native/poll
                                                       state=ready 后才放行 drop（CF_HDROP，仅 COPY）
                                                     helper POST /api/v1/drag/native/result
                                                       dropped → 保留副本（1h 清扫）
                                                       missed/esc → 移回 inbox（文件夹 move_dir_into）+ 选中
```

关键文件：

| 文件 | 职责 |
|---|---|
| `crates/tetherly-core/src/drag.rs` | `DragControlMsg`（含 `RemoteDragPull`），能力常量 `DRAG_PULL_CAP="dragpull"`、`DRAG_DIR_CAP="dragdir"` |
| `crates/tetherly-core/src/frame.rs` | `FileMeta.bundle: Option<String>`，`BUNDLE_DIR_TAR="dir-tar"`（serde default，旧端兼容） |
| `runtime/drag_handoff.rs` | 发送端：staging、文件夹打包、enter/confirm、offer |
| `runtime/drag_native.rs` | 接收端原生拖拽状态机（begin/bind/resolve/finish/legacy handover/sweep） |
| `platform/drag_source.rs` | Win helper：OLE `DoDragDrop`、`QueryContinueDrag` 扣 drop、看门狗 |
| `runtime/files.rs` | 接收：本地去重、bundle 解包安装、`finish_received` |
| `runtime/dedupe.rs` | 接收端 sha256 去重索引 `.dedupe.json` + `.cache/<sha>` 硬链缓存 |
| `runtime/bundle.rs` | tar 打包/安全解包/文件夹移动 |
| `uihttp/v1.rs` | `/api/v1/drag/native/{poll,result}` |

## 2. 能力（caps）协商规则

- 新消息/新语义一律加 cap 门控：旧版本遇到未知 `DragControlMsg` 变体会断会话。
- `dragpull`：对端理解 `RemoteDragPull`（v0.3.0）。
- `dragdir`：对端发送端会把文件夹打成 bundle（v0.3.4）。缺它的对端带文件夹的 carry 走旧路径。
- `filestream`（`tetherly_core::FILE_STREAM_CAP`，v0.4.1）：接收端理解 `sha256_trailer` offer（大小先知、摘要在流尾）。发送端仅对有此 cap 的对端流式发文件夹，否则仍先 `pack_dir` 落 staging。
- helper `--session` 不是 cap：helper 与 daemon 同一二进制；`NativeDragRequest.session` 缺省视为老 helper，照常服务。
- 新增 cap：`lifecycle.rs::local_caps` 插入 + 消费处 `peer_has_cap` 判断 + 单测覆盖"缺 cap 时回落"。

## 3. 排障：日志签名

日志：Win `C:\Users\<user>\AppData\Local\tetherly\tetherly.log`；Mac `~/.local/share/tetherly/tetherly.log`；健康 `curl http://127.0.0.1:45716/api/v1/health`。

| 签名 | 含义 |
|---|---|
| `native drag: carry taken over` | 原生拖拽接管成功（期望路径） |
| `native drag: peer cannot serve this carry` | cap 不满足 → 旧路径（查对端版本/caps） |
| `drop confirmed on target; requesting data stream` → `drag session delivered and revealed in file manager` | 旧回落路径 |
| `drag handoff: offering files` / `file skipped` / `staging task failed` | 发送端 staging/offer |
| `native drag handed back before any offer` | 松手早于任何 offer，交还旧路径 |
| `native drag delivered to the inbox` | missed/esc 回落 inbox |
| `drag source: DoDragDrop failed hr=…` | OLE 拒绝（v0.3.1 修过 E_FAIL：按压窗口铺满虚拟屏 + 补线程键态；v0.4.1 修掉"上一个 helper 仍占着 OLE 拖拽"这一来源） |
| `drag source watchdog: exiting … in_loop qcd_calls qcd_idle_ms wakes` | 看门狗强退（取消/失败后 5s 或存活 2h）。`in_loop=true` 且 `qcd_idle_ms` 大 = OLE 循环没在调 `QueryContinueDrag`；`wakes>0` 仍卡 = 投递的 WM_MOUSEMOVE 没唤醒，需要换唤醒手段 |
| `drag source finished … qcd_calls wakes alive_ms` | 每次 helper 正常结束的遥测；`wakes>0` 表示靠唤醒（而非用户输入）结束/放下 |
| `native drag: superseded helper's result ignored` | 同一 drag_id 二次接管，旧 helper 的结果被丢弃（预期，非故障） |
| `drag source result not needed` | helper 结果 404：已被取代或 daemon 已自行结算（info，非故障） |
| `folder stream produced X of Y bytes` / `folder stream task failed` / 传输错误 `sha256 mismatch` | v0.4.1 流式文件夹：扫描后文件夹被改（大小不符，发送端中止）/ 生成线程异常 / 接收端 trailer 摘要不符（不会安装） |
| `folder delivered incomplete` | v0.3.5：bundle 有条目未能解出（UI 同步提示 N 项） |
| `bundle: unsafe entry skipped` / `entry not unpacked` | 具体哪个条目 |

关闭原生拖拽：Win 环境变量 `TETHERLY_NO_NATIVE_DRAG=1`。单测里 `cfg!(test)` 恒关（否则 Windows CI 会真拉起 helper）。

## 4. 不变量（改动时勿破坏）

1. helper 只提供 `DROPEFFECT_COPY`，且仅在全部文件落盘后放行 drop。
2. helper 必定回报结果；daemon 不会等一个死掉的子进程（看门狗兜底）。
3. 打包不跟随符号链接；解包只建普通文件/目录，路径分量必须是已清洗的普通名字，绝不落到目标目录外。
4. 收到的每个文件在出现在用户可见路径前已打 MOTW / quarantine。
5. inbox 永不覆盖已有文件（硬链占名 `name (n).ext`；文件夹 `move_dir_into`）。
6. 去重只在 (sha256, size) 均匹配且文件经核验未被改动时命中；`.cache/<sha>` 与交付文件共享 inode，**未被有效索引时必须重算哈希**（v0.3.5 修复）。

## 5. 本阶段 Review 结论（v0.3.5 已修）

| 级别 | 问题 | 处理 |
|---|---|---|
| P1 | `dedupe::lookup` 对 `.cache/<sha>` 仅凭文件名+大小信任。缓存与交付副本是硬链接，用户把拖入应用的副本原地保存（同尺寸）后，下次同 sha 的 offer 会被"零字节完成"成被改过的内容 | 未被有效索引时重算 sha256（≤2 GiB），不符即删缓存；新增测试 `cache_edited_through_its_link_never_matches` |
| P2 | 文件夹解包有条目失败（Win 路径长度/保留名等）时静默跳过，仍报"接收完成" | `unpack_into` 返回 `Unpacked{files,skipped}`；skipped>0 时 warn + UI Status 提示 |
| P3 | `hand_off_drag_files_with` 的 staging JoinError 被 `unwrap_or_default` 吞掉 | 记 warn |
| P3 | files.rs / drag_handoff.rs 注释错位（MOTW/占名注释挂在 bundle 分支上；pull 语义注释挂在 `offer_and_send_folder` 上） | 归位 |

## 6. 遗留与下阶段建议（按优先级）

1. ~~helper OLE 循环偶发卡死只被看门狗掩盖~~ **已定位并修复（v0.4.1，f258a6c）**。根因（home-win v0.3.2 日志 drag 9374…）：
   - 拖回再拖入使同一 drag_id 被接管两次，两个 helper 轮询同一 drag；后者 `DoDragDrop` 撞前者活着的 OLE 循环 E_FAIL，其 `failed` 把整个 drag 结算掉；
   - 前者留在 `DoDragDrop` 里 2.7 s：OLE 模态循环只在鼠标/键盘消息上调 `QueryContinueDrag`（隐藏窗口的 `WM_TIMER` 不算），指针静止时 cancel 要等用户下一次移动，期间下一次拖拽也 E_FAIL。
   - 修复：helper session nonce（被取代者读到 cancel、结果不计）；daemon 已有决定（cancel/failed、或松手后 ready）时 poller 向 helper 自己的 OLE 线程 `PostThreadMessageW(WM_MOUSEMOVE)`，不向桌面注入输入，按住时绝不唤醒；`QueryContinueDrag` 计数/空闲时长/唤醒次数进日志。
   - 待真机观察：`drag source finished` 的 `wakes` 字段；若出现 `watchdog: exiting in_loop=true wakes>0`，说明 posted 消息不足以唤醒，改为 `SendInput` 零位移 move（需评估对用户桌面影响）。
2. ~~大文件夹首帧延迟 + 双倍磁盘~~ **已完成（v0.4.1，75743e0）**：`filestream` cap 下文件夹流式发送，无 staging、无预打包；摘要走 AEAD trailer；支持断点续传（重生 tar 跳前缀 + 接收端重哈希前缀）；扫描后文件夹被改 → 摘要不符不安装。旧对端仍走 pack_dir。
   - 已知限制：流式 offer 没有 `source_path`，outbox"重发"对其返回 "cannot resend memory-only"，需用户重新拖。
3. ~~tar 头开销未计入 `FILE_MAX`~~ **已完成（ddb23d6）**：`pack_dir_limited` 按 `entry_tar_bytes`（512 B 头 + 数据按 512 对齐，含 GNU 长名扩展头）+ 1024 B trailer 精确预估，单测校验与实际 tar 字节数相等。
4. ~~`.<tid>.unpack` 崩溃残留~~ **已完成（ddb23d6）**：daemon 启动 `load_resume_table` 扫描 inbox 时删除 `.*.unpack` 隐藏目录。
   - 附带：文件夹 tar 在打包时同步计算 sha256（`HashingWriter`），offer 前不再整包重读一遍。
5. **硬链接别名语义**：dedupe 交付与原生拖拽副本可能和 inbox 里的旧文件共享 inode，原地编辑一份会同时改另一份。完整性已由 #6 不变量兜住；若用户在意隔离，可改为 reflink/复制。
6. **测试 flake**：`tests/phase3.rs::m3_2_peer_gone_returns_cursor`（ubuntu CI）、Android release Maven Central 429 —— 均 `gh run rerun <id> --failed`。
7. ~~Win 上 `inbox` 测试残留~~ 已于 2026-11-28 按 hash 核对后移入回收站。

## 7. 发布 / 部署 SOP

1. 门禁（全部必须过）：
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace`
   - 动了 client：`client/src-tauri` 下 fmt/clippy/test；`client` 下 `npm run build` 后 `git status --porcelain -- client/dist` 必须为空
   - 动了 Win 平台代码：scratch crate 里 `cargo clippy --target x86_64-pc-windows-msvc --offline -- -D warnings`
2. 提交 → `git fetch -q && git rebase -q origin/main && git push -q origin HEAD:main`（共享 main：禁止 amend + force push，CI 红用 fixup 提交）。
3. `gh run list --limit 5 --json databaseId,headSha,name,status` 找 run（`--commit` 推送后可能暂时为空）→ `gh run watch <id> --exit-status`。
4. 版本号：`Cargo.toml`、`client/package.json`、`client/src-tauri/tauri.conf.json` 三处 → `cargo update -w` → 提交 → tag `vX` 推送 → release workflow（14 资产）。
5. 部署（只用 CI 产物，生产禁止本地构建）：`TETHERLY_ALLOW_STOPPED=SRBMiner-MULTI scripts/deploy.sh --target all --tag vX`，后台运行输出到日志（Win 资产下载很慢，超过 600s 前台超时）。
6. 复核：双端 health 版本、Win daemon 新 PID、客户端 relaunch=ok、受保护进程完好。
7. 更新 `.codex-memory/todo.md` 与 `session-log.md`。

### home-win 红线

- `SRBMiner-MULTI` / `verge-mihomo` / `komari-agent` 绝不启停或 kill；只 kill 核对过命令行的 tetherly 自身 helper。
- 不在用户实时桌面上做合成点击。
- `api.token` / `client.token` 不打印、不提交；远端输出过滤 `token|bearer|CLIXML|<Obj`。
- 远端命令：ps1 → `LC_ALL=C iconv -f UTF-8 -t UTF-16LE | base64` → `ssh home-win "powershell -NoProfile -EncodedCommand $enc"`。
