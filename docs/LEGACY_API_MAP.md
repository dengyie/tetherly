# 旧 `/api/*` 路由映射表（ADR-139 §9-2 附录）

> 生成于 2026-11-17（v0.2.60 发版轮）。来源：`grep '/api/'` 于 `bin/tetherly` 与 `platform/tray.rs` 的静态调用点计数。
> 运行时命中按调用方分类见 `GET /api/v1/diagnostics` 的 `legacy_routes`（§129）。
> Android 无 HTTP 层（JNI 直连），不受本表约束；旧控制台 `/legacy/` 整体随 M5 收敛删除。

**M5 收敛完成（§169，2026-11-19）：legacy `/api/*` 路由面已整体删除（uihttp.rs 429 arms + 27 个 api_*.rs 模块 + requires_token 特权表），v0.2.63 起 daemon 只服务 `/`、`/assets/*` 与 `/api/v1/*`。下文批次表保留作迁移史存档与 v1 语义差异对照；`hub.legacy` 命中计数与 diagnostics 的 `legacy_routes` 观测字段保留至外部旧版客户端流量归零。**

## 已迁移（v1 对应端点）

| 旧路由 | v1 | 轮次 |
|---|---|---|
| `GET /api/pin` | `GET /pairing/pin`（404=无 PIN） | §142 |
| `POST /api/pin` | `POST /pairing/pin/rotate` | §142 |
| `POST /api/pin/clear` | `DELETE /pairing/pin` | §142 |
| `GET /api/health` | `GET /health` + `GET /system` | §143 |
| `GET /api/state` | `/system` + `/pairing` + `/devices` + `/transfers` 组合 | §145 |
| `GET /api/sys` | `GET /diagnostics/sys` | §145 |
| `GET /api/peers`（roster） | `GET /devices`（扁平列表按标志重建） | §144 |
| `GET/POST /api/config` | `GET/PATCH /settings` | §146 |

## 未迁移分组（按前缀；"目标"为 §4.2 规划端点或缺契约项）

