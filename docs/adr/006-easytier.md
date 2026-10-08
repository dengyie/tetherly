# ADR-006 EasyTier 侧车（许可证核实）

| 项 | 值 |
|---|---|
| 状态 | 已核实，侧车为默认 |
| 日期 | 2026-10-09 |
| 证据 | https://github.com/EasyTier/EasyTier/blob/main/LICENSE |

## 事实

EasyTier 主仓库 `LICENSE` 为 **GNU LGPL v3**。workspace 含 `easytier-core`、`easytier-ffi`、`easytier-android-jni`、`easytier-ios`。默认虚网示例为 **`10.144.144.0/24`**，监听 TCP/UDP **11010**（另有 WS/WSS/WG 端口）。RPC portal 常见为 `15888`。

LGPL-3 允许「应用」链接「库」，但静态链进 Apache-2.0/MIT 的 Tetherly 需要提供可重链的目标文件或改用动态库，并附 GPL/LGPL 文本。iOS 静态链 LGPL 尤其麻烦。

## 决定

1. Tetherly **不**把 `easytier` 放进 `Cargo.toml`，直到另有书面决定。
2. Phase 2 用户自装官方 EasyTier；Tetherly 只探测虚网接口并对 **单播** 对端 IP:45717。
3. 不假设 EasyTier TUN 转发组播；**禁止把跨网发现建立在 mDNS 上**。
4. 嵌入 FFI 若将来要做：单独修订本 ADR，列出动态链接方案与版权声明 UI。
