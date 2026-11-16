# 旧 `/api/*` 路由映射表（ADR-139 §9-2 附录）

> 生成于 2026-11-17（v0.2.60 发版轮）。来源：`grep '/api/'` 于 `bin/tetherly` 与 `platform/tray.rs` 的静态调用点计数。
> 运行时命中按调用方分类见 `GET /api/v1/diagnostics` 的 `legacy_routes`（§129）。
> Android 无 HTTP 层（JNI 直连），不受本表约束；旧控制台 `/legacy/` 整体随 M5 收敛删除。

**静态调用点：239 个路由 / ~355 处调用。已迁 v1：6 路由 / 8 处（§142–§146 六命令）。**

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
| `api/peers/*` | 18 | ~24 | cmd_peers（写操作） | 待契约扩展 / 分批迁移 |
| `api/topology/*` | 7 | ~8 | cmd_topology | 待契约扩展 / 分批迁移 |
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
| `api/net/*` | 2 | ~2 | cmd_peers | 待契约扩展 / 分批迁移 |
| `api/metrics/*` | 2 | ~2 | cmd_core（流量） | 待契约扩展 / 分批迁移 |
| `api/discovery/*` | 2 | ~2 | cmd_core | 待契约扩展 / 分批迁移 |
| `api/daemon/*` | 2 | ~2 | cmd_core | 待契约扩展 / 分批迁移 |
| `api/routes/*` | 1 | ~3 | cmd_peers | 待契约扩展 / 分批迁移 |
| `api/state/*` | 1 | ~2 | — | 待契约扩展 / 分批迁移 |
| `api/recommendation/*` | 1 | ~2 | cmd_notify | 待契约扩展 / 分批迁移 |
| `api/health/*` | 1 | ~2 | — | 待契约扩展 / 分批迁移 |
| `api/caps/*` | 1 | ~2 | cmd_core | 待契约扩展 / 分批迁移 |
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

## 剩余待迁（peers 写域）

| 旧路由 | v1 状态 |
|---|---|
| `POST /api/peers/tags/*` | `PATCH /devices/{id}` `{tags}` 全量替换——CLI 语义是加/删单条，需读改写，§150 |
| `GET /api/peers/blocked` | Device DTO 无 block reason 字段——待契约扩展 |
| `POST /api/peers/disconnect` | v1 无断开动作——待契约扩展 |
| `GET /api/peers/ping`、`/heartbeat`、`/link`、`/link/probe` | 链路质量域 → diagnostics，待契约扩展 |

## 已定案的特殊项

- `POST /api/dial`：首配对拨号（addr+PIN），v1 无对应资源——等 `POST /pairing/dial` 契约扩展（§146 定案）。
- `/api/log/*`、`/api/metrics`：v1 已有 `GET /diagnostics/logs`、`PATCH /diagnostics/log-level`，CLI `tetherly logs/metrics` 迁移候选。
- `tray.rs` 内的 `tray_keepalive`/SSE 使用 v1，不在下线范围。
- 判定来源：调用方 UA 分类（cli/curl/browser/android），删除每组别名前先查 diagnostics 快照确认零命中。