| 前缀 | 路由数 | 调用点 | CLI 模块 | v1 目标 / 状态 |
|---|---|---|---|---|
| `api/screen/*` | 94 | ~165 | cmd_screen*（10 模块） | 待契约扩展 / 分批迁移 |
| `api/file/*` | 21 | ~26 | cmd_files | 待契约扩展 / 分批迁移 |
| `api/peers/*` | 18 | ~20 | cmd_peers（写操作） | 待契约扩展 / 分批迁移（§149-§152 已迁 8 条） |
| ~~`api/topology/*`~~ | — | — | — | §156b 已迁 v1（/topology、/topology/peers、/topology/policy、links/unlinks/sync/config） |
| `api/input/*` | 7 | ~8 | cmd_input | 待契约扩展 / 分批迁移 |
| `api/clip/*` | 7 | ~10 | cmd_clip | 待契约扩展 / 分批迁移 |
| `api/action/*` | 6 | ~9 | cmd_action | 已迁移 §164 |
| `api/notify/*` | 6 | ~7 | cmd_notify | 待契约扩展 / 分批迁移 |
| `api/ancs/*` | 6 | ~6 | cmd_ancs | 待契约扩展 / 分批迁移 |
| `api/handoff/*` | 5 | ~7 | cmd_handoff | 待契约扩展 / 分批迁移 |
| `api/mic/*` | 4 | ~6 | cmd_mic | 待契约扩展 / 分批迁移 |
| `api/focus/*` | 4 | ~7 | cmd_focus | 待契约扩展 / 分批迁移 |
| `api/camera/*` | 4 | ~7 | cmd_camera | 待契约扩展 / 分批迁移 |
| `api/bench/*` | 4 | ~6 | cmd_bench | 已迁移 §164 |
| `api/audio/*` | 4 | ~6 | cmd_audio | 待契约扩展 / 分批迁移 |
| `api/locate/*` | 4 | ~5 | cmd_locate | 待契约扩展 / 分批迁移 |
| `api/display/*` | 4 | ~5 | cmd_display | 待契约扩展 / 分批迁移 |
| `api/message/*` | 4 | ~4 | cmd_messages | 待契约扩展 / 分批迁移 |
| `api/media/*` | 3 | ~4 | cmd_media | 待契约扩展 / 分批迁移 |
| `api/battery/*` | 2 | ~4 | cmd_power | 待契约扩展 / 分批迁移 |
| `api/storage/*` | 2 | ~2 | cmd_power | 待契约扩展 / 分批迁移 |
| `api/reply/*` | 2 | ~2 | cmd_notify | 待契约扩展 / 分批迁移 |
| ~~`api/net/*`~~ | — | — | — | §156b 已迁 v1（GET /net、POST /net/refresh） |
| `api/metrics/*` | 2 | ~2 | cmd_core（流量） | 待契约扩展 / 分批迁移 |
| `api/discovery/*` | 2 | ~2 | cmd_core | 待契约扩展 / 分批迁移 |
| `api/daemon/*` | 2 | ~2 | cmd_core | 待契约扩展 / 分批迁移 |
| ~~`api/routes/*`~~ | — | — | — | §156b 已迁 v1（GET/POST/DELETE /routes） |
| `api/state/*` | 1 | ~2 | — | 待契约扩展 / 分批迁移 |
| `api/recommendation/*` | 1 | ~2 | cmd_notify | 已迁移 §164 |
| `api/health/*` | 1 | ~2 | — | 待契约扩展 / 分批迁移 |
| ~~`api/caps/*`~~ | — | — | — | §156a 已迁 v1（GET/PATCH /caps、/caps/peers、/devices/{id}/caps） |
| `api/sys/*` | 1 | ~1 | — | 待契约扩展 / 分批迁移 |
| `api/pin/*` | 1 | ~1 | — | 待契约扩展 / 分批迁移 |
| `api/otp/*` | 1 | ~1 | cmd_notify | 待契约扩展 / 分批迁移 |
| `api/dial/*` | 1 | ~1 | cmd_core::dial | 待契约扩展 / 分批迁移 |
| `api/copy/*` | 1 | ~1 | cmd_clip | 待契约扩展 / 分批迁移 |
| `api/config/*` | 1 | ~1 | — | 待契约扩展 / 分批迁移 |
| `api/audit{qs}/*` | 1 | ~1 | — | 待契约扩展 / 分批迁移 |
| `api/audit/*` | 1 | ~1 | cmd_audit | 待契约扩展 / 分批迁移 |

## 已迁移第二批（§149，v1 设备动作）

| 旧路由 | v1 | 报告形态差异（明示） |
|---|---|---|
| `POST /api/peers/connect` | `POST /devices/{id}/connect` | PeerConnectReport→Device；already_connected 折入 online |
| `POST /api/peers/block` | `POST /devices/{id}/block` | PeerBlockReport→Device；already_blocked/终止会话细节折入 |
| `POST /api/peers/unblock` | `DELETE /devices/{id}/block` | PeerUnblockReport→Device；was_blocked 折入（404→友好文案） |
| `POST /api/peers/alias` | `PATCH /devices/{id}` `{alias}` | Device 回读 alias |
| `POST /api/peers/unpair` | `DELETE /devices/{id}` | 204 忘记 / 404 未配对（语义 1:1） |

## 已迁移第三批（§150，v1 读改写）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `POST /api/peers/tags/add` | `GET /devices` + `PATCH /devices/{id}` `{tags}` | 全量替换 → CLI 读改写；增删幂等/case-blind 同 `add_peer_tag`；1-32 字符校验客群两端一致 |
| `POST /api/peers/tags/remove` | 同上 | 同上；已不在列表的 tag 静默成功（与 legacy 一致） |

注：读改写窗口内外并发改动标签集会被最后一写覆盖（无 If-Match/ETag）——单用户 CLI 场景可接受，附录留档。

## 已迁移第四批（§151，契约扩展先行）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `POST /api/peers/disconnect` | `POST /devices/{id}/disconnect` | PeerDisconnectReport→Device；already_disconnected 折入 online（动作后回读 online=false 即已断） |
| `GET /api/peers/blocked` | `GET /devices` 过滤 `blocked==true` | BlockedPeersReport→行内过滤；block_reason 新契约字段（§151）；排序从插入序变 id 序 |

