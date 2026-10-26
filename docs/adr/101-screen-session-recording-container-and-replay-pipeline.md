# ADR-101 屏幕串流会话录制容器与回放流水线（Screen Streaming Session Recording Container & Replay Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-27，v2.15 / v0.1.37）**——建立屏幕串流会话录制（TREC 自定义容器格式）与回放读回流水线：定义 27 字节定长容器头 `SCREEN_RECORDING_MAGIC = *b"TREC"`（`proto(1) + width(4) + height(4) + format(1) + codec(1) + stride(4) + started_us(8)`）与逐帧记录（`seq(8) + pts_us(8) + len(4) + payload`）；实现 `ScreenRecordingMeta`（从首帧自动推导）、`ScreenRecordingWriter`（惰性写头，BufWriter 落盘）、`ScreenRecordingReader`（逐记录读回 + `decode_frame` 还原为 `ScreenFrame`）；在屏幕控制通道引入录制告知信令 `ControlMsg::SetRecordingState { active }`（Envelope Tag = 9，14 字节定长），收端开始/停止录制时向宿主机广播，宿主机 `peer_recording` 状态透视（录制知情权）；录制默认落盘 `<data_dir>/recordings/rec-<unix_ms>.trec`；回环 REST API `POST /api/screen/recording/start`、`POST /api/screen/recording/stop`、`GET /api/screen/recording` 与 `GET /api/screen/recording/replay`（容器读回校验）；`ScreenStatusReport` 扩充 `recording_active`、`peer_recording`、`recording_frames`；实现 100% 严格全同构双端 CLI（`screen recording start/stop/status/inspect`）；新增核心单元测试、节点级 REST 测试与端到端自动化集成测试 `m5_25_screen_session_recording_container_and_replay_pipeline` |
| 日期 | 2026-10-27 |
| 关联 | ADR-077「屏幕流链路吞吐压测与带宽画像」、ADR-087「屏幕串流接收端动态抖动缓冲与平滑重放流水线」、ADR-097「屏幕串流多显示器枚举与虚拟显示器流复用」、ADR-099「屏幕串流微秒级端到端延迟拆解与多阶段逐帧分析器」、ADR-100「屏幕串流瓶颈驱动的自主调优闭环流水线」 |

---

## 动机

1. **远程协助会话缺少取证与回放能力**：远程运维、跨设备协作与技术支持场景中，被控端呈现的内容瞬息万变，访客端缺少「把整场会话录制下来事后回看/取证/复盘」的能力；事后只能靠记忆或截图拼凑。
2. **录制知情权（Recording Disclosure）**：对被控端而言，"我的屏幕正在被对端录制" 是与隐私模式（ADR-098）同等重要的知情信息。录制必须在协议层可感知：收端开始/停止录制时通过控制通道向宿主机广播，宿主机状态面板可见，杜绝静默录制。
3. **格式自主可控**：引入外部容器（MP4/MKV）会引入重依赖与专利编码器约束（违背 ADR-006 的许可证边界）。本项目的屏幕帧本就是自描述的 `ScreenFrame` 线协议载荷（TMV1 头 + codec_tag 分派），录制容器只需最小化封装「帧线协议载荷 + 呈现时间戳」，即可无损保存会话并可逐帧还原。
4. **双端 CLI 100% 同构与控制面闭环**：依据 Tetherly 核心工程规范，录制启停、状态透视与容器读回必须在本地回环 REST API 与 `tetherly` / `tetherly-cli` 双端命令行中保持 100% 字节级同构。

---

## 决定

### 1. TREC 容器格式（`SCREEN_RECORDING_MAGIC = *b"TREC"`，`proto = 0x01`）

- **容器头（27 字节定长）**：`magic(4: TREC)` + `proto(1)` + `width(4, 大端)` + `height(4)` + `format_tag(1)` + `codec_tag(1)` + `stride(4)` + `started_at_us(8)`；
- **帧记录**：`seq(8)` + `pts_us(8)` + `len(4)` + `payload(len 字节)`；`payload` 为该帧的 `ScreenFrame::encode_payload()` 线协议字节（TMV1 头 + 像素/压缩体），容器层零转码、零重编码；
- **惰性写头**：容器头在首帧到达时以实际帧的 rect/format/codec 推导写出（`ScreenRecordingMeta::from_frame`），避免「先开录但分辨率未知」的元数据空窗。

### 2. 录制告知信令（`TYPE_SCREEN_CONTROL = 0x0502`，14 字节定长）

- **Tag = 9 `ControlMsg::SetRecordingState { active: bool }`**：低 8 位承载 active 标志；收端经 REST/CLI 开始或停止录制时向宿主机广播，宿主机原子更新 `screen_peer_recording` 并在状态报告与 `screen status` 中透视；断连自动随会话生命周期清理语义（`on_peer_gone` 复位）。

### 3. 落盘与回放

- **默认路径**：`<data_dir>/recordings/rec-<unix_ms>.trec`；自定义路径必须绝对化后落入 `data_dir` 白名单之外亦可（本地面板自用），但禁止目录穿越符号链接与空路径（复用 `UnsafeFileName`/`OpenRefused` 语义）；
- **回放读回（Replay Inspection）**：`GET /api/screen/recording/replay?path=...&limit=...` 打开容器、校验 magic/proto、读回容器头元数据与至多 `limit`（默认 32、上限 512）条帧记录摘要（seq/pts/len），供 CI 与运维做录制完整性校验；全量帧还原由 `ScreenRecordingReader::decode_frame` 提供（还原为标准 `ScreenFrame`，可直接送 `ScreenSink` 或重新编码）。

### 4. REST API 设计

在 `tetherly-node` 本地回环接口（默认 `127.0.0.1:45716`）提供：
- **`POST /api/screen/recording/start`**：body/query 可选 `path`；开始收端录制（若屏幕会话在线则向宿主机发送 `SetRecordingState { true }`）；重复开始返回 400；
- **`POST /api/screen/recording/stop`**：停止录制、flush 落盘并返回终态报告；
- **`GET /api/screen/recording`**：当前录制状态报告（`ScreenRecordingReport`：active/path/frames/bytes/duration/last_seq/peer_recording/meta）；
- **`GET /api/screen/recording/replay`**：容器读回校验报告。

### 5. 双端 CLI 100% 同构（`tetherly` & `tetherly-cli`）

```bash
# 开始录制（默认路径或 --out 自定义）
tetherly screen recording start --out /tmp/session.trec
# 停止录制并查看终态统计
tetherly screen recording stop
# 状态透视（本端录制 + 对端录制告知）
tetherly screen recording
tetherly screen recording status
# 容器读回校验
tetherly screen recording inspect /tmp/session.trec --limit 16
```

---

## 结论与工程影响

- **会话可取证**：远程协助全程可无损录制与逐帧回放，容器零转码、格式自主可控，不引入任何新依赖与专利编码器；
- **录制知情权入协议**：`SetRecordingState` 信令让被控端对「正在被录制」有协议级感知，补齐远程桌面信任模型；
- **100% 同构与健全测试**：双端 CLI、回环 HTTP API、核心单元测试与端到端集成测试 `m5_25` 全面落地通过。
