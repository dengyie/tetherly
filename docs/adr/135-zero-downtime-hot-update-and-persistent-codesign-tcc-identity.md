# ADR-135 无感热更新重载、断线秒级自愈与 macOS TCC 永久代码签名 (Zero-Downtime Hot Update, Sub-Second Resumption, and macOS TCC Persistent Code Signing)

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-11-03，v2.49 / v0.1.71）**——macOS 永久 Designated Requirement 代码签名打包流水线（`/Applications/Tetherly.app`）；输入控制会话 2.5 秒优雅断线保持宽限期与环形缓冲区（容量 64）；重连即时恢复对端焦点与相对光标定位（`resume_remote()`）；Windows NTFS 原地重命名热替换；`/api/daemon/restart` 与 `tetherly restart` 热重启命令全链路落地。 |
| 日期 | 2026-11-03 |
| 关联 | ADR-071（输入通道安全策略）、ADR-072（屏幕边缘漫游）、ADR-132（macOS 原生键鼠与重映射引擎）、ADR-134（跨平台桌面系统托盘与零扰民权限治理） |

---

## 动机与背景

在生产环境双机常态化运行中，更新、编译新版本或重启守护进程时面临三大核心痛点：
1. **macOS 辅助功能（Accessibility / TCC）权限重编译后失效**：
   - 默认的 Ad-hoc 签名（`codesign -s -`）在每次重新编译二进制时会生成全新的代码目录哈希（CDHash）。
   - macOS TCC 安全子系统以 CDHash 认定程序身份，重新编译后认定为新程序，静默撤销辅助功能权限，导致键盘鼠标全局钩子失效，漫游越界阻断，需要频繁打开系统偏好设置重新授权。
2. **连接中断光标强制复位与重连顿挫**：
   - 当 TCP 45719 出现短暂网络抖动、进程重启或重新拉起时，控制端（Mac）旧逻辑会立即触发 `force_local()`，并将光标强制拉回本地屏幕中心。
   - 重连后，受控端（Windows）新建的 `InputClient` 初始状态为 `focused: false`，在未收到显式 `Enter` 帧之前会丢弃后续所有鼠标位移帧，导致重连时光标无法无缝继续移动。
3. **Windows 生产环境文件锁与多任务保护**：
   - Windows 阻止直接覆盖或删除正在运行的 `tetherly.exe`（产生 `ERROR_SHARING_VIOLATION`）。
   - Windows 工作站同时承载挖矿、代理、探针等生产任务，更新时必须针对 Tetherly 精确 PID 操作，严禁全局进程误杀。

---

## 核心架构设计与技术实现

### 1. macOS 永久代码签名与 Designated Requirement 规则 (`scripts/package-macos-app.sh`)

macOS LaunchServices 与 TCC 允许通过显式 **指定规则（Designated Requirement / DR）** 将权限授权永久绑定至 Bundle Identifier：
```bash
BUNDLE_ID="dev.tetherly.tetherly"
codesign -f -s - \
    --identifier "$BUNDLE_ID" \
    -r="designated => identifier \"$BUNDLE_ID\"" \
    "$APP_DIR/Contents/MacOS/Tetherly"

codesign -f -s - \
    --identifier "$BUNDLE_ID" \
    -r="designated => identifier \"$BUNDLE_ID\"" \
    "$APP_DIR"
```
- **一次授权，永久生效**：无论内部二进制重新编译多少次，macOS TCC 依据 DR 判定其均为合法的 `dev.tetherly.tetherly`，系统不再提示授权弹窗，权限持久保留。
- **打包流水线自动化**：`scripts/package-macos-app.sh` 自动构建 Release 产物、组装 App Bundle、写入 `Info.plist` 并执行深度签名与权限自检。

### 2. 协议与运行时断线秒级自愈与焦点恢复 (Zero-Downtime Input Resumption)

- **控制端 2.5 秒优雅断线保持宽限期 (`crates/tetherly-node/src/runtime/input.rs`)**：
  当底层输入 TCP 连接断开时，若当前处于跨屏控制态（`CursorSeat::Remote`）：
  1. 不立即执行 `force_local()`，不把光标拉回本地，保持 OS 物理锁边；
  2. 记录 `input_reconnect_deadline = Instant::now() + 2.5s`；
  3. 将断线期间的鼠标键盘输入事件暂存入轻量环形缓冲区（`input_buffered_frames: Mutex<VecDeque<InnerFrame>>`，容量 64）；
  4. 启动异步指数退避快速重连循环（30ms, 60ms, 120ms...）。
- **受控端焦点极速接管 (`crates/tetherly-core/src/input.rs`)**：
  - 在重连握手成功后，控制端调用 `engine.resume_remote()`，优先向受控端下发 `InputKind::Enter` 与 `InputKind::Move(last_x, last_y)`，强制激活受控端焦点并同步物理坐标；
  - 随后立即清空并补发暂存环形缓冲区中的数据包，保证跨屏操作连贯无断裂感；
  - 仅当 2.5 秒超时仍未恢复时，才触发安全降级复位本地，杜绝光标失控。
- **编译器异步递归解除**：
  将 `input_out` 由 `tokio::sync::Mutex` 转为标准库 `std::sync::Mutex`，使得 `attach_input_server` 成为同步函数，解除了其与重连协程之间的相互调用类型推导环（解决 Rust `E0391` 编译周期错误）。

### 3. Windows NTFS 原地重命名热替换与优雅重启 (`crates/tetherly-node/src/runtime/lifecycle.rs`)

- **NTFS In-Use Rename 机制**：
  Windows NTFS 虽然禁止覆写正在运行的可执行文件，但**完全允许对其原地重命名**：
  1. 将正在运行的 `tetherly.exe` 原地重命名为 `tetherly.exe.old.<timestamp>`；
  2. 将新编译或下载的新二进制文件复制/写入原位置 `tetherly.exe`；
  3. 启动新进程，完整继承启动参数 `std::env::args_os().skip(1)`；
  4. 守护进程向调用方返回 `DaemonRestartReport`（包含 `old_pid` 与 `new_pid`）；
  5. 延迟 200ms 后旧进程优雅释放端口并退出，新进程无缝接管；
  6. 节点每次启动时自动扫描并清理历史 `.old` 文件。
- **管理控制面支持**：
  - 新增 `POST /api/daemon/restart` 端点（支持 `replacement_path` 与 `delay_ms` 参数）；
  - 新增 `tetherly restart` CLI 命令，实现远程/本地无感一键热重启。

---

## 验证与效果

1. **macOS 权限持久化验证**：
   打包并在 `/Applications/Tetherly.app` 授权一次后，连续多次重新编译、替换二进制并重新拉起，`/api/input/ensure` 始终返回 `accessibility_trusted: true`，系统完全不再弹窗。
2. **断线无感漫游恢复**：
   在光标位于受控端（Windows）桌面时手动触发重连或重启守护进程，光标保持在 Windows 对应位置，200ms 内瞬间重新响应鼠标移动与点击，未发生光标拉回本地的现象。
3. **生产机环境安全隔离**：
   热替换与重启过程仅作用于 Tetherly 自身所属 PID，两台生产主机上的常驻进程与后台工作流（包含挖矿、网络代理、监控探针等）平稳运行，零误杀、零性能抖动。