## 已迁移第五批（§152，链路质量域契约扩展）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `GET /api/peers/link` | `GET /link` | PeerLinkQualityDto 列表→LinkQuality（heartbeat+计数+links）；grade 由 enum 变字符串；samples_count 显式 u64 字段 |
| `GET /api/peers/heartbeat` | `GET /link/heartbeat` | 同值；§152 新 HeartbeatConfig 契约 |
| `POST /api/peers/heartbeat` | `PATCH /link/heartbeat` | GET 动词语义修正为 PATCH；空 patch 400 "patch sets no field" |
| `POST /api/peers/ping` | `POST /devices/{id}/ping` | DevicePingReport 契约（device_id/rtt_ms/path/addr）；auto-target（单在线 peer 自动选）移到 CLI 侧 |
| `POST /api/peers/link/probe` | `POST /devices/{id}/probe` | LinkProbeReport 契约；grade 字符串化；count/interval_ms 可选参数同语义 |

## 已迁移第六批（§153-§154，诊断遥测 + 拨号契约扩展）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `GET /api/metrics`（+`/api/traffic`） | `GET /diagnostics/metrics` | TrafficTelemetry 逐字段镜像 TrafficReport；legacy `ok:true` 折掉（200 即 ok） |
| `POST /api/metrics/reset`（+`/api/traffic/reset`） | `PATCH /diagnostics/metrics` | POST 动作→幂等 PATCH；reset 折入同一 GET 回读形状 |
| `POST /api/dial` | `POST /pairing/dial` | DialRequest/DialReport 契约；pin 先设后拨；400 透传 legacy 文案（不可达/坏 pin/缺 target） |

## 已定案的特殊项

- ~~`POST /api/dial`~~：§154 迁 `POST /devices`→`/pairing/dial`（DialRequest/DialReport 契约；pin 先设后拨，400 透传 legacy 文案）。已迁移第六批。
- ~~`/api/log/*`~~：§155 删除（v1 `GET /diagnostics/logs`、`PATCH /diagnostics/log-level` 同语义承接；消费面核查 CLI/前端/tray 全零后删除）。
- ~~`/api/metrics`（+`/api/traffic`、`/reset`）~~：§153 迁 v1、§155 删除旧别名（`GET/PATCH /api/v1/diagnostics/metrics`）。
- ~~`POST /api/dial`~~：§154 迁 v1、§155 删除旧别名（`POST /api/v1/pairing/dial`）。
- `tray.rs` 内的 `tray_keepalive`/SSE 使用 v1，不在下线范围。
- 判定来源：调用方 UA 分类（cli/curl/browser/android），删除每组别名前先查 diagnostics 快照确认零命中。

## 已迁移第七批（§156a，profiles/caps 域 + Patch<T> 三态契约）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `GET /api/peers/profiles` | `GET /profiles`（tag/group/type 过滤） | PeerProfileDto 契约；`ok` 折掉 |
| `GET /api/peers/profile` | `GET /profiles/{id}` | 单对象；未知 peer 404 |
| `POST /api/peers/profile` | `PATCH /profiles/{id}` | PATCH 语义修正；三态：值→Set、null→Clear、缺省→Leave（`Patch<T>` 枚举解 serde 双 Option 塌缩）；空补丁 400 |
| `POST /api/peers/group` | `PATCH /profiles/{id}` `{group}` | 同上单键 |
| `POST /api/peers/group/clear` | `PATCH /profiles/{id}` `{group:null}` | null→Clear |
| `GET/POST /api/caps` | `GET/PATCH /caps` | CapsUpdate{enable,disable} deny_unknown_fields；空 PATCH 400；LocalCaps 镜像 legacy 报告（`ok` 折掉） |
| `GET /api/peers/caps?peer=` | `GET /devices/{id}/caps` | 单 PeerCaps 对象（信封折掉）；未知 peer 404（legacy 为空数组） |
| `GET /api/peers/caps`（全量） | `GET /caps/peers` | `Vec<PeerCaps>` 裸数组；`ok`/`count` 折掉（count=数组长度） |

