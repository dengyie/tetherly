# ADR-139 独立控制客户端：daemon 即后端、v1 契约、事件流与 Tauri 桌面壳

| 项 | 值 |
|---|---|
| 状态 | **提议（2026-11-15）**——调研完成，等待评审后按 M0–M5 实施 |
| 日期 | 2026-11-15 |
| 关联 | ADR-134（托盘）、ADR-135（签名身份）、ADR-137（托盘面板/拖放）、ADR-138（远程桌面） |
| 详细设计 | [docs/CONTROL_CLIENT_DEVELOPMENT.md](../CONTROL_CLIENT_DEVELOPMENT.md) |

## 背景

`ui/index.html` 是一个 10,226 行的单文件控制台：约 30 个区块纵向堆叠，用 50 个 `setInterval` 每 2–3 秒轮询一次，调用约 555 个带大量别名的 `/api/*` 路由。托盘面板每秒拉取一次数据，托盘菜单上的状态文字是写死的。daemon 已经有 96 种 `UiEvent` 和 `/api/events` SSE 推送，但前端几乎没有用上。

## 调研

对比了 Deskflow、Input Leap/Barrier、PowerToys MWB、RustDesk、LocalSend、KDE Connect/GSConnect、Tailscale 七个项目，对照表见设计文档 §2。得出的主要原则：
- 用推送，不用轮询，也不解析日志；
- GUI 不管理 daemon 的生命周期；
- 设备优先、能力按设备分组；
- 配对默认"拒绝"，并排比对验证码；
- 事件流采用 Tailscale IPN bus 式的快照加增量。

## 决策

1. **daemon 作为唯一后端。** 新增有版本的 `/api/v1`（约 45 个资源化端点，统一错误外壳，读、写、admin 三级鉴权），并冻结旧 `/api/*`。
2. **契约只有一份来源**：新 crate `tetherly-api`。用 `ts-rs` 生成 TS 类型，用 `schemars` 生成 JSON Schema 再生成 Kotlin 类型，CI 检查生成结果有没有漂移。
3. **新增 `GET /api/v1/events`**：先发快照，再发增量，每条事件带 `seq`，支持 `Last-Event-ID` 续传（环形缓冲用完时重新发快照）、按主题过滤、服务端节流。由纯函数 `project()` 把 `UiEvent` 投影成约 25 种对外事件。
4. **桌面客户端用 Tauri 2 + Svelte 5 + TypeScript**，以托盘为入口。token 只放在 Rust 侧，所有请求经 `invoke` 代理。客户端不拉起 daemon，也不把 daemon 作为 sidecar 打包。
5. **同一份 SPA 也由 daemon 托管**，作为浏览器兜底入口。业务代码只有一份，通过 `Transport` 接口适配两种环境。
6. **Windows 客户端 token**：SYSTEM 身份的 daemon 为交互用户签发 `%LOCALAPPDATA%\Tetherly\client.token`（ACL 只允许该用户，权限不含 admin）。
7. **信息架构**：概览、设备、布局、传输、动态、设置、诊断。
8. **Android** 保留 Kotlin 应用，统一信息架构，类型从契约生成。

## 后果

- 正面：
  - 空闲时零轮询；
  - 前后端可以独立迭代；
  - 前端可以做单元测试和端到端测试；
  - 路由最终收敛到约 60 个；
  - 托盘显示真实状态。
- 负面：
  - 新增前端工具链和 Tauri 依赖；
  - 迁移期间要同时维护门面和旧路由；
  - Windows 需要新的 token 签发路径。
- 迁移：按 M0（契约）→ M1（事件流）→ M2（门面 + SPA）→ M3（Tauri）→ M4（切换）→ M5（收敛）推进。每个阶段都能单独发布；旧控制台在 M4 前一直可用。
