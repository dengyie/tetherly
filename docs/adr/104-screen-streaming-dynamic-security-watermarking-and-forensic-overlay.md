# ADR-104 屏幕串流动态安全水印、防摄录泄密溯源与取证叠加层流水线（Screen Streaming Dynamic Security Watermarking & Forensic Overlay Pipeline）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-27，v2.18 / v0.1.40）**——建立屏幕串流动态安全水印、防摄录泄密溯源与取证叠加层流水线：(1) **零依赖 8x16 点阵字模与定点数 Alpha 混合器**（`crates/tetherly-core/src/screen.rs`）：严格遵循 ADR-006，零重型字体库依赖内嵌 95 字符标准 ASCII 点阵字模（约 1.5 KB），定点整数算法 $C_{out} = (C_{fg} \times \alpha + C_{bg} \times (255 - \alpha) + 128) / 255$ 原生适配 `PixelFormat::Bgra8` 与 `Rgba8`；(2) **动态安全水印图样与配置模型**（`ScreenWatermarkConfig` / `ScreenWatermarkReport`）：支持对角交错平铺网格（`Grid`）、单点角标/居中（`Single`）与上下横幅（`Banner`），支持 `{peer}`, `{host}`, `{time}` 动态宏替换、不透明度 (0..=255)、RGB 色彩与 1x/2x 缩放；(3) **帧内就地打标原语**（`ScreenFrame::apply_watermark`）：零内存重排直接合成原始帧像素缓冲区；(4) **线协议信令 Tag 10**（`ControlMsg::SetWatermarkState { enabled, host_enforced }`）：14 字节定长信封，同步对端水印与宿主机强制锁状态；(5) **节点运行时安全管线与宿主机强制锁**（`crates/tetherly-node/src/runtime.rs`）：呈现流（playout/playback）与快照提取（`screen_snapshot`）自动叠加水印并累加遥测计数；访客端试图关闭宿主机强制水印时直接拒绝；(6) **回环 REST 控制面**（`crates/tetherly-node/src/uihttp.rs`）：`GET /api/screen/watermark`、`POST /api/screen/watermark`、`POST /api/screen/watermark/enable` 与 `POST /api/screen/watermark/disable`；(7) **双端 CLI 100% 全同构**：`screen watermark status/enable/disable/set`；(8) **测试与集成**：core 核心单元测试、uihttp REST 单测与 Phase 5 端到端自动化集成测试 `m5_28_screen_dynamic_security_watermarking_and_forensic_overlay` 全链路验证。 |
| 日期 | 2026-10-27 |
| 关联 | ADR-004「端到端 Noise 传输与对端主权原则」、ADR-006「许可证与自包含依赖边界」、ADR-098「屏幕隐私模式、黑屏遮罩与物理输入屏蔽安全流水线」、ADR-101「屏幕串流会话录制容器与回放流水线」、ADR-103「屏幕串流实时快照提取与录制容器单帧解析流水线」 |

---

## 动机

1. **防拍照/截图泄密与 DLP 安全合规缺失**：在企业远程桌面运维、外包技术协助与远程办公场景中，桌面画面包含核心业务代码、内部客户数据、敏感凭证或未公开设计。访客端可能通过操作系统截图工具、外挂采集卡或手机拍摄物理显示器屏幕泄露数据。缺乏身份溯源机制时，一旦泄密事件发生，难以追查泄密源头。
2. **动态屏幕水印与取证溯源需求（Forensic Attribution）**：企业安全体系要求在远程桌面画面中以半透明形式动态叠加访问者身份标记（访客设备 ID、对端别名、会话时间戳、警告标语），确保任意局部截图或照片均能快速溯源至泄密主体。
3. **宿主机强制水印（Host-Enforced Security Lockdown）**：遵循 ADR-004 的被控端最高主权原则，宿主机管理员在开启远程协助时可开启「强制水印」模式，访客端无法单方面关闭水印叠加，协议层与控制面予以硬性锁死拦截。
4. **零重型外部依赖（Zero External Crates, ADR-006）**：严格禁止引入 `freetype`、`fontdue`、`image` 等重型 C/Rust 字体渲染库。在 `tetherly-core` 内部实现自包含的 8x16 ASCII 点阵字体渲染器与定点数 Alpha 混合器，对 `PixelFormat::Bgra8` 与 `Rgba8` 实现纯安全内存像素合成。
5. **全链路协同与双端 CLI 100% 同构**：动态水印必须无缝穿透并联动实时渲染呈现（`screen_sink`）、会话录制（ADR-101）、单帧快照（ADR-103）、回环 REST API（`/api/screen/watermark`）以及双端 CLI（`tetherly` 与 `tetherly-cli`）。