## 已迁移第八批（§156b，topology/routes/net 域）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `GET /api/topology/layout` | `GET /topology` | `ok`/`message` 折掉；HashMap<edge,neighbor> 折成平行 `neighbors`+`neighbor_edges` 向量（排序确定） |
| `GET /api/topology/peers` | `GET /topology/peers` | `ok`/`message` 折掉；per-peer 邻居 map 折为 `neighbors_count`；`age_ms`→`timestamp_ms`（CLI 现算） |
| `POST /api/topology/link` | `POST /topology/links` | 动词名词化；body 同形（edge/peer/peer_edge/offset）；edge 大小写不敏感 |
| `POST /api/topology/unlink` | `POST /topology/unlinks` | 同上；回读折为 `{edge}` |
| `POST /api/topology/config` | `PATCH /topology/config` | PATCH 语义修正；12 稀疏键同形；query 别名（resistance/delay/scale…）折掉 |
| `GET /api/topology/policy` | `GET /topology/policy` | `ok`/`message` 折掉；裸 policy 对象 |
| `POST /api/topology/policy` | `PATCH /topology/policy` | PATCH 语义修正；body-only（legacy query 参数变体折掉）；clear_filters 保留 |
| `POST /api/topology/sync` | `POST /topology/sync` | `ok`/`message`/重复 `broadcast_count` 折掉；留 `sync_requests_sent`+`broadcasts_sent` |
| `GET /api/routes` | `GET /routes` | `ok` 折掉；RouteEntryDto 逐字段镜像 |
| `POST /api/routes` | `POST /routes` | RouteSetRequest{peer,addr} deny_unknown_fields；device_id 别名折掉 |
| `DELETE /api/routes?peer=` | `DELETE /routes?peer=` | 同语义；404 镜像 resolve 失败；body 变体折掉 |
| `GET /api/net` | `GET /net` | `ok` 折掉；Network{interfaces,overlay} 镜像 |
| `POST /api/net/refresh` | `POST /net/refresh` | `ok`/`action`/`timestamp_ms` 折掉 |

## 已迁移第九批（§160，discovery/daemon 生命周期域）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `GET /api/discovery` | `GET /discovery` | `ok` 折掉；count=数组长度现算 |
| `POST /api/discovery/rescan` | `POST /discovery` | rescan 动词折进 method；`ok` 折掉，message 保留 |
| `POST /api/daemon/shutdown` | `POST /system/shutdown` | **Admin 级**（V1_ADMIN_PATHS 首个现存路由）；`ok`/`message` 折掉，留 `status:"shutting_down"` |
| `POST /api/daemon/restart` | `POST /system/restart` | **Admin 级**；`ok:true`/`message` 折掉，留 action/old_pid/new_pid/timestamp_ms；替换路径 400 / 进行中 409 |

## 已迁移第十批（§161，notify/reply/OTP + message 域）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `POST /api/notify` | `POST /notify` | `ok` 折掉，留 `peer`；0 连接 peer→409（Conflict 类）、多 peer 无名→400；app_id 缺省 com.example.sms 保留 |
| `GET /api/notify/candidates` | `GET /notify/candidates` | `ok` 折掉，count 现算；query peer/app_id/has_otp 同形 |
| `GET /api/notify/replies` | `GET /notify/replies` | `ok` 折掉，count 现算；no-content 规则保留（不回文本） |
| `POST /api/otp/get` | `POST /notify/otp` | `ok` 折掉，留 `{id,otp}` |
| `POST /api/copy` | `POST /notify/copies` | `ok:true` 折成 NotifyActionReport{ok} |
| `POST /api/insert` | `POST /notify/insertions` | 同上 |
| `POST /api/dismiss` | `POST /notify/dismissals` | 单条（id）与批量（peer/app_id/all）双形态折叠进一个 report（只填相关字段）；无目标 400 |
| `POST /api/notify/dismiss` | `POST /notify/dismissals` | 同上（旧双端点并一）；query 变体仍吃 |
| `POST /api/open` | `POST /notify/open` | `ok` 折掉，留 `url`；allowlist 拒绝→400（OpenRefused 类） |
| `POST /api/reply` | `POST /replies` | `ok:true` 折成 NotifyActionReport |
| `POST /api/reply/take` | `POST /replies/take` | `ok` 折掉，留 id/uid/app_id/text；unknown→404；文本唯一释放点不变 |
| `GET /api/notify/config` | `GET /notify/config` | `ok` 折掉；平铺 total_dispatched/suppressed 折进嵌套 totals{dispatched,suppressed} |
| `POST /api/notify/config` | `PATCH /notify/config` | PATCH 语义修正；body 稀疏 patch + query 兼容折叠（body 胜出）；响应同上 |
| `POST /api/notify/test` | `POST /notify/test` | `ok` 折掉；sent/suppressed/suppression_reason/message/title 保留；query 折叠进 body |
| `POST /api/message/send` | `POST /messages` | `ok` 折掉；单目标留 message（MessageEntry）；带 tag/group/type/all 时响应换 MessageDispatchReport（selector 折 as_label 字符串） |
| `POST /api/message/send-multi` | `POST /messages` | 并入同一端点；report 同上；`ok` 折掉 |
| `GET /api/message/history` | `GET /messages` | `ok` 折掉；query peer/limit/unread_only/mark_read 同形（body 变体仍吃） |
| `POST /api/message/clear` | `DELETE /messages?peer=` | 动词修正（DELETE）；peer 上 query；`ok` 折掉，留 target/cleared_count/message |

