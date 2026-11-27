# 阶段交接：跨端原生拖拽 / 去重 / 文件夹传输（v0.3.0 – v0.4.0）

- 更新日期：2026-11-28
- 覆盖范围：`v0.2.99..v0.4.0`（v0.4.0 = v0.3.x 里程碑版本，见 `docs/releases/v0.4.0.md`）（Mac 控制端 → Windows 受控端 home-win）
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
                                                       ├ 拉起 `tetherly drag-source` helper（控制台会话）
                    ◀──────────────── RemoteDragPull ─┘
drag_handoff::hand_off_drag_files_with
  stage_drag_item（spawn_blocking）
    文件：硬链/复制到 staging/drag/<rand>/
    文件夹：bundle::pack_dir → <folder>.tar
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
| `drag source: DoDragDrop failed hr=…` | OLE 拒绝（v0.3.1 修过 E_FAIL：按压窗口铺满虚拟屏 + 补线程键态） |
| `drag source watchdog: exiting` | 看门狗强退（取消/失败后 5s 或存活 2h） |
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

1. **helper OLE 循环偶发卡死只被看门狗掩盖**（v0.3.2）：根因未定位。下次复现时抓 helper 的 `drag source` 日志序列和卡住时 `QueryContinueDrag` 是否还在 tick。
2. **大文件夹首帧延迟 + 双倍磁盘**：发送端先完整 `pack_dir` 再发 enter/offer，GB 级文件夹在 Mac 上要等打包完 Win 侧才有反应，且 staging 占一份等大空间。方向：流式 tar（offer 时 size 未知需要协议支持）或先发 enter 再打包。
3. **tar 头开销未计入 `FILE_MAX`**：`pack_dir` 只累计文件字节，接近 64 GiB 时打完包才在 offer 处 `FileTooLarge`。可按每条目 +1024 B 预估。
4. **`.<tid>.unpack` 崩溃残留**：daemon 在解包中途崩溃会在 inbox 留下隐藏目录，目前无清扫。
5. **硬链接别名语义**：dedupe 交付与原生拖拽副本可能和 inbox 里的旧文件共享 inode，原地编辑一份会同时改另一份。完整性已由 #6 不变量兜住；若用户在意隔离，可改为 reflink/复制。
6. **测试 flake**：`tests/phase3.rs::m3_2_peer_gone_returns_cursor`（ubuntu CI）、Android release Maven Central 429 —— 均 `gh run rerun <id> --failed`。
7. Win 上 `inbox\muse2api-extension (1)…(5)` 为 v0.3.3/0.3.4 测试残留，可删。

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
