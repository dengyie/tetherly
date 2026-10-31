# ADR-103 屏幕串流实时快照提取、零依赖标准图像导出与录制容器单帧解析流水线（Screen Streaming Live Snapshot Extraction, Zero-Dependency Image Export & Container Frame Inspection Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-27，v2.17 / v0.1.39）**——建立屏幕串流多源实时快照提取、零依赖标准图像导出与录制容器单帧精准解析流水线：(1) **零依赖 Top-Down 32-bit BMP 编码器**（`crates/tetherly-core/src/screen.rs`）：纯 Rust 构造 54 字节 Windows DIB 头部（负高度 `biHeight = -h`），与 `ScreenFrame` 的 `Bgra8` 像素阵列零内存重排、零色彩转码拼接输出标准 `.bmp`，支持 `Rgba8` 通道快速置换；(2) **TREC 容器单帧抽取器**（`ScreenRecordingReader::extract_frame`）：支持按帧序号 `target_seq` 或微秒时间戳 `target_pts_us` 遍历抽取，非目标帧经由 `SeekFrom::Current(len)` 零内存分配跳过，定位目标帧即时解码为标准 `ScreenFrame`；(3) **节点运行时快照管线**（`crates/tetherly-node/src/runtime.rs`）：`NodeShared` 挂载 `screen_last_frame`，在 playout/playback 弹帧路径 try-tap 维护最新可用帧，提供 `screen_snapshot` 与 `screen_recording_snapshot` 原语；严格守卫隐私模式（ADR-098）：若隐私模式激活，强制全黑遮罩（Blackout Mask）并将 `privacy_blanked = true`；防御 `..` 路径穿越；(4) **回环 REST 控制面**（`crates/tetherly-node/src/uihttp.rs`）：`GET /api/screen/snapshot`（支持 `format=json|bmp|raw` 与 `out=<path>`，`format=bmp` 直接返回 `Content-Type: image/bmp` 二进制流）与 `POST /api/screen/recording/snapshot`（按帧序号/时间戳精准抽帧）；`ScreenStatusReport` 扩充 `snapshot_count`；(5) **双端 CLI 100% 全同构**：`screen snapshot [--out] [--format] [--json]` 与 `screen recording snapshot <path> [--frame] [--pts] [--out] [--json]`；(6) **测试覆盖**：core 核心单元测试、uihttp REST 端点测试与 Phase 5 端到端自动化集成测试 `m5_27_screen_live_snapshot_and_recording_frame_extraction` 全链路验证。 |
| 日期 | 2026-10-27 |
| 关联 | ADR-006「许可证与自包含依赖边界」、ADR-098「屏幕隐私模式、黑屏遮罩与物理输入屏蔽安全流水线」、ADR-101「屏幕串流会话录制容器与回放流水线」、ADR-102「屏幕串流会话容器回放引擎与虚拟屏幕源馈送流水线」 |

---

## 动机

1. **实时画面取证与轻量告警缺乏标准出口**：外部监控系统、自动化测试回归、告警通知（如 TG/企业微信 Bot 巡检）以及 Web 仪表盘在需要截取远程桌面当前画面时，此前必须拉起重量级播放器或专用 viewer 进程，无法通过单条轻量命令或单个 HTTP 请求即时导出单帧静态图片（Snapshot）。
2. **录制容器（`.trec`）随机抽帧与视觉审计断层**：ADR-101/102 实现了屏幕会话录制与连续回放，但当安全审计或技术复盘人员需要对录制文件中的「第 $N$ 帧」或「某一具体时刻」的画面制作调查报告时，此前只能从头到尾重放整个视频，缺乏对 `.trec` 文件单帧随机定位（Seek-and-Extract）与离线导出的能力。
3. **零外部重依赖与零内存拷贝图像导出**：严格遵循 ADR-006，杜绝引入庞大的外部图形编码库（如 `image` 或 `png`）。标准 Windows/RFC BMP 格式仅需 54 字节头（14 字节 `BITMAPFILEHEADER` + 40 字节 `BITMAPINFOHEADER`），通过设置负高度 `biHeight = -h`（Top-Down 格式），可以直接与 `ScreenFrame` 的 `Bgra8` 像素内存完全对齐，实现**零内存重排、零色彩转码、零依赖落盘**。
4. **隐私保护与安全红线守卫**：当宿主机处于隐私模式（ADR-098 `screen_privacy` 启用）时，快照提取引擎必须严格遵循防偷窥原则，强制输出全黑遮罩（Blackout Mask），同时在元数据中标注 `privacy_blanked: true`，杜绝后台静默偷窥。
5. **双端 CLI 100% 同构与控制面闭环**：依据 Tetherly 一贯的设计契约，所有快照操作与状态透视必须通过 127.0.0.1 安全回环 REST API 提供，并在 `tetherly` 与 `tetherly-cli` 命令行工具中保持参数、行为与 `--json` 格式的 100% 严格同构。