## 已迁移第十一批（§162，文件域：inbox/offers/transfers/outbox/storage/resume/policy）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `GET /api/transfers` | `GET /transfers` | 返回全量资源；id/active/terminal/direction 过滤移 CLI 客户端侧（数组上 retain 重算计数） |
| `POST /api/transfers/cancel` | `POST /transfers/{id}/cancel` | id 上路径；`ok` 折掉 |
| `POST /api/transfers/clear` | `DELETE /transfers` | 动词修正（DELETE）；cleared_count 由 CLI 前后快照差值算 |
| `GET /api/file/inbox` | `GET /inbox` | `ok` 折掉；search/limit 服务端过滤（count 镜像 legacy：过滤后截断前） |
| `GET /api/file/inbox/download` | `GET /inbox/{name}/content` | 裸字节（非 JSON envelope）+ sniffed mime + Content-Disposition |
| `DELETE /api/file/inbox` | `DELETE /inbox?before=` | before 截止语义保留：早于 cutoff 的删，其余保留 |
| `DELETE /api/file/inbox/{name}` | `DELETE /inbox/{name}` | name percent-encoded；deleted/freed 由 CLI 快照差值算 |
| `POST /api/file/inbox/open` | `POST /inbox` | 目录 reveal（无 name）；**204→200** 带 InboxOpenReport |
| `POST /api/file/inbox/{name}/open` | `POST /inbox/{name}/open` | 204→200 带 report（name=Some） |
| `GET /api/file/offers` | `GET /offers` | `ok` 折掉，count 折自数组；id/peer 过滤同形（id 过滤留 CLI 侧） |
| `POST /api/file/accept` | `POST /offers/accept` | id 单条 / peer+all 批量双形态折叠进 FileDecisionReport；未知 id→404 |
| `POST /api/file/reject` | `POST /offers/reject` | 同上 |
| `POST /api/file/send` | `POST /transfers` | path 或 data_base64 二选一；FILE_MAX 64GiB 前置；0 连接 peer→409、显式 peer 未连接→404 |
| `POST /api/file/send-multi` | `POST /transfers/multi` | TargetSelector 群发（tag/group/device_type/all）；selector 折 as_label 字符串 |
| `POST /api/file/send-batch` | `POST /transfers/batch` | paths+items 混合；per-file status 保留 |
| `GET /api/file/outbox` | `GET /outbox` | status 解析用别名表（done/sending/…），未知→400 |
| `POST /api/file/outbox/clear` | `DELETE /outbox?peer=&status=` | 动词修正（DELETE）；过滤条件上 query |
| `POST /api/file/outbox/resend` | `POST /outbox/{id}/resend` | id 上路径；留 original_id/new_transfer_id |
| `GET /api/file/storage` | `GET /storage` | `ok` 折掉；快照字段同形 |
| `POST /api/file/storage/clean` | `POST /storage/clean` | orphaned_only 缺省 true（legacy 对齐）；空体=缺省 |
| `GET /api/file/resume` | `GET /resume` | `ok` 折掉；progress_percent f64；source_alias null 显式 |
| `GET /api/file/policy/auto-accept` | `GET /policy/auto-accept` | 平铺资源（legacy 嵌套 policy 折掉）；cumulative_auto_accepted_count→auto_accepted_count（会话计数） |
| `POST /api/file/policy/auto-accept` | `PATCH /policy/auto-accept` | PATCH 语义修正；三态 patch（absent=不动 / null=清 / 值=设）；空 patch→400 |