---

## 决定

### 1. 核心字模与 Alpha 混合算法（`crates/tetherly-core/src/screen.rs`）

- **标准 8x16 ASCII 单色点阵字模**：
  - 内嵌 95 个可打印 ASCII 字符（ASCII 32 到 126）的 8x16 点阵掩码，每个字符占用 16 字节，总字模大小为 $95 \times 16 = 1520$ 字节（约 1.5 KB）；
  - 字符行每字节的 8 个比特依次代表从左至右的像素开关。
- **定点数 Alpha 混合公式**：
  - 针对前景文字色彩 $(R_{fg}, G_{fg}, B_{fg})$ 与背景像素 $(B_{bg}, G_{bg}, R_{bg})$，混合公式采用避免浮点除法的四舍五入整数定点运算：
    $$C_{out} = \frac{C_{fg} \times \alpha + C_{bg} \times (255 - \alpha) + 128}{255}$$
  - 分别针对 `PixelFormat::Bgra8`（通道 0: B, 1: G, 2: R）与 `PixelFormat::Rgba8`（通道 0: R, 1: G, 2: B）进行精准字节写入。
- **水印图样（`WatermarkPattern`）与参数配置**：
  - `Grid`：全屏对角平铺网格，默认横向间距 240 像素、纵向间距 160 像素，奇偶行错位 50%，覆盖全屏任一部位；
  - `Single`：单点角标或中心居中；
  - `Banner`：顶部或底部信息栏；
  - `ScreenWatermarkConfig`：包含 `enabled`, `host_enforced`, `text_template`, `opacity`, `color_rgb`, `scale`, `pattern`, `spacing_x`, `spacing_y`；
  - `ScreenWatermarkReport`：包含状态画像、格式化不透明度百分比、十六进制颜色与打标帧数统计。
- **就地打标方法**：
  - `ScreenFrame::apply_watermark(&mut self, config: &ScreenWatermarkConfig, peer_id: &str, timestamp: &str) -> Result<bool, CoreError>`。

### 2. 线协议控制信令 Tag 10（`ControlMsg::SetWatermarkState`）

- 在 14 字节定长信封（`TYPE_SCREEN_CONTROL = 0x0502`）中引入 Tag 10：
  - `magic = *b"TMV1"`；
  - `proto = 0x01`；
  - `tag = 10`；
  - `seq: u64` 编码：低 8 位为 `enabled: u8`，次 8 位为 `host_enforced: u8`。
- 双向信令往返与信道同步。

### 3. 节点运行时管线与安全控制（`crates/tetherly-node/src/runtime.rs`）

- `NodeShared` 挂载 `screen_watermark: Arc<RwLock<ScreenWatermarkConfig>>`、`screen_watermark_frames: AtomicU64` 与 `screen_peer_watermark_enforced: AtomicBool`；
- 在接收端呈现 (`screen_sink.present`) 与回放步进 (`screen_playback`) 时即时打标；
- 快照提取方法 `screen_snapshot` 若处于水印激活态，抓取画面时自动嵌入水印，并标记 `watermarked: true`；
- 宿主机安全锁定守卫：若 `screen_peer_watermark_enforced` 为真，访客端发起的禁用请求返回 `CoreError::InvalidConfig("watermark is enforced by host and cannot be disabled")`。

### 4. 回环 REST API 与双端 CLI 全同构

- `GET /api/screen/watermark` 与 `POST /api/screen/watermark[/enable|/disable]` 端点；
- `ScreenCmd::Watermark` 子命令族（`status`, `enable`, `disable`, `set`）；
- `screen status` 补充水印遥测展示。