---

## 决定

### 1. 核心图像编码与容器单帧抽取（`crates/tetherly-core/src/screen.rs`）

- **Top-Down 32-bit BMP 编码器**：
  - `ScreenFrame::to_bmp(&self) -> Result<Vec<u8>, CoreError>`：
    - 检验帧处于原始未压缩状态，像素字节长度与 `rect.w * rect.h * 4` 完全一致；
    - 预分配 $54 + \text{pixel\_bytes}$ 大小的缓冲区；
    - 写入 14 字节 `BITMAPFILEHEADER`（魔数 `b"BM"`, 文件总大小, 偏移量 54）；
    - 写入 40 字节 `BITMAPINFOHEADER`（宽 $W$, 负高 $-H$, planes=1, bpp=32, compression=0 BI_RGB, 72 DPI）；
    - 若为 `PixelFormat::Bgra8`，零拷贝直接将 `self.bytes` 追加至头部后；若为 `PixelFormat::Rgba8`，利用内置 `converted` 快速转换 R/B 通道后追加。
- **TREC 容器单帧抽取器**：
  - `ScreenRecordingReader::extract_frame(&mut self, target_seq: Option<u64>, target_pts_us: Option<u64>) -> Result<Option<ScreenFrame>, CoreError>`：
    - `rewind_to_start()` 重置到数据起始位置；
    - 逐条解析 20 字节 record 前缀；非匹配帧经 `SeekFrom::Current(len)` 快速跳过；
    - 命中 `target_seq` 或达到 `target_pts_us` 时，读取当前帧并 `decode_frame` 还原为 `ScreenFrame`。
- **快照报告模型**：
  - `ScreenSnapshotReport`：包含 `ok`, `source` ("live" | "recording" | "playback"), `frame_seq`, `pts_us`, `width`, `height`, `format` ("bmp" | "raw"), `bytes_len`, `privacy_blanked`, `path`, `data_base64`。

### 2. 节点运行时快照管线与安全控制（`crates/tetherly-node/src/runtime.rs`）

- **`NodeShared` 状态更新**：
  - 维护 `screen_last_frame: Mutex<Option<ScreenFrame>>` 与 `screen_snapshot_count: AtomicU64`；
  - 呈现流水线（playout / playback）呈现新帧时更新最新可用帧缓存。
- **快照控制方法**：
  - `screen_snapshot(&self, format: Option<String>, out: Option<String>) -> Result<ScreenSnapshotReport, CoreError>`：
    - 依次从 `screen_last_frame`、`screen_observer.last()`、`screen_source.grab()` 获取最新帧；
    - **隐私防御**：若宿主机处于隐私模式，像素阵列强制清零并标记 `privacy_blanked = true`；
    - 编码为 BMP；若指定输出路径，拒绝包含 `..` 的路径并安全落盘；
  - `screen_recording_snapshot(&self, path: &str, frame_seq: Option<u64>, pts_us: Option<u64>, out: Option<String>) -> Result<ScreenSnapshotReport, CoreError>`：
    - 检验文件路径与目标输出路径安全性；
    - 调用 `extract_frame` 提取帧并转换为 BMP 导出。

### 3. 轻量级回环控制面端点（`crates/tetherly-node/src/uihttp.rs`）

- `GET /api/screen/snapshot`：
  - 查询参数：`format=json|bmp|raw`（默认 `json`）、`out=<path>`；
  - `format == "bmp"` 时返回二进制 `image/bmp`，浏览器直出；`format == "json"` 返回结构化 `ScreenSnapshotReport`；
- `POST /api/screen/recording/snapshot`：
  - 接收 JSON `{ "path": "...", "frame": 5, "pts_us": 1000, "out": "...", "format": "bmp|json" }`；
  - 抽取并返回结果。

### 4. 双端 CLI 100% 全同构

- `screen snapshot [--out <path.bmp>] [--format <bmp|raw|json>] [--json] [--ui-port <port>]`
- `screen recording snapshot <path> [--frame <seq>] [--pts <us>] [--out <path.bmp>] [--json] [--ui-port <port>]`
- `screen status` 扩充展示快照统计指标 `Snapshots: <count>`。

---

## 影响与验证

1. **测试完整性**：
   - 核心单测：`crates/tetherly-core/src/screen.rs::screen_snapshot_bmp_encoding_and_recording_extraction`
   - 端点单测：`crates/tetherly-node/src/uihttp.rs::test_uihttp_screen_snapshot_endpoints`
   - 端到端集成测试：`tests/phase5.rs::m5_27_screen_live_snapshot_and_recording_frame_extraction`
2. **规范升级**：
   - Workspace 版本发布至 `0.1.39`。
   - `docs/DEVELOPMENT.md` 更新至 v2.17 并增补 § 52。
   - Obsidian 路由表与权威文档全量对齐。