## 已迁移第十二批（§163，clip + audit 域）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `GET /api/clip/status` | `GET /clip/status` | `ok` 折掉；其余字段同形（bytes usize→number） |
| `GET /api/clip/history` | `GET /clip/history?limit=` | `ok` 折掉；query body 变体折掉（只留 query） |
| `POST /api/clip/clear` | `DELETE /clip` | 动词修正（DELETE）；`ok` 折掉 |
| `GET /api/clip` | `GET /clip` | `ok` 折掉；headless 仍 200 + clipboard_available:false + reason（不 500） |
| `POST /api/clip/sync` | `POST /clip/sync` | `ok` 折掉；显式 peer 必须已连接（否则 400）；text 超 1 MiB→400 |
| `GET /api/clip/policy` | `GET /clip/policy` | `ok` 折掉；**嵌套 policy 展平**进资源（mode/sensitive_guard/... 直读） |
| `POST /api/clip/policy` | `PATCH /clip/policy` | PATCH 语义修正；body 稀疏 + query 兼容折叠（body 胜出）；auto_clear 0 仍含清零义 |
| `POST /api/clip/auto-sync/poll` | `POST /clip/poll` | `ok` 折掉；嵌套 stats 展平进 policy；dispatched_peers 保持数字 |
| `GET /api/audit` | `GET /audit` | `ok` 折掉；query category/severity/limit/since + body 镜像（body 胜出）；坏枚举→400 |
| `POST /api/audit/clear` | `DELETE /audit` | 动词修正（DELETE）；`ok` 折掉 |

## 已迁移第十三批（§164，action + bench + recommendation 域）

| 旧路由 | v1 | 语义差异（明示） |
|---|---|---|
| `GET /api/action/local` | `GET /action/local` | `ok`/`message` 折掉；字段同形（timeout_secs/enabled 直读） |
| `POST /api/action/local` | `POST /action/local` | strict body（deny_unknown_fields）；timeout 缺省 10、enabled 缺省 true |
| `DELETE /api/action/local?id=` | `DELETE /action/local/{id}` | id 上路径；未找到→400（CoreError InvalidPayload 镜像） |
| `GET /api/action/remote?peer=` | `GET /action/remote?peer=` | `ok`/`message` 折掉；缺 peer→400、未知 peer→404 |
| `POST /api/action/remote/refresh?peer=` | `POST /action/remote/refresh` | body `{peer}`（query 兼容）；显式 token |
| `POST /api/action/trigger` | `POST /action/trigger` | body strict；query 兼容 id 别名；显式 token |
| `GET /api/action/history` | `GET /action/history` | `ok`/`message` 折掉；运行时帧无 direction/peer 字段（CLI 兜底列保留空） |
| `GET /api/action/policy` | `GET /action/policy` | `ok` 折掉；**嵌套 policy 展平**；mode 折 as_str 字符串 |
| `POST /api/action/policy` | `PATCH /action/policy` | PATCH 语义修正；body 稀疏 + query 兼容（body 胜出）；legacy query 对 allow-list 是追加去重；max_timeout 过 clamp |
| `GET /api/bench` | `GET /bench` | `ok` 折掉；session/last_report null 显式 |
| `POST /api/bench/start` | `POST /bench/start` | direction lenient（pull/download/rx/recv）；未知 peer→404；显式 token |
| `POST /api/bench/stop` | `POST /bench/stop` | 无活动会话仍 200 决策报告；显式 token |
| `GET /api/bench/policy` | `GET /bench/policy` | `ok` 折掉；**嵌套 policy 展平** |
| `POST /api/bench/policy` | `PATCH /bench/policy` | PATCH 语义修正；body 稀疏 + query 兼容（body 胜出）；legacy query 对 allow-list 是追加去重 |
| `GET /api/recommendation?peer=` | `GET /recommendation?peer=` | `ok` 折掉；tier DTO 展开（codec 折 as_str 字符串）；未知 peer→404 |
| `GET /api/recommendation` | `GET /recommendation` | 聚合视图（peers 数组 + timestamp） |

