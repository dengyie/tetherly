# 旧 `/api/*` 路由映射表（ADR-139 §9-2 附录）

> 生成于 2026-11-17（v0.2.60 发版轮）。来源：`grep '/api/'` 于 `bin/tetherly` 与 `platform/tray.rs` 的静态调用点计数。
> 运行时命中按调用方分类见 `GET /api/v1/diagnostics` 的 `legacy_routes`（§129）。
> Android 无 HTTP 层（JNI 直连），不受本表约束；旧控制台 `/legacy/` 整体随 M5 收敛删除。

**静态调用点：239 个路由 / ~355 处调用。已迁 v1：§142–§146 六命令 + §149-§154 peers 域 + §156a profiles/caps 域 + §156b topology/routes/net 域（累计 30+ 旧路由）。**

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
| `api/action/*` | 6 | ~9 | cmd_action | 待契约扩展 / 分批迁移 |
| `api/notify/*` | 6 | ~7 | cmd_notify | 待契约扩展 / 分批迁移 |
| `api/ancs/*` | 6 | ~6 | cmd_ancs | 待契约扩展 / 分批迁移 |
| `api/handoff/*` | 5 | ~7 | cmd_handoff | 待契约扩展 / 分批迁移 |
| `api/mic/*` | 4 | ~6 | cmd_mic | 待契约扩展 / 分批迁移 |
| `api/focus/*` | 4 | ~7 | cmd_focus | 待契约扩展 / 分批迁移 |
| `api/camera/*` | 4 | ~7 | cmd_camera | 待契约扩展 / 分批迁移 |
| `api/bench/*` | 4 | ~6 | cmd_bench | 待契约扩展 / 分批迁移 |
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
| `api/recommendation/*` | 1 | ~2 | cmd_notify | 待契约扩展 / 分批迁移 |
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