## 已迁移契约层（§165 第一批，11 小域 DTO-only）

> 本批只落 tetherly-api 契约（93 DTO），端点/CLI 未切——下表各行待后续批 §165 续（标 ⏳）。域与 legacy 路径映射不变。

## 已迁移契约层（§165 第一批，11 小域 DTO-only）

> §165（一）只落 tetherly-api 契约（93 DTO）；§166（二）端点 ×49 + CLI ×57 已全部切换（下表 ✅）。/legacy/ 别名与 SPA legacy 路径未动，归 §167 统一删除。

| 域 | legacy 路径 | v1 端点 | CLI 调用点 |
|---|---|---|---|
| power | api/power/*、api/battery/*、api/storage/* | GET /battery、GET /battery/all、POST /battery/policy | cmd_power 8 ✅ |
| media | api/media/* | GET /media/state、GET|POST /media、POST /media/command(/multi)、GET|PATCH /media/policy | cmd_media 4 ✅ |
| ancs | api/ancs/* | GET /ancs、POST connect/disconnect、POST|DELETE /ancs/allow、POST rules/clear | cmd_ancs 6 ✅ |
| input | api/input/* | GET /input、/trace、/tunnel、POST start/stop/tunnel/attach/tunnel/release、GET|PATCH /policy | cmd_input 8 ✅ |
| handoff | api/handoff/* | POST /send(/multi)、GET|DELETE /inbox、POST inbox/open|dismiss、GET|PATCH /policy | cmd_handoff 7 ✅ |
| focus | api/focus/* | GET|POST /focus/state、GET /peers、POST /sync、GET|PATCH /policy | cmd_focus 7 ✅ |
| camera | api/camera/* | GET /camera、POST attach/detach、GET|PATCH /policy | cmd_camera 6 ✅ |
| mic | api/mic/* | GET /mic、POST attach/detach、GET|PATCH /policy | cmd_mic 6 ✅ |
| audio | api/audio/* | GET /audio、POST attach/detach、GET|PATCH /policy | cmd_audio 6 ✅ |
| locate | api/locate/* | GET /locate/status、POST ring/stop、GET|PATCH /policy | cmd_locate 5 ✅ |
| display | api/display/* | GET /display/virtual、POST attach/detach、GET|PATCH /policy | cmd_display 5 ✅ |
| screen | api/screen/*（102 路由） | GET /screen、allow/revoke/start/stop/keyframe、displays/switch、privacy、recording×4、playback×4、snapshot、laser、annotation、whiteboard×6、watermark×3 + 32 模式组 GET|PATCH|DELETE | cmd_screen_* 151 ✅ |

> §168（二）补充：`/legacy` 与 `/legacy/*`（旧单文件控制台 + `ui/index.html`）已整体删除；旧控制台的 `/api/*` 调用面成为不可达死代码，归 §169 删除。screen 域契约 ×113 + 端点 ×~150 + CLI ×151 详见 DEVELOPMENT.md §168。

> 契约修正：`ActiveDisplaySessionDto.mode` 由 String 修为 `DisplayModeDto`（runtime 实字段为结构体，CLI 打印器读 `mode["width"]`）。

> §169（M5 收敛）：legacy `/api/*` 路由面已整体删除——uihttp.rs 429 arms + 27 个 api_*.rs 模块 + requires_token legacy 特权表 + 107 个纯 legacy 集成测试，净 -33.8k 行。本表上方批次表保留作迁移史存档；`hub.legacy` 命中计数与 diagnostics 的 legacy_routes 观测字段保留至外部客户端迁移完成。
