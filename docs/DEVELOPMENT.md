# Tetherly 开发文档

| 项 | 值 |
|---|---|
| 产品名 | **Tetherly** |
| 文档版本 | **1.25** |
| 日期 | 2026-10-17 |
| 状态 | **全面接手开发与 v1.25 开工**——(1) **确立 GitHub CI-First 构建与多端验证体系**：项目跨平台矩阵构建（Windows x86_64、Ubuntu Linux、macOS arm64）、Android 原生库（`libtetherly_android.so` arm64-v8a + Gradle）、代码规范与安全审计（`cargo fmt`、`clippy -D warnings`、`cargo deny check`），以及基于 `debian:bullseye`（glibc ≤ 2.31 守卫）的 Linux 发布产物打包**全面依托 GitHub Actions（`ci.yml` 与 `release.yml`）**；本地开发机专注轻量单测与逻辑演进，杜绝环境依赖污染与构建发版漂移。(2) **v1.25 核心攻坚方向**：① 补齐生产级文件发送入口（解决长期「只收不发」缺口）：落地 `/api/file/send` 回环 API 与 `tetherly send <file> --peer <id>` CLI 命令；② 设计基于 64 KiB 块对齐增量 SHA-256 与流式收发的大文件流式传输协议演进，消除全量 `Vec<u8>` 内存驻留对 `FILE_MAX = 512 MiB` 的束缚。+ v1.24 把两处「写的是保护、实际不生效」的边界修实（FILE_MAX 512 MiB 成为真门、网卡分类可解释且单一真理源，290 全绿）+ v1.23 落地 Windows ANCS 真机传输栈（WinRT BLE GATT，285 全绿）+ v1.22 修正内层帧预算（推导 65507，280 全绿）+ v1.21 存活检测与背压 + v1.20 CLI 控制面回环客户端 + v1.19 macOS/Linux 采集后端 + v1.18 APK 端到端真机闭环 + v1.8 DXGI 远程屏幕闭环 |
| crate / 二进制 | `tetherly-core`、`tetherly-crypto`、`tetherly-net`、`tetherly-node`（`tetherly` 二进制）、`tetherly-cli`、`tetherly-android`（`libtetherly_android.so` + `android_probe`） |

本文是实现合同。与代码冲突时改文档或改代码，必须一致。旧名 InterLink 作废。

**1.25 相对 1.24(全面接手、CI-First 开发与构建体系确立、文件发送生产闭环与流式化蓝图)**

- **动机(全面接手开发与生产缺口攻坚)**:接手后经过全套 290 项测试验证与代码拓扑审查,确定核心传输与安全架构已高度稳固,但在工程实践与生产可用性上面临两个关键断层:
  1. **构建与验证体系必须 CI-First**:Tetherly 涉及 Windows(WinRT GATT/DXGI/SendInput)、Linux(bullseye glibc 2.31/X11)、macOS(CoreGraphics)以及 Android(NDK arm64-v8a + Gradle JNI)等多端复杂编译链。若依赖本地环境多端拼装,极易重蹈旧 MinGW 32/64 位混淆或 glibc 符号过高的覆辙。遵循用户指示:「构建什么的能用 github ci 的就用 ci」,正式将 GitHub Actions 确立为构建、多平台矩阵回归与安全审计的第一责任面。
  2. **生产文件「只收不发」缺口(v1.24 诚实报告的待办)**:自 v1.12 引入断点续传以来,文件发送功能 `offer_and_send_bytes` 仅在集成测试 `tests/phase1.rs` 中被调用;Node 服务的生产 Web UI 只有接收确认交互,CLI 亦缺少 `tetherly send` 指令。生产节点处于「接收链路健全、发送入口缺失」的半闭环状态。
  3. **大文件流式化架构演进**:当前 `FILE_MAX = 512 MiB` 是受制于两端整文件 `Vec<u8>` 内存驻留的妥协上限。必须设计基于 64 KiB 块对齐的边读/边收/边写与增量 SHA-256 流式协议,将内存占用降至 $O(1)$,为解除 512 MiB 上限铺平道路。
- **CI-First 开发与交付规约**:
  - **多平台矩阵集成 (`ci.yml`)**:每次功能提交与 PR 必须由 GitHub Actions 在 `ubuntu-latest`、`windows-latest`、`macos-latest` 上原生执行 `cargo fmt --check`、`cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo test --workspace --locked`;同步触发 `android` 容器化任务跑 `cargo ndk` 编译 `libtetherly_android.so` 并跑通 Gradle 单测与打包;通过 `cargo-deny-action` 执行许可、漏洞（advisories/licenses/bans/sources）全量守卫。
  - **Linux 发版安全容器 (`release.yml`)**:发布构建严格在 `debian:bullseye`(glibc 2.31)容器中执行,并以 `objdump` 强校验拒绝 `GLIBC_2.32+` 动态符号,确保二进制向下兼容 Debian 11 / Ubuntu 20.04。发版时自动合并 Windows 与 macOS 产物并推送到 GitHub Releases。
  - **本地开发机准则**:本地开发机只负责代码编写、局部语法检查与单测调优,禁止在本地维护脆弱的多端打包交叉编译器,发版制品一律以 GitHub Actions 生成并由 `gh release` / `actions/upload-artifact` 托管为准。
- **v1.25 实施方案(生产文件发送入口闭环)**:
  - **回环 HTTP API**:在 `crates/tetherly-node/src/uihttp.rs` 增设 `POST /api/file/send` 路由,接收 JSON `{ "path": "/path/to/file", "peer": "tdev_..." }`。调用方验证文件可读性、存在性与不超过 `FILE_MAX`,解析目标 `peer` 在 `live_peers` 集合内,调用 `node.send_file(path, peer)`。
  - **CLI 客户端命令**:在 `bins/tetherly-cli/src/main.rs` 与 `crates/tetherly-node/src/bin/tetherly.rs` 增设 `tetherly send <file_path> [--peer <device_id>] [--ui-port <port>]` 子命令。作为 127.0.0.1 活节点回环客户端,与 `tetherly pin`/`dial`/`notify` 保持完全一致的 loopback client 模式（ADR-013）。
  - **流式架构蓝图**:规划基于既有 `TFL2`（魔数 + 块对齐起始偏移）协议的零拷贝流式发送引擎（`FileStreamSource`/`FileStreamSink`）,分块读取（64 KiB）+ `ring::digest::Context` 增量 SHA-256 哈希 + 增量落盘 `.part`,彻底解耦内存大小与文件大小。
- **新增回归与验证目标**:
  - `tests/phase1.rs` 增补 `file_sent_via_live_node_api_reaches_the_paired_peer` 端到端集成回归；
  - UI 自动化测试覆盖 `/api/file/send` 成功与超限/离线错误码处理；
  - 维持 290+ 全绿与零破坏性变更。

**1.24 相对 1.23(文件单文件上限成为真门;网卡分类可解释且单一真理源)**

- **动机(补齐 §9.4 只缺的一段)**:§9.4 明写「Windows:`windows` crate GATT」,但仓库里只有 `tetherly-core::ancs`(协议编解码 + `AncsIngress` 状态机 + `AncsTransport` 端口 + `MemoryAncsTransport`)。真实栈、乃至**入站的接线**都不存在:`NodeConfig.ancs` 在生产路径里恒为 `None`,`tetherly-core` 也刻意不知道「值变了」这件事。于是 ANCS 在真机上**根本无从启动**——端口有了,插头没接。
- **实现**:新增 `crates/tetherly-node/src/ancs.rs`(装配点:`AncsInbound` 四态通道 `Ready`/`NotificationSource`/`DataSource`/`Disconnected`;`open(address) -> Result<Option<AncsLink>, CoreError>`,非 Windows 目标诚实 `Ok(None)`)与 `crates/tetherly-node/src/ancs/win_gatt.rs`(自包含,只 import 绝对路径,照 `screen/macos.rs`、`screen/x11.rs` 先例)。
  - **线程模型**:专属 `tetherly-ancs` 线程持有全部 WinRT 接口并自建 **MTA** 公寓(`CoInitializeEx(COINIT_MULTITHREADED)`;`windows` 0.59 无 `initialize_mta` 封装)。`AncsTransport` 前端只持有一个 `Sender`,故端口**按构造** `Send + Sync`——node 永不直接触碰 COM 接口。
  - **同步端口的落点**:`windows` 0.59 的 async 扩展为 `IAsyncOperation<T>` 提供**阻塞 `get()`**(内部 OS `Waiter` 信号,不空转),因此同步的 `AncsTransport` 三个方法可直接实现,**不需要自造 executor、不需要新依赖**。
  - **建链**:`FromBluetoothAddressAsync` → `GetGattServicesForUuidAsync(ANCS)`(要求 `Status == Success` 且服务非空)→ `GetCharacteristicsAsync` → 按 UUID 找三特征,**并校验属性**(Notification Source / Data Source 必须含 `Notify`,Control Point 必须含 `Write`;缺一即点名报错,不静默降级)。
  - **订阅与写**:`subscribe_*` 写 CCCD 为 `Notify` 并要求 `Success`;`write_control_point` 经 `DataWriter` 打包 `IBuffer` 后 `WriteValueWithResultAsync` 并核对 `GattWriteResult::Status`,**阻塞到结果为真**——这正是「串行、且只有一条在途」成为事实而非假设的原因。前端对每次命令的 ack 等待设 **10s 上界**,超时/线程消失一律报 `AncsSubscribe`/`AncsWrite`,与 v1.21 的 `WriteStalled` 同一态度(不谎报成功)。
  - **入站**:两个特征的 `ValueChanged` 回调**只把字节拷到无界通道**(§9.4 禁止在回调线程写 Control Point);`ConnectionStatusChanged` 置原子标志,驱动线程据此收尾并发 `Disconnected`。`Session::drop` 显式 `RemoveValueChanged`/`RemoveConnectionStatusChanged`,不留注册悬挂。
  - **重连**:建链失败或断开后按 `tetherly_core::ancs::ancs_backoff_delay(attempt)` 指数退避(1s 起、30s 封顶),成功即清零。
  - **node 侧接线**:`Node::spawn_ancs_pump(inbound)` 在**独占线程 + current-thread 运行时**里循环——`Ready` → `ancs_connect()`(由 `AncsIngress::connect` 固定「先 Data Source 后 Notification Source」的顺序);`NotificationSource`/`DataSource` → 对应 feed 点;`Disconnected` → `ancs_disconnected()`;每 500ms `ancs_tick()` 驱动串行 Control Point 队列。CLI `tetherly run --ancs <11:22:33:44:55:66>` 打开链路(先在系统蓝牙里配对 iPhone)。
- **新增回归(全绿,280 → 285)**:`win_gatt` 纯逻辑单测 4 条(Windows 地址记法 `:`/`-`/裸十六进制解析且首字节为高位、6 类畸形输入拒绝、四个 ANCS UUID 的 **LE 解码 ↔ 规范 GUID 逐字段一致**、畸形 UUID 报错——映射错会静默查错服务,故此断言保值);`tests/phase4.rs::m4_5` 端到端 1 条,用 `MemoryAncsTransport` 喂 `AncsInbound` 通道,断言**订阅顺序**(Data Source 先)、**回调只入队而 tick 才写 Control Point**、DataSource 响应成为候选、`Disconnected` 落入 `Backoff`。
- **验证与诚实边界**:`cargo fmt --check` / `clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace --locked`(**285 全绿**)/ `cargo deny`(advisories/bans/licenses/sources)全绿;`Cargo.lock` **零改动**(WinRT 蓝牙特性同属既有 `windows` crate,无新包)。**诚实边界**:WinRT 代码在**原生 Windows x64** 上真编译(比 v1.19 的跨目标 type-check 更强),但 `FromBluetoothAddressAsync`/配对/真机通知的**运行时行为需物理 iPhone**,故仍是 **Manual-required**;macOS(CoreBluetooth)与 Linux(BlueZ)栈**未实现**,`open()` 诚实返回 `Ok(None)` 而非假装可用。

**1.24 相对 1.23(文件单文件上限成为真门;网卡分类可解释且单一真理源)**

- **动机(Tetherly 自查 + 参考项目对照)**:对照 Lancat(README 里「可设单文件大小上限」)回看本仓库的文件接收路径时,发现 `crates/tetherly-node/src/runtime.rs` 那句 `if total > size.max(32 * 1024 * 1024)` 是**死代码**——紧邻上一行的 `if total != size { .. }` 已经把两者钉成相等,于是 `total > size.max(k)` 恒等于 `size > max(size, k)`,永远为假。也就是说「拒绝荒谬的声明大小」这道**读起来存在的保护从未生效**。同族的第二个缺口:§14「默认 LAN 监听」只有 IP 段判据,既无法区分真网卡与第三方 VPN TUN,也不给任何解释——用户对着一个 `100.64.x.x` 的 Tailscale 地址只会看到「什么都没发现」。
- **修复(1):`FILE_MAX` 成为真门,并说明它为什么是内存边界**。v1 的整条文件路径是**整文件驻留内存**的:接收端把正文累积进 `Vec<u8>` 再 `atomic_write`,发送端 `pending_out` 还要在调用方缓冲之上**再存一份**(`data.to_vec()`)。因此上限的意义是**内存边界**而非磁盘策略:`tetherly-core::FILE_MAX = 512 MiB`。
  - **报价即拒(最外层)**:`FileHub::offer_resumable` 在解析时按**声明大小**拒绝 `size > FILE_MAX`,返回 `CoreError::FileTooLarge(FILE_MAX)`——**零字节分配,用户从未看到候选**。
  - **通道再钉一次**:`accept_file` 把 `FILE_MAX` 作为真实 `max_size` 交给 `recv_chunks_after_header`(此前传 `u64::MAX`,使通道自带的 `total > max_size` 检查形同虚设),并在本地再断言一次(通道头是**未认证**的,不可单独信任)。
  - **发送端同向**:`offer_and_send_bytes` 拒绝超过上限的 `data`,不向对端报一个对方必须拒绝的价。
  - **拒绝 ≠ 断链**:`SessionEvent::FileOffer` 分支此前用 `?`,任何 `CoreError`(不安全文件名、坏摘要、超限、候选表满)都会冒泡**拆掉整个会话**;现改为记录即丢(warn + 丢弃该报价),会话继续。
- **修复(2):网卡分类单一真理源 + 可解释**。`tetherly-core::bind` 新增 `BindClass`(`Listen`/`LoopbackAlias`/`Unspecified`/`Broadcast`/`Multicast`/`Overlay`/`CgnatOverlay`/`Ipv6`/`NotLan`)与 `classify`/`classify_v4`/`is_cgnat`;`should_listen`/`should_listen_v4` 改为**就是** `classify(..).listens()`,故「诊断」与「实际绑定」在结构上**不可能分叉**。`tetherly-node::lan` 新增 `IfaceReport` + `interface_report()`(每次 `/api/state` 快照里的 `lan` 字段)与 `log_skipped_interfaces()`(启动时每个被跳过的地址各一行 `reason`)。要点:CGNAT 段 `100.64.0.0/10` 不再与「普通公网地址」混为一谈,而是 `cgnat-overlay-needs-explicit-cidr`——这正是教用户去传 `--overlay-cidr 100.64.0.0/10` 的那句话;`127/8` 的非规范别名(如解析器 stub 的 `127.0.0.53`)显式归为 `LoopbackAlias`,`lan_ips` 因此不再需要「跳过 loopback 接口」这条隐式规则,规则只剩**一处**。
- **新增回归(全绿,285 → 290)**:`tetherly-core/src/bind.rs`(1 条:`classify` 逐类原因 + 「只有 `Listen` 会 listen、非 `Listen` 必给 reason」的不变量);`tetherly-core/src/filexfer.rs`(1 条:超限报价得 `FileTooLarge` 且**不入表**,恰好等于上限则接受);`crates/tetherly-net/src/filechan.rs`(1 条:通道头声明 1025 而上限 1024 → `TooLarge`,且**未缓冲任何字节**);`tetherly-node/src/lan.rs`(1 条:报告与 `lan_ips()` 必须点名**同一批地址**);`tests/phase1.rs::an_oversized_offer_is_refused_without_dropping_the_session`(端到端:伪造 `size = FILE_MAX + 1` 的报价 → 对端候选**保持为空**、会话未断,**随后一笔正常传输仍然成功**)。
- **验证与诚实边界**:`cargo fmt --check` / `clippy --workspace --all-targets -- -D warnings` / `cargo test --workspace --locked`(**290 全绿**)/ `cargo deny`(advisories/bans/licenses/sources)全绿;`Cargo.lock` **零改动**(未新增依赖)。**诚实边界**:`FILE_MAX` 是「当前整文件驻留内存」这一实现下的边界,**不是产品策略**——真正的流式(接收端边收边写 + 增量 sha256、发送端分块读)是后续项,做完即可抬高该值;网卡分类仍是 **IP 段 + CGNAT 判据**,**并未**采用 Lancat 那套 OS 标记分类(Windows `GetIfTable2`/Linux sysfs),故「一个带 RFC1918 地址的第三方 TUN 会被当作 LAN」这一点**尚在**;v1 的跨网仍只走用户自装的 EasyTier 侧车(§2.2),CGNAT fabric 需用户显式 `--overlay-cidr`。

**1.22 相对 1.21(修正内层帧预算:分片屏幕帧此前无法加密)**

- **动机(为 v1.21 缺陷补回归时暴露)**:编写「写停滞」回归时用满额 payload 的内部帧,竟先撞上 `Crypto(Noise("input error"))`。顺藤摸瓜发现这不是测试的问题,而是**生产路径的潜伏缺陷**:任何**需要分片的屏幕帧**都发不出去。zstd 把真实帧压得远小于 64 KiB,故已跑的端到端链路从未触及,属「成功但不可证」的同类风险(v1.18 `pin` 缺陷族)。
- **根因**:`INNER_PAYLOAD_MAX` 原为写死的 `64 * 1024 = 65536`,**未计入** `InnerFrame::encode()` 前置的 **12 字节内层帧头**与 **16 字节 AEAD tag**——而这两者都在 Noise 单条消息 **65535 字节**上限之内。实测(见下 `limits` 回归)可加密明文上限恰为 **`65535 − 16 = 65519`**,故内层 payload 其实最多只能到 `65519 − 12 = 65507`,原常量**多算了 29 字节**。屏幕分片公式 `SCREEN_FRAG_CHUNK_MAX = INNER_PAYLOAD_MAX − SCREEN_FRAG_HEADER` 因此产出 `24 + 65512 = 65536` 字节的分片 payload,编码后 **65548 字节明文**,传输层直接拒加密。同一根因也波及 `to_frame()` 的单帧路径(payload 允许到 65536,同样加不了密)。**文件通道不受影响**:它走第二条 TCP 上自己的 ChaCha20-Poly1305 + 长度前缀(`aead_seal`),不经 Noise transport。
- **修复(以传输上限推导,而非写死)**:`crates/tetherly-core/src/frame.rs` 新增 `INNER_HEADER = 12` 与 `NOISE_MESSAGE_MAX = 65_535`,`INNER_PAYLOAD_MAX = NOISE_MESSAGE_MAX − 16 − INNER_HEADER`(= **65507**);`encode`/`decode` 改用 `INNER_HEADER`,不再写死 12。于是「帧层不能把传输层加密不了的明文交上去」成为**结构性事实**:分片 payload `24 + (65507 − 24) = 65507` 恰好等于上限,编码后 65519 = 传输上限,恰好可加密。向后兼容:任何版本都**从未**成功发出过 payload ∈ (65507, 65536] 的帧(加密必失败于写之前),故收紧解码上限对线上格式无损。
- **新增回归(全绿,275 → 280)**:`crates/tetherly-crypto/tests/limits.rs`(2 条,真实 IK 握手后量出明文上限 65519 通过/65520 被拒,并断言满额内层帧明文 ≤ 上限且可加密、再多一字节被拒);`crates/tetherly-net/tests/screen_wire.rs`(1 条,4 MiB 1024×1024 BGRA 帧经 `to_frames()` 分片、**真实会话**逐片 `send_bounded`、对端 `recv_inner` + `ScreenAssembler` 重组后逐字节相等);`crates/tetherly-net/tests/backpressure.rs`(2 条,v1.21「写停滞」缺陷的会话级回归:不读的对端在 `SESSION_WRITE_TIMEOUT` 后得 `WriteStalled` 而非永久挂起,持续读的对端绝不误报停滞)。
- **验证与诚实边界**:本机 `cargo fmt --check` / `clippy --workspace --all-targets -D warnings` / `cargo test --workspace --locked`(**280 全绿**)/ `cargo deny`(advisories/bans/licenses/sources)全绿。**诚实边界**:窗口预算的修正为端到端**新回归**所证,但真机满屏帧(未压缩/大分辨率)的 45720 线上实测仍在 Phase 5「真机闭环」的既有 Manual-required 范围内。

**1.21 相对 1.20(会话存活检测与背压;Linux 发版锚点由 glibc 2.35 下移到 2.31)**

- **动机(跨 VPS 实测暴露)**:两台 VPS 配对成功、双向 `path lan` 正常,但**只有第一条 `notify` 到达对端**;此后每次 `notify` 都返回 `notified tdev_...`,对端却毫无日志,双向皆然。两端 `ss` 显示连接仍 `ESTAB`,但 **Send-Q 堆积未确认字节**(一侧 422 B、另一侧 194 B)——路径已被中间设备**静默黑洞化**,而**没有任何一层会发现**。
- **根因**:控制会话任务的 `select!` 里 **`await` 了写操作**,而 `write_tagged` 末尾 `flush()` 在发送缓冲填满、对端零窗口时会**无限期挂起**——整个任务就此卡死(既不再 `recv_event`,又因队列**无界**而继续吞下后续帧);`send_to` 用 `mpsc::unbounded_channel`,只要队列还在就返回 `Ok`。桌面节点**从不主动发 ping**(只在收到时回 pong),故 TCP 的 `ESTABLISHED` 被一路当作「活着」。
- **修复三层**:
  - `tetherly-core::liveness`(**抽离于时钟之外**,无 `now()`/随机源):`SESSION_HEARTBEAT=15s`、`SESSION_LIVENESS_DEADLINE=45s`;`Liveness::is_expired` 判据是「45s 内**无任何入站帧**即判死」,减法饱和故**时钟回拨不会误判**;4 条确定性单测(边界、重装、时钟回拨)。
  - `tetherly-net`:`SESSION_WRITE_TIMEOUT=15s` + `NetError::WriteStalled` + `ActiveSession::send_bounded()`——把写包进超时,把「静默挂起」变成**硬错误**;超时即视为致命(写可能停在中途),调用方据此拆会话。
  - `tetherly-node`:控制/输入/屏幕/屏幕控制四条通道**全部改为有界**(`mpsc::channel`;控制与输入 256、屏幕 32),入队改 `try_send`——满即**诚实报错**(`peer busy; send queue full`)而非静默缓冲,落实背压;控制会话加**心跳**(每 15s 经同一通道发 `ping`,顺带保活 NAT 映射)与存活判据(拆会话 → 发 `PeerDown` → 后续 `notify` 快速真失败);**全部 8 处会话写**改用 `send_bounded`;屏幕帧队列满时**整帧丢弃**(直播语义,绝不排队部分帧——`try_send` 前先看 `capacity()`),避免慢链路下无界增长。
- **效果**:路径死掉后**至多 15s** 会话被拆,`notify` 从「谎报成功」变为**可操作的失败**(`peer gone`/`not connected`);对活跃流则表现为**丢帧而非阻塞**。
- **Linux 发版锚点下移到 glibc 2.31(`.github/workflows/release.yml`)**:v1.20 的 `ubuntu-22.04` 锚点只保证「不高于 2.35」,产物实测要求 **`GLIBC_2.34`**——可达主机都能跑,却**跑不了 glibc 2.31 的老发行版**,与 §14「覆盖 glibc 2.31」的承诺不符。改为在 **`debian:bullseye`(glibc 2.31)容器**内构建:动态符号地板随之 **≤ 2.31**,一个产物覆盖 Debian 11 / Ubuntu 20.04 及更新主机;守卫改为拒绝 `GLIBC_2.32+` 并**打印实际地板**(v0.1.3 实测地板 **`GLIBC_2.30`**)。桌面 build 拆为独立 job(`build-linux` 容器 + `build-desktop` 矩阵),`publish` 依赖两者。两条容器细节是**必须的**:①容器默认 `/bin/sh` 是 dash、拒绝 `set -o pipefail`,故 job 的 `run` shell 固定为 `bash`;②bullseye 已过 LTS,`deb.debian.org` 不再提供其 pool(`apt-get update` 全部 404),故 `sources.list` 指向 **`archive.debian.org`** 并加 `Acquire::Check-Valid-Until=false` 以接受归档的过期 `Release`。
- **验证与诚实边界**:本机 `cargo fmt --check` / `clippy --workspace --all-targets -D warnings` / `cargo test --workspace --locked`(**275 全绿**,=271 + `liveness` 4 条)/ `cargo deny`(advisories/licenses/bans/sources)全绿。CI(`ci.yml`)三平台 + android + deny 五个 job 全绿。跨 VPS 重连后连发两条通知**均被对端入库**(OTP 正文 `has_otp:true`、第二条 `has_otp:false`)。发版(`release.yml`)在 `v0.1.3` tag 上全绿:`build-linux` 容器内 `objdump` 地板为 `GLIBC_2.30`,六个产物(linux/windows/macos 各 tetherly 与 tetherly-cli)已发布。**诚实边界**:容器构建路径只在 CI 验证(本机无 Linux 链接器);「glibc 2.31 真机启动」以 CI 守卫 + 地板打印为证据,仍需在一台真实 glibc 2.31 主机上复验一次。

**1.20 相对 1.19(CLI 控制面全部改为「活节点回环客户端」;新增 `/api/notify`;Linux 发版锚定 glibc 2.35)**

- **动机**:v1.18 只修了 `pin` 这一处「CLI 自起节点」缺陷,但同一类错误在 `dial` 上仍然存在,且**跨节点投递通知根本没有 CLI 路径**。跨 VPS 实测暴露:`tetherly dial` 在守护进程已在跑时执行 `Node::start(cfg)?` 去**再绑一遍**同一批 LAN 端口,直接死于 `no LAN address bound`;而 `tetherly inject-notify` 在**自己的**进程里构造 `NotifyPush` 并 `ingest_local_push`,只把候选喂给本进程——从不经 Noise 会话,**永远到不了对端**。根因与 ADR-013 同出一辙:能接受 PIN、能建立/持有会话、能投递消息的,永远是**持有那份进程内状态的那个进程**。
- **`Cmd::Dial` → 回环 `POST /api/dial`**:与 `pin` 同一形状——附带 `--ui-port`(默认 45716),经 `tetherly_node::loopback_post` 把 `{addr, pin}` 交给**运行中**的节点;`dial` 的 PIN 装置、拨号、`persist_trust`、`remember_route` 全部发生在活节点里,CLI 只读回结果。无守护进程监听时**诚实非零退出**并提示 `tetherly --data-dir <dir> run`。**红线保持**:`/api/dial` 仅在 127.0.0.1 可达(复用既有回环 UI 的 `Host` 校验),非回环来源一律 403。
- **`Cmd::InjectNotify` → `Cmd::Notify` + 新路由 `POST /api/notify`**:`inject-notify` 只在本进程灌假推送、无法投递,予以**删除**并替换为真正的投递命令——`tetherly notify --title … --body … [--app-id …] [--peer tdev_…] [--ui-port …]`,经回环 `POST /api/notify` 由活节点 `send_notify()` **经已配对的 Noise 会话**发出。目标选择**诚实**:恰好一个 live peer 时隐含之;零个直接 400 `no connected peer`;**多个**则要求显式 `--peer`,不静默挑一个。`--peer` 先 `DeviceId::parse` 再校验它在 live 集合内,未知/不在线一律 400。正文与 v1.18 一致:**只投递、不执行、不入日志**;`uid` 取本机时刻、`app_name` 固定 `tetherly`、动作集裁剪为 `copy`(`NotifyPush::truncate` 再兜一次长度与 URL 动作,与入站路径同一道闸)。
- **Linux 发版锚定 glibc 2.35(`.github/workflows/release.yml`)**:发布二进制除 glibc/libgcc 外**全静态**,故 runner 的 glibc 就是它的运行地板。原矩阵用 `ubuntu-latest`(24.04 / glibc 2.39),产物在 glibc 2.31 的主机上**拒绝启动**。改为 `ubuntu-22.04`(glibc 2.35,覆盖 Debian 12 / Ubuntu 22.04 / 当代 RHEL 系),并加一步 `objdump -T … | grep -E 'GLIBC_2\.(3[6-9]|[4-9][0-9])'` 守卫:一旦二进制引用了更新的 glibc **构建即失败**,而不是把不可运行的包发出去。Windows / macOS 行不受影响(CI 测试矩阵不变,仅发布产物变化)。
- **验证与诚实边界**:本机 `cargo fmt --check` / `clippy --workspace --all-targets -D warnings` / `cargo test --workspace --locked`(**271 全绿**,`tests/phase1.rs` 由 9 → 10:新增 `notify_posted_to_the_live_node_api_reaches_the_paired_peer`,断言回环 `/api/notify` 投出的文本**确实抵达对端候选**,且未连接的 `--peer` 被 400 拒绝,使 200 不可能是「无条件收下」)/ `cargo deny`(advisories/licenses/bans/sources)全绿。**诚实边界**:本机 Windows-gnu 工具链的 MinGW.org 6.3.0 是 32 位、其 `ld` 不认 rustc 1.99 发出的 `--high-entropy-va`,故本地链接一度失败——这是**开发机环境问题**,不属项目代码;为取得本地测试证据,本次临时以 rustup 自带的 64 位 self-contained MinGW(gcc 14.2)作链接器、并以 NDK clang 20 补上其缺失的 `as`/C 前端(`zstd-sys`),**均未写入仓库**(`.cargo/config.toml` 保持机器无关,CI 走 MSVC 主机不受影响)。真机 LAN 直连双桌面、8h soak 等仍 Manual-required。

**1.19 相对 1.18(填掉 §9.7 承诺的 macOS/Linux 采集缺口:real macOS CoreGraphics + Linux X11 SHM;两端后端文件自包含、按目标编译验证)**

- **动机**:§9.7 与 §14 一直把「macOS `core-graphics`、Linux X11 SHM(`x11rb`)」标为**下一迭代**;v1.8 只把 Windows 真机链闭合。本次把这两个采集后端按 `cred.rs` 的后端化先例落地——每个后端一个**自包含文件**(只 import 绝对路径),因而可在本机按目标交叉编译验证其编译正确性。
- **macOS `MacSource`(`crates/tetherly-node/src/screen/macos.rs`,新)**:`CGMainDisplayID` → `CGDisplayCreateImage` → 绘入**显式 BGRA8 `CGBitmapContext`**(`PremultipliedFirst | Order32Little`)→ 按框架返回的实际 stride **去行尾填充** → `ScreenFrame::raw`。规范化经过已知格式是刻意的:宽色域 / Retina 屏可能回 16-bit 或 RGBA 采样,远端解码器不该去猜。请求 stride 取 **16 字节对齐**,再读回框架实际 stride 去填充——否则 1365px 这类行宽(5460 字节)会因框架补位而整帧失败。屏幕录制权限被拒时返回 `ScreenRefused`(**不返回空白帧**)。`CGDisplayCreateImage` 被软弃用(建议 ScreenCaptureKit),保留原因:ScreenCaptureKit 需要签名 app bundle + 用户同意弹窗,并会把依赖面扩到 AppKit;弃用警告**只在该 import 一处** `#[allow(deprecated)]`,不 crate 级静默。**呈现**在 macOS 需要 AppKit `NSWindow`(CoreGraphics 位图上下文无法触及屏幕),§9.7 已把它列入下一迭代,故 `present` 返回点名 `NSWindow` 的诚实 `ScreenRefused`,不假装。
- **Linux/BSD X11 `X11Source`(`crates/tetherly-node/src/screen/x11.rs`,新)**:两条传输共用同一解码——**MIT-SHM**(`shm_create_segment` + `shm_get_image`,服务器直接写入我们 `mmap` 的段,整帧不过 socket)为快路径;**plain `GetImage`** 为无 SHM 服务器(远程 X、加固服务器)的回退。SHM 可用性门:**扩展存在 ≠ 允许 fd 传递**,要求 MIT-SHM ≥ 1.2(分段段)。读到 `reply.size` 字节(而非整段),避免上帧残留字节混入本帧尾部。根窗口字节序非 LSB-first、或根深度不在 `{24,32}` 时**诚实拒绝**;depth 24 的未用高字节填 `0xFF`(depth 32 已有真实 alpha,不动)。**呈现**在 X11 可落地,故一并实现:override-redirect 窗口 + `PutImage`,窗口深度经 `COPY_DEPTH_FROM_PARENT` 从根继承,`PutImage` 用**读回的根深度**而非硬编码 24(否则 32 位根上服务器回 `BadMatch`)。
- **接线**:`crates/tetherly-node/src/screen.rs` 按目标 `#[cfg]` 选择 `inner`(`DxgiSource` / `MacSource` / `X11Source`),`SystemSource` 保持单态(无动态分派);`SystemSink::present` 四臂(macOS → 诚实拒绝,`not(any(windows,macos,unix))` → 诚实拒绝)。`Cargo.toml` 新增**目标依赖**:macOS `objc2-core-foundation` + `objc2-core-graphics`(`default-features = false`,避开 objc2-metal/dispatch2 面)、Linux `x11rb`(`shm`)+ `libc`(x11rb 自带的 `libc` 在其 `allow-unsafe-code` 之后,未启用,故显式声明)。**未引入任何新包**:这些 crate 已随 `arboard 3.6.1` 传入 lockfile。
- **验证方式与诚实边界**:两个后端在**本机对各自目标**(`x86_64-apple-darwin` / `x86_64-unknown-linux-gnu`)以最小探针 crate 完成 `clippy -D warnings` **类型检查**(含整个 `screen.rs`,用桩 `zstd` 绕开目标 C 编译器),探针用后即删;后端单元测试(7 + 7 条:stride 对齐、去填充、截断拒绝、depth 处理、非 BGRA 拒绝)是**纯逻辑、不触显示器**,CI 在 `ubuntu-latest` / `macos-latest` 上原生编译并执行。**真机采集需真实显示器/会话**(macOS 需屏幕录制权限,X11 需 X server),仍 Manual-required;本机无 Linux/macOS 链接器,故不做本地执行,只做类型检查 + CI 原生执行。
- **诚实边界**:开放编码器(OpenH264/rav1e)、硬件编码、真机注入、macOS 呈现(NSWindow)、Wayland portal 仍 Manual-required/下一迭代。本次不宣称 macOS/Linux 真机远程桌面已可用,只宣称**后端已实现且按目标编译验证**。

**1.18 相对 1.17(真机准入口:APK 内 Kotlin→JNI→实时会话 UI 全链路;`tetherly pin` 缺陷修复;手机通知 p95 上机实测)**

- **APK 端到端已验(云手机 Galaxy Fold / arm64-v8a / Android 13)**:真机 `app.tetherly` 的前台 `NotificationListenerService` → JNI(`Java_app_tetherly_NativeBridge_*`)→ `libtetherly_android.so` 的实时 Noise 会话整条链路闭合:真机侧推送的测试通知经桌面节点提取为候选 `{app_id:"com.android.shell", has_otp:true, source:"tdev_…"}`(通知**正文与验证码不入快照、不入日志**,只暴露 `has_otp` 布尔与来源设备 id)。§9.6 此前标注的「APK 内 Kotlin→JNI→会话的 UI 全链路仍以真机手动为准」由本次真机实测关闭。
- **M1.1 p95 上机计量(手机→桌面通知往返)**:节点 wall-clock 计量在真机链路上给出 `rtt_min≈17.4ms / p50≈17.9ms / mean≈37.5ms / p95≈56.9ms / p99≈361.8ms / max≈361.8ms`。p95 落在 M1.1 的 **<1.0s 预算内**(约 18× 余量)。**诚实边界**:这是**经 `adb reverse` 隧道的云手机**往返,不是 LAN 直连同机型(后者约 15–16ms);p99 尾尖峰来自云手机 adb 隧道抖动,不当作协议成本。8h soak 仍 Manual-required。
- **`tetherly pin` 缺陷与修复(真机排查暴露)**:CLI 的 `Cmd::Pin` 此前实现为 `Node::start(cfg)?` + `node.generate_pin()?`——它在**另一个一次性进程**里生成 PIN 并打印。而 PIN 只对**持有它的那个进程**有意义(`pin_created_ms` 是纯进程内状态,从不持久化),所以打印出的码**永远无法**被正在运行的守护进程接受;真机表现为「PIN 明明显示出来了,手机却一直 `pin expired`」。**可用路径一直是运行中守护进程的回环 API `POST 127.0.0.1:45716/api/pin`**(UI 的「显示 8 位 PIN」按钮走的就是它)。修复:`Cmd::Pin` 改为携带 `--ui-port`(默认 45716)的**回环客户端**,经新增的 `tetherly_node::loopback_post(port, "/api/pin", b"{}")` 把 PIN 装置到**活着的**节点;没有守护进程监听时**诚实**非零退出并给可操作提示。配套改动:`uihttp::spawn` → `bind_and_spawn`(同步 bind 并返回实际端口,端口被占由静默 `warn!` 变为硬错误)、`Node::spawn_ui()`/`Node::ui_port()`(`Shared.bound_ui: AtomicU16`)、`run()` 返回前先 bind UI 端口,使「节点已起」== 该端口真实可达。
- **回归测试**:`tests/phase1.rs::pin_shown_by_the_live_node_api_is_the_one_that_pairs`——起宿主+对端,`host.spawn_ui()` 取回环端口,`POST /api/pin` 取 PIN 并配对成功;再起第三个只装了硬编码 PIN 的节点,断言 `peer.dial(stale.control_addr()).await.is_err()`,使断言不可能凑巧通过。全量 269 → **270 全绿**。
- **诚实边界**:APK 链路与 p95 为**云手机经 `adb reverse` 隧道**实测,非 LAN 双物理桌面;8h soak、双桌面 100 次物理往返、macOS/Linux 采集与注入、开放编码器(OpenH264/rav1e)仍 Manual-required。本次不宣称真机 LAN 远程桌面或 8h soak 已达标。

**1.17 相对 1.16（把 §13 的属性测试从「单个解码器/单条消息」推进到「有状态状态机 + 任意操作序列」）**

- **动机**：v1.16 的属性测试与变异语料覆盖的是**无状态函数**（解码器、往返编解码）。§13 里真正危险的入站面是**有状态机器**——`ReplayGuard` 的窗口、限流器的令牌/漏桶、`NotifyHub` 的去重与过期、`FileHub` 的接受/落盘门、`ResumeTable` 的偏移领取、屏幕收发与重组、`AncsIngress` 的订阅/退避/持有。这些的危险不在「单条输入能不能解」，而在「**任意顺序的一串输入能否把状态推入违约态**」。v1.17 就补这一层。
- **`proptest`（core，`crates/tetherly-core/tests/state_machines.rs`，11 条）**：以 `program(max_ops)` 策略生成**任意字节操作程序**（`Vec<u8>` 当指令流：取字节选动作、取若干字节做参数），逐条喂给被测机器并**在每一步之后**断言安全契约；`Cur` 游标对越界读**饱和返回**而非 panic，保证程序本身永远不会崩，从而把 panic 全部归因于被测机器。覆盖：
  - `ReplayGuard`：**已被接受过的 id 永不再被接受**；远古 id（高水位之下 `>1024`）恒拒；任意 id 序列下不 panic（衔接 v1.15 的 `saturating_add` 修复）。
  - `TokenBucket`：**冻结时钟下恰好只放行 `floor(burst)` 个**，第 `burst+1` 个必拒（O(1) 摊还，无线性扫描）。
  - `NotifyLimiter`：任意时刻放行数**永不超过漏桶上界** `10 + 2t/1000 + 1`。
  - `ClipLimiter`：**时钟回拨不产生额外额度**（单调性），回拨后的放行数不超过回拨前口径。
  - `FileHub`：任意名字经 `sanitize_file_name` 后**非空、非 `.`/`..`、不含路径分隔符与 NUL**；**永不自接受**（未点击接受前 `is_accepted` 恒假）；洪泛 offer 时**在途表有界**（`INCOMING_MAX = 256`），不再无界增长。
  - `ResumeTable`：`claim` 给出的偏移**恒为 `0 < off < total` 且按 `FILE_CHUNK` 对齐**，并与来源绑定（换源不得蹭偏移）。
  - `NotifyHub`：任意 push/reply/时钟推进序列下去重、回复关联、过期三组不变量恒成立。
  - `ScreenSender`：仅在本机显式同意后 `state().is_streaming()` 才为真——**无本机同意绝不推流**；`ScreenReceiver` 游标单调且 `stats().received` 与实际收到严格一致。
  - `ScreenAssembler`：任意顺序碎片可重组，不完整流被拒，**永不 panic**（衔接 v1.15 的 `checked_mul`/`off >= total` 修复）。
  - `AncsIngress`：任意事件序列不 panic；**未就绪（非 `Ready`）时的回调是惰性的**，绝不让 `PRE_EXISTING` 的 uid 或任何事件泄漏到 `drain_events()`（M4.3）。
- **确定性用例（同文件，7 条，作非空性见证）**：`screen_sender_gate_opens_once_the_local_machine_consents`（证明上面的门控断言**不是空洞真**——同意后确实能进入 streaming）、碎片乱序重组、不完整流拒绝、ANCS 惰性/不暴露既有 uid、冻结时钟去重、`FileHub` 必须先 accept 才能 done、`TcpLimiter` 按 IP 隔离。
- **结果**：**未发现任何新缺陷**——v1.15 的两处修复在状态机层面同样成立，新属性未再击穿。全量 `cargo test` 由 251 升至 **269 全绿**；新测试位于既有 `cargo test --workspace --locked` 步骤，CI 无需新增 job。
- **诚实边界**：覆盖的是**确定性状态机**（时钟与随机源全部注入）；`libfuzzer`/`cargo-fuzz`（需 nightly）、真机采集/注入、双桌面物理往返仍是 Manual-required。本层不冒充 coverage-guided fuzz，也不冒充真机验证。

**1.16 相对 1.15（落地 §13 的 `proptest` 与 `fuzz` 两层：解码器属性测试 + 确定性变异语料）**

- **补上了 §13 声明的两层**：此前 §13 表已列 `proptest`（控制 JSON 往返）与 `fuzz`（内层帧、ANCS tuple），但仓库里**只有单测/向量/集成**三层真实存在。v1.16 把 `proptest` 与 `fuzz` 落成可复现、可在**稳定工具链**与 CI 上跑的测试，不再依赖 nightly。
- **`proptest`（core，`crates/tetherly-core/tests/decoders.rs`，27 条）**：以策略生成任意输入，对**每个入站解码器**断言「永不 panic」，并对**每个可编码消息**断言「往返一致」。never-panic：`InnerFrame::decode`、`ScreenFrame::decode_payload`、`ControlMsg::decode_payload`、`ScreenAssembler::push`、`InputEvent::from_frame`、`NotificationEvent::parse`、`Hello::from_bytes`、`DeviceId::parse`、`parse_pin`、`sanitize_file_name`、JSON 控制面 `from_payload`、`DataAssembler`（附 8 KiB 上界断言）。往返：notify push/dismiss/reply、clip set、file offer/decision/done、caps update、input event、screen frame、control msg、hello。分片：任意顺序重组、不完整流拒绝、载荷上限。
- **`proptest`（net，`crates/tetherly-net/tests/decoders.rs`，7 条）**：`read_len_prefixed`/`read_tagged`/`read_header` 对任意字节**永不 panic**；oversized 长度前缀被拒（`MAX_OUTER` 之上，防对端驱动的分配 DoS）；合法头往返；`TFL1` 旧格式读作 `start=0`；错误 magic 拒绝。`&[u8]` 直接作 `AsyncRead`（`tokio` 实现），`prop_assert!` 不入 `async` 块，一律用 panic 式断言。
- **`fuzz`（core，`crates/tetherly-core/tests/fuzz_decoders.rs`，2 条）**：因只有 stable 工具链（无 libfuzzer/`cargo-fuzz`），以**确定性 xorshift 变异器**驱动一个「每个解码器都有合法语料」的种子集（内层帧、全部 7 类控制 JSON、全部 7 类输入事件、整帧 + 分片屏幕帧、控制 msg、ANCS 通知源事件 + 数据源 tuple、规范 Hello JSON），施加 6 类变异（位翻、极端字节、截断、扩展、多字节覆写、拼接）后喂给**全部解码器**，`catch_unwind` 捕获 panic。`mutating_the_valid_corpus_never_panics_a_decoder`（固定种子 `0x9E3779B97F4A7C15`，每种子 512 次、每 4 次叠加，稳定可复现）+ `adversarial_edge_inputs_are_rejected_cleanly`。
- **结果**：**未发现任何解码器缺陷**——v1.15 的两处修复成立，新属性未再击穿。全量 `cargo test` 由 205 升至 **251 全绿**。新测试位于既有 `cargo test --workspace --locked` 步骤，CI 无需新增 job。
- **诚实边界（Manual-required 顺延）**：**libfuzzer/`cargo-fuzz`（需 nightly）仍是 Manual-required 的后续**；本轮的确定性变异驱动是**稳定工具链下的诚实替代**——覆盖面与可复现性明确写在用例里，不冒充 coverage-guided fuzz。

**1.15 相对 1.14（对端可控输入健壮性加固：重放窗口溢出 + 屏幕碎片越界）**

- **`ReplayGuard` msg_id 溢出**：`check_and_record` 里的 `msg_id + 1024 < self.next` 对**对端可控的 `u64`** 直接相加——`msg_id = u64::MAX` 时在 debug 下 `attempt to add with overflow` panic、在 release（`overflow-checks` 关闭）下回绕为「远古 id 被当成 fresh 接受」，削弱重放防护。改为 `msg_id.saturating_add(1024) < self.next`；语义不变（`recent` 1024 项窗口仍兜底）。补两条回归：`rejects_id_far_behind_the_high_water_mark`、`extreme_peer_ids_do_not_overflow`。
- **`ScreenAssembler::push` 越界下溢**：`off = idx * SCREEN_FRAG_CHUNK_MAX` 后直接 `self.total - off`，未校验 `off < total`——对端用合法 `total`（例如 16）配 `idx = 1` 即令 `total - off` 下溢（debug panic / release 得巨大 `expected` 而错配）。改为 `checked_mul` + `off >= self.total` 直接拒为 `ScreenFragment`，不分配、不 panic。补回归 `assembler_rejects_an_index_past_the_stream_end`。
- **范围**：两处均为**入站防护**缺陷（远端可触发），不改变任何线格式、端口、状态机或对外语义；`cargo test` 由 202 升至 **205 全绿**。

**1.14 相对 1.13(凭据库后端化:macOS Keychain + Linux libsecret)**

- **`cred` 后端化**:`crates/tetherly-node/src/cred.rs` 拆为**按目标平台选择的后端**——`cred/windows.rs`(Credential Manager,已 v1.13)、`cred/macos.rs`(**Keychain 泛型密码**,`security-framework`)、`cred/linux.rs`(**Secret Service / libsecret**,`secret-service`,D-Bus)、`cred/other.rs`(无库目标回退)。每个后端文件**自包含**(只 import 绝对路径),因而可在**本机交叉编译验证**其目标平台的编译正确性;`vault_available()`/`vault_store()`/`vault_delete()`/`load_or_create_identity()` 语义不变,`.ref` 指针与 `identity.bin` 回退对所有平台一致。
- **macOS**:`tetherly.identity`(service)/ `<device_id>`(account)经 `set_generic_password`/`generic_password`/`delete_generic_password`;`errSecItemNotFound` 视作「无条目」而非错误,任何 keychain 失败(无登录 keychain/被锁)一律回退文件,启动不阻塞。
- **Linux/BSD**:`secret-service` **blocking** API,默认 collection 内以属性 `device_id=<id>`、标签 `tetherly.identity` 建项(`replace=true` upsert);无 `org.freedesktop.secrets` 提供者(无 gnome-keyring/KWallet 的 headless)时 `available()==false`,透明回退文件;`default-features=false` + `rt-tokio-crypto-rust`(纯 Rust 密码学,**不引 openssl**)。
- **依赖**:新增 `[target.'cfg(target_os="macos")']`:`security-framework`/`security-framework-sys`;`[target.'cfg(all(unix, not(macos)))']`:`secret-service`。全部 **MIT OR Apache-2.0**(deny 许可检查通过);`unsafe` 仍**仅限** `cred/windows.rs` 的 Win32 FFI,macOS/Linux 后端**无 unsafe**。
- **验证**:本机 `cargo check --target x86_64-apple-darwin` 与 `--target x86_64-unknown-linux-gnu` 对两个后端文件**逐字编译**(经 path-include 探针);探针即实测捕获并修复了 Linux `create_item` 返回 `Item<'_>` 借用于临时值的生命周期错误。**运行时** (真读写 Keychain / Secret Service) 仍 **Manual-required**:CI 的 `macos-latest`/`ubuntu-latest` job 已在真实平台上编译它们,`TETHERLY_VAULT_TEST=1` 可在真机跑一次真库往返。
- **诚实边界**:Windows 已真机验(v1.13);macOS/Linux **仅编译验证 + 平台 CI 编译**,真库运行时读写未在本机验证,标 Manual-required。

**1.13 相对 1.12(Windows 系统凭据库身份存储:Credential Manager + `identity.ref`)**

- **凭据库落地**:新增 `crates/tetherly-node/src/cred.rs`。Windows 上身份 64 字节密钥存为 **Credential Manager 泛型凭据**(`CredWriteW`/`CredReadW`/`CredDeleteW`,`CRED_TYPE_GENERIC`,`CRED_PERSIST_LOCAL_MACHINE`),目标名 `tetherly.identity:<device_id>`;数据目录只留 `identity.ref`(公开 device_id 指针,原子写 0600),**不再落密钥明文**。`unsafe` 仅限 `cred.rs::ffi`(原始 Win32 FFI),与 `platform.rs`/`screen.rs` 既有豁免同规则;新增 windows-sys 特性 `Win32_Security_Credentials`。
- **启动语义**:`cred::load_or_create_identity` 优先凭据库——凭 `.ref`(或旧 `identity.bin` 解析出的 device_id)读回密钥;**旧 `identity.bin` 首次运行自动迁移入库并从磁盘删除**。凭据库不可用/条目缺失/解析失败一律**回退文件**路径,启动永不依赖凭据库。
- **卫生规则(关键)**:凭据库是**机器全局、按 Windows 用户**的存储,只对**已安装数据目录**启用;临时目录节点(全部集成/单元测试)继续用该目录的 `identity.bin`,**绝不写操作者的真实 Credential Manager**。`TETHERLY_VAULT=1/0` 环境变量可覆盖(供测试显式驱动)。
- **测试**:`cred::tests::{file_fallback_roundtrips_on_every_platform, windows_vault_roundtrips_via_the_ref_pointer, installed_data_dir_opts_into_the_vault}`。Windows 上真跑 CredRead/CredWrite 往返(写→`.ref`→读回 device_id 一致→`identity.bin` 不存在→删除后读不到);两项 Windows 测试由 CI 的 `windows` job 覆盖,Unix 上 `vault_available()==false` 走文件回退。
- **诚实边界**:macOS Keychain / Linux libsecret **仍未接**(`vault_available()==false`,回退文件);`identity.bin` 与 `identity.ref` 两种形态在非 Windows/回退路径下并存兼容。

**1.12 相对 1.11(文件跨进程续传:内容寻址 + TFL2 起始偏移 + 持久侧车)**

- **内容寻址 `transfer_id`**:发送方对整份文件取 `sha256`,取前 16 字节十六进制构成 `transfer_id = "tr_" + hex[..16]`(取代随机的 per-transfer id)。同字节 → 同 id → 接收方凭 id 命中其此前落盘的半成品续传;不同字节 → 不同 id → 全新传输,不会误续。
- **core(filexfer)**:新增 `ResumeState{transfer_id,source,total,offset}` 与 `ResumeTable`;`ResumeTable::claim(source,transfer_id,total)` 仅当 **来源匹配 + 大小匹配 + 偏移 >0 且 <总长 且按 64KiB 对齐** 时返回可续偏移,否则 `None`(拒绝伪续)。`put` 将 offset 向下对齐到块边界。`IncomingTransfer` 增 `resume_from: Option<u64>`;`FileHub::offer` 委托新 `offer_resumable(...,resume_hint)`,`accept` 委托 `accept_at(...,resume_from)`(`accept_at` 校验偏移,非法即 `CoreError::ResumeOffset`)。`FileDecision`(JSON)增可选 `resume_from` 字段,新增 `FileDecision::accept_resuming`。
- **net(filechan)**:新线格式 **`TFL2`**(`magic(4) | id | total u64be | start u64be`)在 `TFL1` 基础上加 **起始偏移**;`read_header` 同时接受 TFL2/TFL1(v1 视 start=0)。新 `send_chunks_from(w,token,id,total,start,data)` 仅从 `start` 起发尾块;`recv_chunks_after_header(...,start,...)` 校验对齐与 `out.len()==start` 后追加。**每块 AAD = `id || chan_offset_le`**,把块钉死在其通道位置上 —— 块不可跨续传边界被搬移;nonce 计数 = `chan_offset/64KiB + 1`,续传必须块对齐方能与既有 nonce/AAD 匹配(协议级保证,非约定)。新增 `resumable_offset(path,chunk)` 截断半尾并返回对齐长度。
- **node(runtime)**:发送路径 `offer_and_send_bytes` 改为内容寻址;`FileOffer` 处理时按 `hubs.resume.claim(peer,id,total)` 得 `hint` 走 `offer_resumable`;`FileAccept` 回读 `decision.resume_from` 经 `send_file_data` 从该偏移发尾块。接收路径 `accept_file` 改为**增量落盘**:读头 → 校验 `total`/`start` → 预载既有 `.<transfer_id>.part`(截断到 `start`,拒绝 `aligned<start`)→ `set_len(start)` 后逐块 append + `sync_all`;`on_chunk` 回调即写即持久;中断时 `note_resume` 把块对齐偏移写入 `.<transfer_id>.resume.json` 侧车;完成后 sha256 校验 → 原子 `rename` 到 `inbox/` → 删 `.part`/侧车。**重启续传**:`load_resume_table(data_dir)` 启动时扫 `inbox/.*.resume.json` 重建 `ResumeTable`;若侧车对应的 `.part` 已丢失或短于声明偏移(崩溃撕裂态),**丢弃侧车从 0 重传**,不承诺无法兑现的续传。
- **测试(CI 绿)**:core `resume_claim_requires_source_and_size_and_alignment`、`accept_at_validates_resume_offset`;net `resume_sends_only_the_tail_from_a_chunk_boundary`、`a_chunk_cannot_be_relocated_across_resume_positions`、`resumable_offset_truncates_partial_tail`;phase1 `m1_8_file_resumes_from_persisted_offset`:预置 B 的 `inbox/`(`.part` 前 2 块 + 侧车)后**重启** B(证明重启扫描路径),配对 → offer 回带 `resume_from==2*64KiB` → 接受后 sha256 一致 → `.part`/侧车清除。全量回归见 §13。

**1.11 相对 1.10(notify.reply:桌面快速回复手机通知)**

- **协议落地(不再是 stub)**:crates/tetherly-core/src/frame.rs 新增 NotifyReply{uid,app_id,text},to_frame()/from_payload(),文本按 NOTIFY_TEXT_MAX=4 KiB 截断(不切多字节字符)。TYPE_NOTIFY_REPLY=0x0103 原样保留,MUST_UNDERSTAND 对它不再解除。
- **net 分派**:tetherly-net 的 SessionEvent::ReplyStub 占位删除,改为 SessionEvent::Reply(NotifyReply);反序列化失败即 NetError(与其它 JSON 帧一致)。
- **接收方收件箱**:NotifyHub 增 ingest_reply/take_reply_id/take_reply/reply_views,键为 reply_id(source,app_id,uid)(=sha256),取用即删(快速回复永不二次触发)。空白文本拒绝;同键后写覆盖(幂等)。面向 UI 的 ReplyView 不含正文,只有 take_reply* 才释放文本——沿用「通知正文不进列表视图/日志」的规则;QueuedReply 的 Debug 手写 redact 正文。
- **桌面端(node)**:Node::send_reply / reply_to_candidate(按候选寻址回其来源手机;空文本 CoreError::EmptyReply 本地拒绝)/ take_reply / reply_views;handle_event 收 SessionEvent::Reply 后入箱 + 发 UiEvent::ReplyReceived(只记 text_len,不记正文)。
- **桌面 UI**:ui/index.html 每张候选卡加「快速回复…」输入框 → POST /api/reply {id,text};新增「收到的回复」区 + POST /api/reply/take {id} 取出(正文不落快照)。Snapshot 增 replies:[ReplyViewDto]。
- **Android(手机)**:SessionHandle::take_replies()(Vec<NotifyReply>,一次取空)+ serve 循环解析 TYPE_NOTIFY_REPLY(同 uid+app 覆盖);JNI 新增 nativeTakeReplies(JSON 数组,空则空串;正文为用户内容、禁止写日志);Kotlin NativeBridge.takeReplies()/PhoneReply,NotifyListener.drainReplies() 用通知自身的 RemoteInput action 就地回复(不合成应用未声明的回复;activeNotifications 无匹配则丢弃)。真机触发 RemoteInput 需系统权限/前台通知,Manual-required。
- **测试(CI 绿)**:core 单测 3 条;tests/phase2.rs::m2_6 双节点真实会话桌面→手机 reply 到达且只取一次;crates/tetherly-android/tests/client_e2e.rs::android_client_receives_a_quick_reply_once 真 accept_session 发 reply、客户端 take_replies() 一次拿到。全量 188 passed(原 182)。

**1.10 相对 1.9（Phase 1 Android 原生库：真实身份 + Noise 会话落地）**

- **新 crate `tetherly-android`**（`crates/tetherly-android`）：lib 名 `tetherly_android`，crate-type `["cdylib","rlib"]`；`cdylib` 编出 `libtetherly_android.so`（arm64-v8a），`rlib` 使 host/CI 可跑同一份代码的集成测试。依赖仅 `tetherly-core`/`tetherly-crypto`/`tetherly-net`（`default-features=false`）/`tokio`/`serde_json`/`thiserror`/`tracing`/`zeroize`；`jni` 依赖门控在 `[target.'cfg(target_os = "android")'.dependencies]`，桌面构建不引入。
- **真实身份（取代占位）**：`Identity::from_secrets(id_sk, n_sk)` 生成真 Ed25519 签名密钥 + 独立 X25519 静态密钥（ADR-001），`device_id = DeviceId::from_id_pk(id_pk)`；`identity.bin` 64 字节（temp+rename、`0600`）落盘，`load_or_create_identity` 首启生成、之后加载。**彻底删除** v1.3 的 `idPk = sha256(idSk)` 占位路径。
- **信任持久化**：`trusted.json` 与桌面节点**同构**（`TrustFile { peers: Vec<TrustedPeerDto> }`），两端文件可互换；配对成功即写，resume 无 PIN 重拨复用（ADR-002）。
- **会话客户端**：`AndroidClient::start(addr, pin, once)` 在独立线程跑 `current_thread` tokio runtime，按桌面**相同**的 `dial_session` 路径（首配 Hello + SPAKE2 8 位 PIN，之后 Noise IK resume）拨 45717；`SessionHandle` 暴露 `notify()`/`state_code()`/`connected_peer()`/`last_error()`/`stop()`；指数退避重连 250ms→5s，`TYPE_PING` 回 pong。
- **诚实能力集**：手机 `caps = ["notify","clip","file"]`，**永不**广告 `input`/`screen`（手机不是被控桌面）。落点是 `SessionConfig.caps: Option<Vec<String>>` 显式覆盖（`tetherly-net` 新增字段，默认 `None` 走既有推断）。
- **JNI 薄壳**（`#[cfg(target_os = "android")]` 门控）：`Java_app_tetherly_NativeBridge_nativeAttach/SendNotify/DeviceId/IdPk/State/LastError/Stop`，与 Kotlin `NativeBridge`/`NodeSession` 对接。lib 顶层 `#![deny(unsafe_code)]`，仅 `jni.rs` 以 `#![allow(unsafe_code)]` 作 JNI ABI 的**唯一**豁免。
- **端到端已验（真机）**：`android_probe`（同 crate `--bin`，aarch64-linux-android）推到云手机 `adb shell` 直接跑——两台 Galaxy Fold 分别生成新身份、以 PIN 配对成功、桌面节点收到带验证码的 `notify.push`（`has_otp:true`，正文不入快照/日志）、写双方 `trusted.json`；**换错 PIN 仍连接成功**，证明走的是 Noise IK resume 而非重新配对。
- **CI**：`android` job 先 `dtolnay/rust-toolchain`（target `aarch64-linux-android`）+ `cargo install cargo-ndk` + `cargo ndk -t arm64-v8a -o android/app/src/main/jniLibs build -p tetherly-android --release`，再跑 Gradle；`jniLibs/` 是产物、已入 `.gitignore`（不入库）。

**1.9 相对 1.8（Phase 3 键鼠真机链：SendInput 注入 + LL 钩子捕获）**

- **受控端注入**：`SystemInputSink`（`InputSink` 端口实现）——`SendInput` 绝对移动（1000 单位→65535 归一化）、按钮 mask（`BTN_LEFT=1/BTN_RIGHT=2/BTN_MIDDLE=4`）、滚轮、VK 键盘；`NodeConfig.input_sink` 注入，CI 缺省 `None` 仅观察者。协议级过滤（seq 窗口/座位门控）后经 `MemorySink` 观察者 trace 镜像到真实注入。
- **主控端捕获**：`Node::spawn_input_capture` —— `WH_MOUSE_LL` 恒挂（只读光标做过边检测）、`WH_KEYBOARD_LL` **仅远程座位时挂**（本地键盘隐私）、注入事件按 `LLMHF/LLKHF_INJECTED` 过滤防单机回环；喂给与 CI 相同的 `drive_input*` 路径。**注入探针实测**：过边后受控端光标真实移至左缘 (0,539)、中心移动精确、Win/Esc 注入生效。
- **Manual-required**：双桌面 100 次物理往返（需两台交互桌面）。

**1.8 相对 1.7（Phase 5 真机闭环：DXGI 采集 + zstd 编码 + BitBlt 呈现 + p95）**

- **真机采集**：`crates/tetherly-node/src/screen.rs` 的 `DxgiSource` 实现 Windows DXGI Desktop Duplication——`D3D11CreateDevice`（BGRA）→ `IDXGIDevice::GetAdapter` → `IDXGIAdapter1::EnumOutputs(0)→IDXGIOutput1::DuplicateOutput` → `AcquireNextFrame(0)`（`DXGI_ERROR_WAIT_TIMEOUT` 映射为 `Ok(None)`，桌面未变时不产帧）→ staging texture `CopyResource` + `Map(READ)` → 行尾裁剪成 tightly-packed BGRA8 → `ScreenFrame::raw`。COM 单线程模式 `CoInitializeEx(MULTITHREADED)`；D3D11/DXGI 对象经 `unsafe impl Send/Sync`（单 pump 串行 + `Mutex`），是 D3D11 的标准单线程用法。
- **真机编码（可换端口）**：`ZstdCodec`（无损，level 3）实现 `ScreenCodec`；`EncodedSource<S>` 是 `ScreenSource` 装饰器——每帧压缩，压不小就发原帧（病态编码器不可能放大传输）。生产默认链为 `EncodedSource<SystemSource>`；未来 OpenH264/rav1e 作为同一端口替换。**先无损帧 + 后接编码器**的落点。
- **真机呈现**：`SystemSink` 消费侧用 GDI——`CreateWindowExW` + `CreateCompatibleDC` + `CreateDIBSection`（32bpp top-down）+ `BitBlt`（`SRCCOPY`）。**同步 BitBlt**（本线程无消息泵，不依赖 `WM_PAINT`），每帧直写窗口 DC 立即显示；尺寸变化时重建 DIB。
- **分片传输 + 重组**：`ScreenFrame::to_frames()` 把超 64 KiB 内层预算的整帧拆成 `TMF1` 分片（header 24 字节：magic+total+index+count+seq）；`ScreenAssembler::push` 重组（跨流重置、单流内存上限 `SCREEN_PAYLOAD_MAX`，谎报 count 被拒而非分配）。整帧 `TMV1` 走单帧快路径，小帧线上字节不变。
- **客户端解码链**：`attach_screen_client` = `ScreenAssembler` 重组 → 若 `compressed` 则 `screen_codec.decompress(expected_len)` 校验解压长度 → `ScreenReceiver::apply` 校验 seq 窗口 → `screen_sink.present`。呈现目标 `screen_sink` 默认 = 观察者 `MemoryScreenSink`（CI 断言），生产注入 `SystemSink`。
- **p95 帧间隔预算**：`tetherly-node` 加 `ScreenIntervalTracker`（wall-clock，微秒，bounded 1024 样本），`screen_send_frame` 每次成功入队记录一次；`Node::screen_latency_p95_us()` 暴露，UI `Snapshot.screen.p95_frame_interval_us` 输出。core 保持无时钟（确定性）。
- **生产默认**：`Node::start` 默认 `screen_source = EncodedSource<SystemSource>`（DXGI+zstd）、`screen_codec = ZstdCodec`、`screen_sink = 观察者`；`tetherly run` 注入 `SystemSink` 并起 16ms 帧 pump（非 Streaming 状态直接跳过 grab，空闲零开销）。`--screen-adb <serial>` 把源换成 `AdbSource`（云手机屏幕）。测试注入 `MemoryScreenSource`（原帧），路径不受影响。
- **AdbSource 跨设备真机 E2E（云手机 → Windows）**：`AdbSource`（`adb exec-out screencap -p` → PNG 解码 → BGRA8，PNG 字节不变返回 `Ok(None)`）接入既有链路实测：1063 帧发出 / 1060 收达 / 0 丢帧，每帧解压后精确 720x1280x4 BGRA，GDI 窗口画面与真机屏 ±8 容差 97.6% 一致（残差 = loading 动画相位 + DWM 色彩管理），p95 ≈0.9–1.45s（`screencap` 源速率主导）。真机流程暴露并修复两处缺口：`/api/screen/start` 现自动拨号 screen 通道（单对端时；多对端诚实拒绝）；visitor 接收计数 `screen_rx_stats` 接入 Snapshot。
- **测试**：`tests/phase5.rs` 4 个测试全绿（M5.1 帧序 1..=5 严格单调、M5.2 Start-before-allow 拒绝计数、M5.3 对端离开回 `Idle` 并重置 seq、M5.4 45720 拒未配对 dial）。`cargo fmt --check` / `clippy -D warnings` / `cargo test --workspace` 全绿（core 116 + phase1 7 + phase2 4 + phase3 4 + phase4 10 + phase5 4 + …）。
- **Manual-required（收窄）**：真实开放编码器 OpenH264/rav1e（端口已就位、未接入）；DXGI 采集端的真实像素捕获与 Windows 跨桌面 100 帧 p95（需交互桌面）；macOS CGImage/CGWindow、Linux X11 SHM（Wayland 仍标「不支持」）。**DXGI 采集 / zstd 编码 / BitBlt 呈现 / p95 计量 / 跨设备（云手机源）真机闭环已从 Manual-required 转为已实现**。

**1.7 相对 1.6（Phase 5 远程屏幕 CI 切片）**

- 新增 `tetherly-core::screen`（**无 OS API**）：屏幕帧魔数 `TMV1`、proto 1、`SCREEN_SEQ_WINDOW=65536`、`PixelFormat{Bgra8,Rgba8}`、`ScreenFrame` 编解码（header 31 字节：magic+proto+seq+rect+fmt+compression+stride）、`ControlMsg{Start,Stop,Ack{seq}}` 编解码（14 字节）、`ScreenSender`（单调 seq + consent 门控 + push/drain/refused 计数）、`ScreenReceiver`（seq 窗口丢旧/拒大跳/bounded trace）、`MemoryScreenSource`/`MemoryScreenSink`（确定性 CI 假实现）、`ScreenSink`/`ScreenSource` 端口。
- 新端口 **45720**（`SCREEN_PORT`）：Noise IK resume only；`screen_only: true` 的会话在 `HelloDisposition::Pair` 时返回 `NetError::ScreenRequiresTrust`——配对永远只在 45717。Hello 完只允许已配对对端。
- **强制本机同意（M5.2）**：对端连上 45720 仅置 `ScreenState::Requested`，必须本机显式调用 `Node::screen_allow()` 才进入 `Allowed`；对端 `Start` 在 `Allowed` 前被拒且计数。`ScreenSender::push` 仅在 `Streaming` 状态出帧。
- **RustDesk 仅作架构借鉴**：RustDesk 客户端与服务端均为 **AGPL-3.0**，不得以任何形态链入 `Cargo.toml`；`deny.toml` 加 `hbb_common` / `rustdesk` / `rustdesk-server` / `scrap` 为结构化红线。借鉴的公开工程常识（分离采集+编码器 swap+P2P+中继）**不**包含居会合服务器（hbbs/hbbr），匹配 §2.3 禁云账号。
- `tetherly-node`：`NodeConfig.screen_port`（默认 45720）+ `screen_source: Option<Arc<dyn ScreenSource>>`（测试注入 MemoryScreenSource）；`Shared` 加 `bound_screen: AtomicU16`、`screen_out`、`screen_engine: Mutex<ScreenSender>`、`screen_source: Arc<dyn ScreenSource>`、`screen_sink: Mutex<MemoryScreenSink>`、`screen_ctl`；`spawn_listeners`/`spawn_overlay` 各加 45720 绑定与接受；`attach` 的 `caps.update` 加 `"screen"` + `screenport=`；`Node::screen_allow/revoke/start/stop/state/stats/send_frame/sink_snapshot/port/addr`。
- `crates/tetherly-node/src/screen.rs`：`SystemSource`/`SystemSink`按目标 `#[cfg]` 选择后端。**v1.19**：`MacSource`（`screen/macos.rs`,CoreGraphics 采集；呈现需 AppKit `NSWindow`，诚实 `ScreenRefused`）与 `X11Source`（`screen/x11.rs`,MIT-SHM 采集 + plain `GetImage` 回退；呈现 `PutImage`）。另有 Windows `DxgiSource`（DXGI Desktop Duplication）与 `AdbSource`（adb 云手机）；其余目标与 Wayland 诚实 `ScreenRefused`。**真机采集 Manual-required**。
- UI：`ui/index.html` 加「远程屏幕（实验）」区——「① 需本机点击允许才推流；② Wayland 暂不支持；③ 真机采集为 Manual-required」。路由 `POST /api/screen/allow|start|stop`。`Snapshot` 加 `screen_state` 和 `screen` 计数器。
- `tests/phase5.rs`：M5.1 consent-then-start 帧序严格递增 1..=n、M5.2 Start-before-allow 被拒且计数、M5.3 对端离开→`Idle` 重置 seq、M5.4 45720 拒绝未配对 dial。`cfg()` 注入 `MemoryScreenSource`。
- **Manual-required**：真机 DXGI Desktop Duplication / WGC（Windows）、CGImage（macOS）、X11 SHM（Linux）采集；真实编码器（OpenH264/rav1e）；真实窗口呈现（Windows HWND / macOS CGWindow / Linux Wayland portal）；p95 帧间隔预算真机验证。M5.5 硬件编码 / 真机注入「未做」。

**1.6 相对 1.5（Phase 4 iPhone ANCS CI 切片）**

- 新增 `tetherly-core::ancs`（**无 OS API**）：Notification Source 8 字节事件解析、Control Point 编码（`GetNotificationAttributes` / `GetAppAttributes` / `PerformNotificationAction`）、Data Source **按字节**分片重组、`AncsIngress` 状态机、`AncsTransport` 端口与 `MemoryAncsTransport`。
- §9.4 硬约束逐条落地：先订 Data Source 再订 Notification Source；`on_notification_source` **只入队不写** Control Point（写在 `tick`）；Control Point 串行（同一时刻最多一个未应答请求）；分片按字节拼满；静默首响应重试一次后放弃；断线指数退避（500ms→30s 封顶）；uid 去重。
- M4.3：`PreExisting` 标志的通知一律丢弃，重连不连弹；uid 与 app 名缓存在重连后保留。
- M4.2：动作只由 `ancs_actions(app_id, allowlist)` 决定——恒有 `copy`/`dismiss`，**只有**白名单命中本地 scheme 时才加 `open`。ANCS 载荷里的 url 从不被读取或执行；`NotifyPush::truncate` 仍会剔除含 `://` 的动作。
- `tetherly-node`：`NodeConfig.ancs`（`Option<Arc<dyn AncsTransport>>`）、`Node::ancs_connect/ancs_notification_source/ancs_data_source/ancs_tick/ancs_disconnected/ancs_state`、`set_open_allowlist`/`add_open_rule`、`open_candidate(id, &dyn Opener)`。`Opener` 是端口：生产用 `SystemOpener`（`rundll32 url.dll,FileProtocolHandler` / `open` / `xdg-open`，无 shell），CI 用 `MemoryOpener`。
- UI：配对向导拆成两步——①「系统蓝牙里连 iPhone（ANCS）→ 启动 ANCS」②「与本机/其他设备 SPAKE2 配对」。新增 `/api/ancs/connect`、`/api/ancs/allow`、`/api/open`。
- `tests/phase4.rs`：M4.1 模拟时延预算、M4.2 白名单门控 + 载荷 url 不打开、M4.3 重连不连弹、Control Point 串行与回调线程不写、跨 UTF-8 分片重组、`Removed` 清候选、无传输时惰性、退避拒绝热重连。
- **Manual-required**：真实 BLE GATT（Windows `windows` crate / macOS CoreBluetooth / Linux BlueZ）、物理 iPhone 的 p95 < 2s、蓝牙关 30s 再开的真机复现。M4.4 / M4.5（可选 iOS App、无 VPN）本切片不涉及。

**1.5 相对 1.4（Phase 3 原生键鼠 CI 切片）**

- 键鼠走独立 TCP **45719**。Hello 之后只允许 `ResumeNoise`；未知对端返回 `InputRequiresTrust`，配对仍只在 45717。
- 载荷魔数 `TIN1`、proto 1、`seq` u64be、屏幕单位 0..=1000。内层 type `0x0401`–`0x0407`，带 `MUST_UNDERSTAND`。控制面 JSON 表仍不含这些 type，也没有 `input.*` JSON。
- 引擎在 `tetherly-core`（无 OS API）。节点用 `MemorySink` 注入；`caps.update` 广告 `inputport=`。过边第一帧 `x=0` 是进入，不是离开；只有先进屏幕内部再回到 `x=0` 才回 server。
- `tests/phase3.rs`：未配对 45719 拒绝、过边+按键跟随、对端断开光标回本机、键鼠聚焦时 `clip.set` 仍走 45717。
- 真机光标注入（Win SendInput / macOS / Wayland portal+libei）与双桌面 100 次物理往返：**Manual-required**。M3.4 DeskFlow 网关保持 experimental、默认关，**不链接** DeskFlow / Lan Mouse / KDE 源码。

**1.4 相对 1.3（Phase 2 EasyTier 侧车 CI 切片）**

- 仍 **不** 把 `easytier*` 放进 `Cargo.toml`。发现走本机 RPC `127.0.0.1:15888`（2s）或 `easytier-cli -o json peer list`（失败则 `peer`），任意 JSON walker + 文本 IPv4 扫描；缺侧车是成功（LAN only，M2.4）。
- 虚网 bind 与 LAN bind **分开**：LAN/mDNS 永不在 EasyTier TUN 上 advertise。overlay 监听仍禁止 `0.0.0.0`/`::`；无 TUN IP 时 bind 空列表成功。
- 路径：`PathKind::Lan` 赢过 overlay attach；overlay 会话结束只拆自己的 `sess_addr`，不拆仍活着的 LAN 条目（M2.2）。
- UI：缺侧车显示「仅局域网（未检测到 EasyTier 侧车，不影响 LAN）」；在线显示「虚网在线」。CLI：`--overlay-cidr`、`--no-overlay`。
- `tests/phase2.rs`：M2.3 停侧车不崩、M2.4 未装侧车 Phase 1 不变、M2.5 无 mDNS 靠 extra_peers 单播、M2.2 重复 overlay 扫描仍保 LAN。夹具 CIDR 用 `10.199.199.0/24` 隔离本机真实 EasyTier TUN。
- M2.1 Android 蜂窝 ↔ 电脑通知 p95 < 3s：**Manual-required**（真机蜂窝 + 侧车 soak）。

**1.3 相对 1.2（Phase 1 LAN 桌面 + Android）**

- 新增 `tetherly-node`：身份/信任落盘、LAN bind（禁止 `0.0.0.0`、跳过 EasyTier `10.144.144.0/24`）、mDNS、`notify.push` / clip / file 控制面、文件数据走 **45718**、本机 HTTP UI **45716**（仅 127.0.0.1）。
- 桌面壳本阶段是 loopback HTTP + `ui/index.html`，**不是** Tauri/WebView2。`src-tauri` 标为下一桌面壳迭代，避免 CI 依赖 WebView2。
- Android：`NotificationListenerService` + 前台服务 + gradle CI。`libtetherly_android.so` / 真 Ed25519 JNI **尚未构建**；无 `.so` 时不擅自 raw-TCP。协议路径由 `tests/phase1.rs` 双节点 loopback 覆盖。
- `cargo deny` 允许 **BSL-1.0**（Boost，OSI/FSF）：`arboard` → `clipboard-win`。仍拒绝 GPL/AGPL/LGPL；`easytier*` 仍 ban。
- M1.2 / M1.3（MemoryInsertor）/ M1.4 Win 回环 / M1.5 persist 重拨 / M1.6 日志脱敏 / M1.7 文件确认+10MiB sha256：CI 全绿。M1.1 物理 Android p95(v1.18 真机实测关闭)、Win+Android 8h、真记事本 UIA、macOS M1.1：**Manual-required** / 「mac 下一迭代」；v1.24 把文件单文件上限 `FILE_MAX`（报价即拒、拒绝不拆会话）与网卡分类（`BindClass` 单一真理源 + 可解释）补实。

**1.2 相对 1.1（Phase 0 评审修订）**

- PairBind AEAD nonce 按 TCP 角色拆开（`tetherlybndI` / `tetherlybndR`），禁止同一 `(pair_key, nonce)` 加密两份明文。
- 状态机改为 PairBind → NoiseIK → **成功后**写信任库；Hello `name`/`caps` MITM 不得留下信任记录。
- 内层帧带 `u64 msg_id`；握手 20s 超时；新 TCP 每 IP 30/min。
- `NotifyPush` 截断必须落在 UTF-8 字符边界。PIN 生成在 `getrandom` 失败时 fail-closed。

**1.1 相对 1.0（调研修订）**

- EasyTier 许可证已核实为 **LGPL-3.0**，侧车策略从「待查」变成硬约束；默认虚网是 `10.144.144.0/24` 不是 `10.126.126.0/24`。
- 跨网发现改为 **EasyTier 节点单播**，不依赖 TUN 上的 mDNS（组播经常过不去）。
- SPAKE2 必须把 Hello 里的身份公钥绑进口令派生；PIN 先经 **Argon2id** 再进 PAKE，避免抓包离线爆 8 位码。
- `PairBind` 必须在 SPAKE2 会话密钥下加密，不能明文送 X25519 公钥。
- v1 **不在线上单独发 `otp.candidate`**：转发 `notify.push`，各展示端自己提取。短信正文里本来就有码。
- 键鼠 **原生 Noise 端口为主**；DeskFlow 网关要讲人家的 **TLS（默认端口 24800）**，Phase 3 退出不依赖互通。
- Phase 0 crate 收成四件套；ANCS / 键鼠 / uniffi 按阶段再加。
- 分清两套配对：系统蓝牙（ANCS）≠ Tetherly SPAKE2（IP 会话）。
- 锁定密码学 crate：`snow`、`spake2`、`argon2`、`ed25519-dalek`、`x25519-dalek`、`mdns-sd`。

---

## 1. 怎么用这份文档

| 角色 | 先读 |
|---|---|
| 写第一行代码 | §3 ADR、§6 配对、§7 帧、§8 Phase 0 范围、§12 M0.* |
| 加能力 | §5、§8 插件、§10 反模式 |
| 验收 | §12，数字必须可测 |
| 安全 | §4、§6、§11 |

分层：ADR → 协议 → 里程碑 → Phase 5 候选。ADR 不经修订记录不得违反。

---

## 2. 产品

### 2.1 一句话

一个人的 Windows / macOS / Linux / Android / iPhone 在同一信任圈里：通知和验证码到电脑、剪贴板和文件可传、多台桌面共用一套键鼠。同一局域网直连；跨网靠用户自己的 EasyTier 虚网。

### 2.2 必须守住的差异

| 点 | 现实 |
|---|---|
| 跨网 | 桌面与 Android 走 EasyTier **侧车**；iPhone 跨网不是 v1 |
| 验证码 | 本机打分提取；写入必须当次点击 |
| 键鼠 | 桌面 ↔ 桌面；自有 Noise 端口。Windows 真机链 v1.9 已落地（SendInput 注入 + LL 钩子捕获，键盘 hook 座位门控）；macOS/Linux 下一迭代。DeskFlow 仅实验网关 |
| iPhone | 电脑用 ANCS/BLE 收新通知，再经 IP 会话转给其他已配对电脑 |

### 2.3 非目标（v1）

- 远程桌面 / 屏幕镜像（**v1 非目标已由 v1.7 Phase 5 取代**：`tetherly-core::screen` 帧+控制编解码、强制本机同意、45720 独立端口的 CI 切片；**v1.8 已落地** Windows DXGI 采集 / zstd 编码 / BitBlt 呈现；macOS/Linux 与开放编码器仍 Manual-required。完整远程桌面仍不在 v1 承诺范围）
- 云账号、云端存通知或验证码
- iOS 被控
- Tetherly 进程内嵌 EasyTier 或 Network Extension VPN
- 兼容 KDE Connect
- 把对端 URL/正文当命令执行
- Android 10+ 后台静默读剪贴板
- 在 EasyTier TUN 上依赖组播/mDNS
- 把 8 位配对码当「高熵密钥」而不做 PAKE + KDF

### 2.4 端能力

| 能力 | Win | macOS | Linux | Android | iPhone |
|---|---|---|---|---|---|
| LAN 会话 | 听 + 拨 | 听 + 拨 | 听 + 拨 | 主要拨号 | 可选 App 拨号 |
| EasyTier 跨网 | 侧车，单播 | 同左 | 同左 | 侧车，拨虚网 IP | ❌ v1 |
| 通知上行 | — | — | — | NotificationListener | ANCS → 电脑 |
| 通知展示 | ✅ | ✅ | ✅ | ✅ | 可选 App |
| OTP 提取 | 展示端本地 | 同左 | 同左 | 同左 | 电脑提取后展示 |
| OTP 填入 | UIA，点击 | AX，点击 | AT-SPI，点击 | 无障碍，点击 | 仅复制 |
| 剪贴板 | ✅ | ✅ | ✅ | 按钮发送 | 前台 |
| 文件 | ✅ | ✅ | ✅ | SAF | 可选 App |
| 键鼠 server/client | ✅ | ✅ | Wayland beta | ❌ v1 | ❌ |
| 远程屏幕 server | ✅（v1.8 真机闭环） | **已实现（v1.19）**:`MacSource` CoreGraphics 采集;呈现需 AppKit NSWindow（诚实拒绝） | **已实现（v1.19）**:`X11Source` MIT-SHM 采集 + `PutImage` 呈现;Wayland 不支持 | ❌ v1 | ❌ |

话术：桌面全功能；Android 是信源和文件端；iPhone 是通知/验证码发送源。

---

## 3. 约束决策（ADR）

### ADR-001 身份：Ed25519 签名 + 独立 X25519 静态密钥

首次启动生成：

- `id_sk` / `id_pk`：Ed25519，只用于签名
- `n_sk` / `n_pk`：X25519，只用于 Noise

```
device_id = "tdev_" + base32lower(sha256(id_pk)[0..10])  // RFC 4648，无填充
```

信任库存对端 `id_pk` 与 `n_pk`，校验 `device_id` 必须能从 `id_pk` 重算。禁止随机 UUID 当长期身份。禁止把 Ed25519 标量直接当 X25519。

密钥:**Windows 系统凭据库已接(v1.13)**;否则用户目录文件 ACL 仅本人;进程内 `zeroize`。

### ADR-002 首次配对：Argon2id + SPAKE2；之后 Noise IK

1. 发起端显示 **8 位数字**，TTL **3 分钟**，最多 **5 次失败**，然后对该对端或源 IP **锁 15 分钟**。
2. `pwd = Argon2id(PIN, salt, m=64MiB, t=3, p=1)`  
   `salt = sha256("tetherly-pair-v1" || min(id_pk_A, id_pk_B) || max(id_pk_A, id_pk_B))`  
   这样 PIN 与 Hello 里已经交换的身份绑定，中间人换公钥会让 PAKE 失败。
3. SPAKE2 用 crate **`spake2`**（RustCrypto 系，Apache-2.0/MIT），群与密码套件在 Phase 0 写入 `tetherly-crypto` 测试向量后冻结。
4. SPAKE2 成功得到 `pair_key`。用 `pair_key` 加密 **PairBind**（双方 `n_pk` + 对 `id_pk||n_pk||device_id` 的 Ed25519 签名）。ChaCha20-Poly1305 nonce 按 TCP 角色拆开：发起者 `tetherlybndI`，响应者 `tetherlybndR`。禁止同一 `(pair_key, nonce)` 加密两份明文。
5. PairBind 校验通过后做 Noise IK。**信任库只在 Noise 进入 transport 之后写入**；prologue 失败必须不留信任记录。别名用已签名的 `device_id`，不用未认证 Hello `name`。
6. 之后每次连接：**Hello（明文）→ Noise_IK**（已知对端 `n_pk`），不再走 PIN。

禁止：6 位 PIN、HMAC(双方公钥) 当同一显示码、自绘 TLS、明文 PairBind、把 PIN 当 Noise 预共享密钥而不做 PAKE。

### ADR-003 业务只走一层 Noise

模式字符串冻结：

```
Noise_IK_25519_ChaChaPoly_BLAKE2s
```

实现 crate：**`snow`**（纯 Rust Noise）。prologue = `sha256(canonical_hello_A || canonical_hello_B)`，双方按 `device_id` 字典序拼接，防止 Hello 被换。

Hello / SPAKE2 / 密文 PairBind 在 Noise 之外；此后控制面与文件面都是 Noise transport。不再叠 TLS。

### ADR-004 ANCS 是 Ingress，不是 Transport

`Transport` 只有 IP：LAN TCP、Phase 2 虚网上的 TCP。iPhone 通知由**电脑**上的 ANCS 客户端变成 `PhoneNotification`，进 `NotifyHub`。禁止把 BLE 当 RPC。

iPhone v1 **不做** Tetherly SPAKE2，除非可选 iOS App（M4.4）。系统蓝牙配对 ≠ 应用配对。

### ADR-005 键鼠走独立 Noise 端口

- 主路径：`45719/tcp`，Noise IK（可复用已配对 `n_pk`），自有长度前缀事件帧。不进 JSON 控制面。
- DeskFlow 网关：默认 **关闭**。官方默认端口 **24800**，现代 DeskFlow **默认 TLS**。要互通就必须实现其证书/指纹模型，并避开已知 TLS 接受失败卡死问题。Phase 3 **退出不要求** 与官方 client 互通。
- 禁止链接 DeskFlow / Lan Mouse / Barrier 源码。

### ADR-006 EasyTier 侧车（已核实 LGPL-3.0）

详见 [adr/006-easytier.md](adr/006-easytier.md)。`easytier*` 不准进依赖图。跨网用官方进程；发现用虚网 **单播**，不用 mDNS。

### ADR-007 iOS v1 无应用内 VPN

可选 iOS App 只跑 LAN。不写「再开 EasyTier VPN」——双 VPN 互斥，用户会按文档去做然后骂你。

### ADR-008 打开应用的 URL 只来自本机规则

`(app_id glob → 本机 url 常量)`。丢弃对端一切 url/command。验证码按钮只复制。

### ADR-009 许可证

| 依赖 | 允许 |
|---|---|
| MIT / Apache-2.0 / BSD / BSL-1.0 | 链接（BSL = Boost，OSI；`arboard`/`clipboard-win` 需要） |
| EasyTier LGPL-3.0 | 仅侧车 |
| DeskFlow / Lan Mouse / KDE Connect 源码 | 禁止复制实现 |
| `cargo deny` | 拒绝 GPL/AGPL/**LGPL** 进入 Tetherly 依赖图；ban `easytier*` |

Tetherly 自身：**Apache-2.0 OR MIT**。

### ADR-010 验证码写入必须用户当次触发

探测可常驻；`insert()` 只挂按钮/快捷键。日志只记 `code.len()`、score、规则 id。

### ADR-011 v1 线上不单独发 OTP

展示端对 `notify.push.body` 本地跑提取器。不新增 `otp.candidate` 消息（码已在正文里，再发一遍只增加不一致）。

### ADR-012 桌面听、手机拨

LAN 上桌面默认 `listen=true`。Android 默认只拨号（蜂窝网入站困难）。桌面之间全互拨已发现的地址。防火墙默认只绑专用网/已知虚网，不绑公网 `0.0.0.0` 除非用户改配置。

### ADR-013 显示 PIN 必须作用于「活着的」节点

PIN 只对**生成它的那个进程**有意义:`Node` 的 `shared.pin`(含 `pin_created_ms`)是纯进程内状态,**从不持久化**。因此在**另一个**进程里生成并打印的 PIN 永远无法被正在运行的守护进程接受(其 `pin_created_ms=0` ⇒ 任何 dial 都判 `PinExpired`)。

结论:`tetherly pin` **不得**自起节点生成 PIN;它必须是运行中节点的**回环客户端**(`POST 127.0.0.1:<ui_port>/api/pin`,默认 45716),无守护进程监听时**诚实非零退出**。UI「显示 8 位 PIN」按钮走同一条路由。v1.18 前 `Cmd::Pin` 违反此条(在一次性进程里 `Node::start` + `generate_pin`),真机排查时表现为「PIN 显示了却一直 expired」,已修复并加回归测试。

**同一结论推广到全部 CLI 控制面(v1.20)**:`tetherly dial` 与 `tetherly notify` 同样不得自起节点——前者的第二次 `Node::start` 会与运行中守护进程抢同一批 LAN 端口而死于 `no LAN address bound`,后者的 `inject-notify` 在本进程构造推送、永远到不了对端。两者均改为活节点的回环客户端(`POST /api/dial` / `POST /api/notify`),把 PIN 装置、拨号、persist、`send_notify` 全部交给持有进程内状态的那个进程;`inject-notify` 已删除,由 `notify` 取代。UI「连接」按钮一直走的 `/api/dial` 即此路径。

---

## 4. 威胁模型

| 对手 | 能力 | 保证 | 不保证 |
|---|---|---|---|
| LAN 路人 | mDNS、扫端口、改 Hello | 未完成 SPAKE2 读不到 PairBind 与业务 | 设备名、id 指纹会广播 |
| 配对 MITM | 换身份公钥 | salt 绑定双方 `id_pk`；PAKE 失败 | 用户把 8 位码念给攻击者 |
| 抓 SPAKE2 包离线猜 PIN | 约 10^8 空间 | Argon2id 64MiB×3 次拉高成本 + 在线锁定 | 超算长时间离线（可接受） |
| 已配对恶意节点 | 垃圾通知/文件 | 限流；文件要确认 | 能看见你允许转发的通知正文 |
| EasyTier 中继 | 密文 | Noise 对中继保密 | 流量形态 |
| 已解锁电脑 | 读密钥文件 | ACL / 凭据库 | 全盘加密是用户的事 |
| 恶意通知正文 | 钓鱼链接 | 截断；不执行 URL | 用户仍可能被骗去复制码 |

崩溃报告默认无载荷。无遥测。

---

## 5. 架构

```
┌──────────────────────────────────────────────────────────┐
│ UI：Phase 1 = 本机 HTTP 127.0.0.1:45716 + ui/index.html   │
│     下一迭代：Tauri/React（src-tauri）；Android；可选 iOS   │
├──────────────────────────────────────────────────────────┤
│ NotifyHub  Clip  File     │ InputEngine（45719，Phase 3）  │
├──────────────────────────────────────────────────────────┤
│ Session：Hello → [SPAKE2|Noise IK] → caps                  │
├──────────────────────────────────────────────────────────┤
│ LanTcp（mDNS 仅真实 LAN） │ Phase2：虚网单播（无 mDNS）     │
├──────────────────────────────────────────────────────────┤
│ Ingress：Android NLService │ 电脑 ANCS/BLE（Phase 4）      │
├──────────────────────────────────────────────────────────┤
│ tetherly-core：OTP、帧、状态机、去重、白名单（无 OS API）    │
└──────────────────────────────────────────────────────────┘
```

**iPhone 验证码**

```
短信通知 --BLE ANCS--> 电脑 Ingress --> NotifyHub
  --> 本机 extract(body) --> 弹窗
  --> 可选转发 notify.push 给其他电脑（对方再 extract）
用户点填入 --> Insertor
```

**Android 验证码**

```
NLService --> notify.push --> 电脑 NotifyHub --> extract
```

**跨网**

```
Android --EasyTier 虚网 TCP 45717--> 家里电脑
发现：easytier-cli peer / RPC 列出虚网 IPv4，逐个拨端口，Hello 认人
```

### 5.1 发现

**真实 LAN（Phase 1）**

- 服务：`_tetherly._tcp`
- crate：`mdns-sd`
- TXT（短，避免超 UDP 限制）：

```
v=1
id=<device_id>
fp=<hex(sha256(id_pk)[0..6])>   // 12 hex 字符，仅展示
caps=notify,clip,file
```

端口在 SRV。完整 `id_pk` 只在 Hello。

**虚网（Phase 2，已接线）**

- 默认 CIDR `10.144.144.0/24`，可用 `--overlay-cidr` 覆盖。接口名 `et*` / `easytier` / `tun*`（不含 `eth*`）只作启发式分类；**bind / 广告仍要求地址落在 overlay CIDR**，避免把无关 TUN 当虚网。
- `GET` EasyTier RPC（本机 `15888`，超时 `OVERLAY_PROBE_MS=2000`）或解析 `easytier-cli -o json peer list`（失败则 `peer`）。JSON 任意形状 walker + 文本 dotted-quad 扫描。Windows CLI 用 `CREATE_NO_WINDOW`。
- 对每个 overlay peer IPv4 **TCP 连接 45717**，超时 2s。`extra_peers` 是拨号目标，**不必**落在 CIDR（测试 / 手动 IP）。
- 虚网 bind 与 LAN bind 分开；**禁止**在虚网接口上发 mDNS 作为发现手段。
- 同设备已有 LAN 会话时丢弃 overlay attach；overlay 会话结束不拆 LAN。
- 未装或停 EasyTier：`lan_only`，UI「仅局域网」，不崩。

IPv6：v1 不做。

---

## 6. 身份、配对、会话

### 6.1 存储

| 材料 | 位置 |
|---|---|
| `id_sk`、`n_sk` | **凭据库已接(v1.13 Windows / v1.14 macOS+Linux)**：Windows Credential Manager 泛型凭据、macOS Keychain 泛型密码、Linux/BSD Secret Service（libsecret）；回退 `%LOCALAPPDATA%\tetherly\identity.bin` 等，ACL 仅当前用户 |
| 信任库 | `trusted.json`：`device_id, id_pk, n_pk, alias, paired_at, revoked` |
| 配置 | `config.toml` |

写入：临时文件 + `rename`。忘记设备：标 `revoked` 或删除行。**凭据库（v1.13/v1.14）**：Windows→Credential Manager 泛型凭据（`tetherly.identity:<device_id>`）、macOS→Keychain 泛型密码、Linux/BSD→Secret Service（属性 `device_id=<id>`）；数据目录只留公开的 `identity.ref` 指针，旧 `identity.bin` 首次运行自动迁移入库并删除。**仅「已安装数据目录」启用凭据库**——临时目录节点（全部集成测试）仍用本地 `identity.bin`，绝不写操作者真实凭据库（`TETHERLY_VAULT=1/0` 可覆盖）。无可用凭据库（headless Linux / 无 keyring / 非三平台）时透明回退文件。

### 6.2 状态机

```
Idle → Discovering → TcpConnected → Hello
Hello + 未知 id_pk → Pairing(SPAKE2) → PairBind → NoiseIK → 写信任库 → Active
Hello + 信任库命中且 n_pk 一致 → NoiseIK → Active
Hello + 信任库命中但 n_pk 变了 → 断开（需用户重新配对）
失败 → Backoff 1s,2s,4s,…,60s
Active 断线 → Backoff
忘记设备 → 删信任 → Idle
```

Hello 之后、Noise（或 SPAKE2）完成前 **禁止业务帧**。`device_id` 与 `id_pk` 对不上立即断开。

Hello JSON，最大 4 KiB，明文长度前缀：

```jsonc
{
  "proto": 1,
  "device_id": "tdev_...",
  "id_pk": "<base64 32B>",
  "n_pk": "<base64 32B>",   // 未配对也发，供 salt 排序；Noise IK 仍以信任库为准
  "caps": ["notify", "clip", "file"],
  "name": "Mango-PC",
  "platform": "windows"
}
```

`n_pk` 在未配对 Hello 里可见：这是 TOFU 指纹材料，真正绑定靠 SPAKE2 + 签名。

### 6.3 配对 UX

1. A「添加设备」显示 `2517 0394`（空格分组），3 分钟倒计时。
2. B 输入 8 位数字。两端可再显示 `fp` 供人工核对（非必须）。
3. 成功：写入信任库，PIN 与 `pwd` zeroize。
4. 文案：「不要把数字发给任何人，只在你自己的另一台设备上输入。」

测试向量：固定 PIN `25170394`、固定 `id_sk`/`n_sk` 种子、固定 transcript 哈希，放进 `tetherly-crypto`。

### 6.4 Noise

- crate `snow`，模式见 ADR-003。
- IK：发起者必须已有响应者 `n_sk` 对应公钥（来自信任库，不来自本次 Hello 的 `n_pk`，除非指纹一致）。
- 应用层 `msg_id`：内层帧 `u16 type | u16 flags | u64 msg_id | payload`；每会话单调；收端 LRU 防重放。
- 每 2^16 条 Noise 消息或 3600s 调用 `rekey`（若当前 `snow` 版本 TransportState 无 rekey，改为重建 IK 会话，Phase 0 写进注释）。
- Hello / SPAKE2 / PairBind / Noise 握手读超时 20s。新 TCP 每源 IP 30/min。

### 6.5 能力

Active 后可 `caps.update`。未知 cap 忽略。未声明的 type 丢弃。

---

## 7. 线协议

### 7.1 连接上的字节

```
TCP
  ├─ u32be hello_len | hello_json          // 双方各发一次
  ├─ 若未配对：SPAKE2 消息（u32be len | bytes）若干轮
  │            u32be len | PairBind_ciphertext
  └─ Noise handshake 消息（snow 默认带长度由我们再加 u32be）
     └─ 之后：u32be noise_msg_len | noise_transport_payload
              解密后内层：u16be type | u16be flags | u64be msg_id | payload
```

内层 payload 上限 `INNER_PAYLOAD_MAX`（v1.22 起由传输上限推导：Noise 单条消息 65535 − 16 字节 AEAD tag − 12 字节内层帧头 = **65507**；详见 §9.7 与「1.22 相对 1.21」）。文件块走第二条 TCP：Hello 可省略，用 `file_token` 做 Noise PSK 或把 token 放在 IK payload。`file_token = HKDF-SHA256(ck, "tetherly-file-v1" || transfer_id)`，60s 过期。文件帧上限 256 KiB。

`flags` bit0 = `MUST_UNDERSTAND`：未知 type 则断开。

### 7.2 内层 type（v1）

| 符号 | 值（建议） | 载荷 |
|---|---|---|
| ping / pong | 0x0001 / 0x0002 | `unix_ms: u64` |
| notify.push | 0x0101 | JSON：uid, app_id, app_name, title, body, ts, actions[]（无 url） |
| notify.dismiss | 0x0102 | uid, app_id |
| notify.reply | 0x0103 | JSON:uid, app_id, text(快速回复,桌面→手机;≤4 KiB,仅投递、不执行) |
| clip.set | 0x0201 | mime + text 或 blob_ref |
| file.offer | 0x0301 | transfer_id, files[{name,size,sha256}] |
| file.accept / reject | 0x0302 / 0x0303 | transfer_id |
| file.done | 0x0304 | transfer_id, sha256 |
| caps.update | 0x00F0 | caps[] |

没有 `otp.candidate`。没有 `input.*` JSON。

### 7.3 去重与限流

通知键：`(source_device_id, app_id, uid)`，LRU 10 分钟。uid 不保证单调。

| 对象 | 限制 |
|---|---|
| notify.push | 每源 2/s，突发 10，超出丢弃 |
| clip.set | 4/s |
| 新 TCP | 每 IP 30/min |
| title/body | 各 4 KiB 截断 |
| 剪贴板文本 | 1 MiB，更大走文件 |

### 7.4 文件

默认必须点确认。自动接收默认关。文件名只取 basename，拒绝 `..`。v1 无跨进程续传；同连接可重发块 → **v1.12 起**:`transfer_id` 内容寻址(`"tr_"+sha256(file)[..16]`);接收方半成品写 `inbox/.<transfer_id>.part`、块对齐偏移持久化到 `inbox/.<transfer_id>.resume.json` 侧车,Node 重启扫描侧车重建续传表;发送方按 offer 回带的 `resume_from` 仅发尾块。续传**须 64KiB 块对齐**——每块 AAD=`transfer_id||chan_offset_le`、nonce=偏移/块+1,块不可跨位置搬移;侧车与 `.part` 撕裂则丢弃并从 0 重传。断线 offer 作废。控制面 offer/accept 走 45717；数据走对端 `caps.update` 广告的 **45718**（`fileport=`）。`file_token = HKDF-SHA256(handshake_hash, "tetherly-file-v1" || transfer_id)`；块 ChaCha20-Poly1305，魔数 `TFL1`（v1）/`TFL2`（v1.12 带起始偏移），块 64KiB。

### 7.5 剪贴板

桌面：`clip_seq` + last-hash 防回环。Android：按钮「发送剪贴板」。iOS：仅前台。写入 OTP 后默认 60s 清本机剪贴板。

---

## 8. Workspace

Phase 0 四件套仍在。Phase 1 **已加**：

```
tetherly/
├── Cargo.toml
├── crates/
│   ├── tetherly-core/      # OTP、帧、状态机、白名单、去重、bind/clip/file/notify/persist
│   ├── tetherly-crypto/    # Ed25519、X25519、Argon2id、SPAKE2、Noise、file AEAD
│   ├── tetherly-net/       # TCP、hello、mDNS、内层 dispatch、filechan 45718
│   ├── tetherly-node/      # LAN 节点、落盘、hubs、本机 HTTP UI、EasyTier sidecar 发现、tetherly 二进制
│   └── tetherly-android/   # 手机原生库：真身份 + Noise 会话 → libtetherly_android.so（cdylib+rlib）+ android_probe
├── bins/tetherly-cli/      # Phase 0 配对夹具仍保留
├── tests/loopback.rs       # Phase 0
├── tests/phase1.rs         # Phase 1 loopback 双节点
├── tests/phase2.rs         # Phase 2 sidecar unicast / LAN 优先 / 缺侧车
├── tests/phase3.rs         # Phase 3 键鼠 45719
├── tests/phase4.rs         # Phase 4 ANCS 编解码与状态机（含 m4_5 入站泵端到端）
├── ui/index.html           # 本机 UI（loopback HTTP）
├── android/                # NLService + gradle CI
└── docs/
```

其后按阶段加：`tetherly-ffi`、`src-tauri`、`ios/`。`android/` 已存在且 JNI `.so` 已由 `tetherly-android`（v1.10）构建。ANCS 与键鼠引擎都放在 `tetherly-core`（`ancs` / `input`），不开新 crate。

红线：`tetherly-core` 无 `cfg(target_os)`、无 `windows`/`objc`/`jni`。`tetherly-net` 无 `easytier` crate。`tetherly-android` 的 `jni` 依赖锁在 `cfg(target_os = "android")`，桌面构建不引入；JNI `unsafe` 仅豁免 `jni.rs` 一处。

### 8.1 Phase 0 依赖冻结（名称，版本在 lock 时钉）

| crate | 用途 |
|---|---|
| `snow` | Noise |
| `spake2` | PAKE |
| `argon2` | PIN KDF |
| `ed25519-dalek` | 身份签名 |
| `x25519-dalek` | Noise 静态/临时 |
| `sha2` / `hkdf` / `zeroize` | 派生与擦除 |
| `mdns-sd` | LAN 发现 |
| `tokio` `serde` `thiserror` `tracing` | 运行时 |

升级大版本视为协议风险，要重新跑测试向量。

### 8.2 核心类型

```rust
pub struct DeviceId(/* tdev_ + base32 */);

pub struct PhoneNotification {
    pub source: DeviceId,
    pub app_id: String,
    pub app_name: String,
    pub uid: String,
    pub title: String,
    pub body: String,
    pub received_at: SystemTime,
    pub actions: Vec<ActionId>, // Copy | Dismiss | OpenApp，无 URL
}

pub const DEFAULT_MIN_SCORE: i32 = 5;

pub trait OtpExtractor: Send + Sync {
    fn extract(&self, text: &str) -> Option<String>; // 内部仍有 score，对外过阈值才 Some
}
```

OTP 语义冻结为 sms-pop-rs：4–8 位连续数字；关键词前 48 / 后 24 字节；11 位电话整段跳过；1900–2099 四位年剔除；宁缺毋错。先移植测试，不准「改进」规则。

平台 trait（core 定义，平台实现）：`Clock` `Rng` `TrustStore` `Insertor` `Clip` `Notifier`。

---

## 9. 功能模块

### 9.1 NotifyHub

入口：ANCS、Android、loopback。截断、去重、限流、本地 extract、UI、按 caps 转发 `notify.push`。默认只转给桌面，不回推手机。

### 9.2 填入

`insertion::plan(current, code)`：空则整写；前缀则补差；已完整则不写；只读则拒绝。Simulate 必须拿着探测到的 element 引用，禁止对「当前前台」盲打。

### 9.3 打开白名单

```toml
[[open_rules]]
app_id = "com.tencent.xin"
url = "weixin://"
```

### 9.4 ANCS（Phase 4，电脑侧）

与 sms-pop-rs 相同的硬约束：先订 Data Source 再订 Notification Source；禁止在 ValueChanged 线程写 Control Point；Control Point 串行；分片按字节拼满；首次属性延迟重试；断线退避；uid 去重。

Windows：`windows` crate GATT。macOS：CoreBluetooth。Linux：BlueZ，不要把电脑广告成音箱。

用户路径：系统蓝牙设置里让 iPhone 连电脑（ANCS），**再**在 Tetherly 里把这台电脑与 Android/其他电脑 SPAKE2 配对。两步都要在 UI 向导里拆开写。

**v1.23 已落地 Windows 真机传输栈**：`tetherly-node::ancs::win_gatt`（自包含、`#[cfg(windows)]`，照 `screen/macos.rs` 先例）——专属 `tetherly-ancs` 线程 + **MTA 公寓**持有全部 WinRT 接口；`FromBluetoothAddressAsync` → `GetGattServicesForUuidAsync(ANCS)` → `GetCharacteristicsAsync` → 按 UUID 找三特征并**校验属性**（Notification Source / Data Source 必须 `Notify`、Control Point 必须 `Write`，缺一点名报错）；CCCD 写 `Notify` 订阅；`ValueChanged` 回调**只把字节拷进通道**（§9.4 禁止在回调线程写 Control Point）；`write_control_point` 经 `DataWriter` → `WriteValueWithResultAsync` 并核对 `GattWriteResult::Status`，**阻塞到结果为真**（「串行且只有一条在途」成为事实）；前端每次命令的 ack 等待设 **10s 上界**，超时即报 `AncsSubscribe`/`AncsWrite`（不谎报成功）；`ConnectionStatusChanged` 置标志 → 退出会话 → `ancs_backoff_delay` 指数退避（1s→30s）；`Session::drop` 显式 `RemoveValueChanged`/`RemoveConnectionStatusChanged`。**同步的三个端口方法直接用 `windows` 0.59 的阻塞 `IAsyncOperation::get()`**（OS `Waiter` 信号，不空转），故**无新依赖、无自造 executor**。node 侧 `Node::spawn_ancs_pump`（独占线程 + current-thread 运行时）把 `AncsInbound`（`Ready`/`NotificationSource`/`DataSource`/`Disconnected`）喂给 `ancs_connect`/`ancs_notification_source`/`ancs_data_source`/`ancs_disconnected`，并每 500ms `ancs_tick`；CLI `tetherly run --ancs <11:22:33:44:55:66>`。macOS（CoreBluetooth）与 Linux（BlueZ）栈**未实现**，`open()` 诚实返回 `Ok(None)`。物理 iPhone 的运行时 **Manual-required**。

**v1.6 已落地（CI 切片）**：`tetherly-core::ancs` 协议编解码 + `AncsIngress` 状态机 + `AncsTransport` 端口 + `MemoryAncsTransport`。硬约束按构造强制：订阅顺序由 `connect()` 固定；`on_notification_source` 只入队，Control Point 只在 `tick` 写；`in_flight` 保证串行；`DataAssembler` 只在整条属性元组到齐后才产出；静默一次后重试、再静默则放弃；`on_disconnected` 走 500ms→30s 退避。真实 GATT 栈与物理 iPhone 仍 Manual-required。

### 9.5 键鼠（Phase 3）

拓扑在 `config.toml`。断线光标回 server。Wayland 走 portal+libei，不行就标不支持。DeskFlow 网关另开端口、默认关、TLS 未实现就不要宣称兼容。

**TIN1 语义（v1.9 明确）**：`Move` 的 x/y 是 `SCREEN_UNITS=1000` 千分比归一化（避免浮点过线），受控端映射回本地屏幕；`Button` 的 mask 位为 `BTN_LEFT=1 / BTN_RIGHT=2 / BTN_MIDDLE=4`；`Wheel` 的 dx/dy 为 i16 delta；`Key` 的 code 为 **Win32 虚拟键码（VK）**（u16），`mods` 为信息性字段——修饰键以独立 key 事件传输。

**v1.5 已落地**：`tetherly-core::input` 编解码与边缘状态机；`tetherly-node` 在 LAN 与 overlay 上另听 45719，会话 `resume_only`。CI 注入是 `MemorySink`，不调用 Win32 `SendInput`。进入帧落在 `x=0` 时保持焦点；离开只在已经进入内部后再回到左缘时发生。剪贴板仍走 45717，不进键鼠帧。

**v1.9 真机链已落地（Windows）**：受控端 `SystemInputSink`（`InputSink` 端口实现；SendInput 绝对移动/按钮/滚轮/VK 键盘，`NodeConfig.input_sink` 注入，CI 缺省 `None` 仅观察者）——收到的事件在协议级过滤（seq 窗口/座位门控）后经 `MemorySink` 观察者 trace 镜像到真实注入。主控端 `Node::spawn_input_capture`：`WH_MOUSE_LL` 恒挂（只读光标做过边检测）、`WH_KEYBOARD_LL` **仅在远程座位时挂**（本地键盘隐私）、注入事件按 `LLMHF/LLKHF_INJECTED` 过滤防单机回环；事件喂与 CI 相同的 `drive_input*` 路径。**注入探针实测**：过边后受控端光标真实移至左缘 (0,539)、中心移动精确、Win/Esc 键盘注入生效。双桌面 100 次物理往返仍 Manual-required。

### 9.6 Android（Phase 1）

`NotificationListenerService` + 前台服务。Android 13+ `POST_NOTIFICATIONS`。帮助页教各 ROM 自启动，不搞黑保活。不读短信库。Play 上架需通知使用权声明，内部测试可先 sideload。

**原生库（v1.10 已落地）**：`crates/tetherly-android` 产出 `libtetherly_android.so`（arm64-v8a，cargo-ndk）。身份为**真 Ed25519 + X25519**（`Identity::from_secrets`，`device_id = DeviceId::from_id_pk`），`identity.bin`/`trusted.json` 与桌面节点同构、可互换。会话走与桌面**相同**的 `dial_session`（首配 Hello + SPAKE2 8 位 PIN，之后 Noise IK resume），手机仅拨 45717、不做 listener。**手机能力集固定 `["notify","clip","file"]`，永不广告 `input`/`screen`**。Kotlin 侧 `NativeBridge`/`NodeSession` 经 JNI 薄壳（`Java_app_tetherly_NativeBridge_*`）对接，拿到真实 `device_id` 并驱动 `notify.push`。**v1.3 的 `idPk = sha256(idSk)` 占位已删除**；无 `.so` 时 `state()==0` 且 `identity=="unavailable"`，**禁止**自己拼 SPAKE2/Noise TCP。

**已验（真机 v1.10）**：两台 Galaxy Fold（SM-F900F / arm64-v8a / Android 13）分别以 `android_probe` 完成真实配对 + notify 投递（桌面 `has_otp:true`）+ 换错 PIN 仍 resume 成功。CI 跑 `cargo ndk` 出 `.so` 后再 `gradle :app:testDebugUnitTest` + `assembleDebug`（`jniLibs/` 是产物，不入库）。协议与 OTP 路径另由桌面双节点 loopback 覆盖(物理机 M1.1 p95 由 v1.18 真机实测关闭,见下段;8h soak 仍 Manual-required)。

**APK UI 全链路已验(v1.18 真机)**:真机 `app.tetherly`(已装 APK,前台 `NotificationListenerService`)→ JNI → `libtetherly_android.so` 实时会话闭合;真机侧推送测试通知经桌面节点提取为候选 `{app_id:"com.android.shell", has_otp:true, source:"tdev_…"}`(正文/验证码不入快照/日志)。**M1.1 p95 上机实测**:手机→桌面通知往返 `rtt_p50≈17.9ms / p95≈56.9ms / p99≈361.8ms`(云手机经 `adb reverse` 隧道,非 LAN;LAN 同机型约 15–16ms),p95 在 <1.0s 预算内。**Manual-required(收窄)**:8h soak 与 LAN 直连双物理桌面往返;`.so` 缺失、APK 链路、物理 Android M1.1 p95 计量三项阻塞已消除。

**快速回复(v1.11)**:桌面可对手机通知就地回复——notify.reply(0x0103,JSON uid/app_id/text)。NotifyListener.drainReplies() 把收到的回复交给通知自身的 RemoteInput action(actionIntent.send)执行,不合成应用未声明的回复;activeNotifications 无匹配则丢弃。文本为用户内容,不入日志。协议与收件箱由 CI 覆盖(phase2::m2_6 + client_e2e::android_client_receives_a_quick_reply_once);真机触发 RemoteInput(需系统权限/前台通知)Manual-required。

### 9.7 远程屏幕（Phase 5）

**协议**：`tetherly-core::screen`（`crates/tetherly-core/src/screen.rs`）。帧魔数 `TMV1`（`SCREEN_MAGIC`），proto 1，seq u64be 单调递增窗口 `SCREEN_SEQ_WINDOW=65536`，trace 上限 64。单帧 header 31 字节：`TMV1`(4) | proto(1) | seq(8) | x(4) | y(4) | w(2) | h(2) | fmt(1) | compress(1) | stride(4) | pixels。控制消息 14 字节：`TMV1`(4) | proto(1) | tag(1) | seq(8)；tag 取值 Start=0 / Stop=1 / Ack=2。像素格式 `PixelFormat{Bgra8,Rgba8}`，默认 BGRA（一致 Win32）。传输走独立 TCP 端口 **45720**（`SCREEN_PORT`），`resume_only`：只在已配对对端间建会话；`screen_only: true` 的会话在 `HelloDisposition::Pair` 时返回 `NetError::ScreenRequiresTrust`。Hello cap 广告 `"screen"` 与 `screenport=N`。**分片预算(v1.22 修正)**:超内层预算的整帧拆成 `TMF1` 分片(header 24 字节),每片 payload `SCREEN_FRAG_HEADER + SCREEN_FRAG_CHUNK_MAX` 必须 `<= INNER_PAYLOAD_MAX`;`INNER_PAYLOAD_MAX` 由传输上限推导(= `65535 − 16 − 12 = 65507`),故分片 payload 恰为 65507、编码后恰为可加密上限 65519。此前 `INNER_PAYLOAD_MAX` 写死 64 KiB 时,分片 payload 65536、编码后 65548 明文**无法加密**,是「分片屏幕帧发不出去」的潜伏缺陷根因(见「1.22 相对 1.21」)。

**门控模型**：`ScreenState` 四态状态机：`Idle → Requested → Allowed → Streaming`。对端 dial 45720 成功仅置 `Requested`，不开始推流。必须本机显式调用 `Node::screen_allow()` 才进 `Allowed`。对端发 `ControlMsg::Start` 进入 `Streaming` 后才出帧；在 `Allowed` 之前收到的 `Start` 被拒绝并计数（`refused`）。`Node::screen_revoke()` 回到 `Idle`。对端断开（`on_peer_gone`）强制回 `Idle` 并清零 seq。重连不重放旧帧——接收方 seq 窗口丢弃 seq < 已收最大 seq 的帧。

**采集**：`ScreenSource` 端口在 `tetherly-core` 声明，`tetherly-node` 注入实现。CI 用 `MemoryScreenSource`（确定性帧序列）。Windows 真机（v1.8 已落地）：`DxgiSource`（`windows` crate DXGI Desktop Duplication）——`D3D11CreateDevice(BGRA)` → `IDXGIDevice::GetAdapter` → `IDXGIOutput1::DuplicateOutput`，`AcquireNextFrame(0)` 遇到 `DXGI_ERROR_WAIT_TIMEOUT` 返回 `Ok(None)`（无新帧），staging 纹理 + `Map(READ)` + 行尾裁剪产出紧凑 BGRA8 帧；`SystemSource` 按目标选择采集器（Windows=`DxgiSource`；macOS=`MacSource`；X11=`X11Source`）。另有 `AdbSource`（v1.8）：`adb exec-out screencap -p` 抓取 adb 连接的 Android 设备（云手机场景），PNG 解码 → 紧凑 BGRA8；PNG 字节与上帧相同则 `Ok(None)`（DXGI 同款空闲语义）；`tetherly run --screen-adb <serial>` 选用，`TETHERLY_ADB` 指定 adb 路径。**v1.19 落地**:macOS `MacSource`(`core-graphics` `CGDisplayCreateImage` → 显式 BGRA8 `CGBitmapContext`,读回框架实际 stride 去行尾填充)与 Linux/BSD X11 `X11Source`(`x11rb`;MIT-SHM 快路径 + plain `GetImage` 回退,含 `shm_query_version >= 1.2` 门与按 `reply.size` 精确读取)。两者按 `cred.rs` 先例各为**自包含文件**(只 import 绝对路径),`Cargo.toml` 加目标依赖 macOS `objc2-core-foundation`/`objc2-core-graphics`(`default-features = false`)、Linux `x11rb`(shm)/`libc`,**不引入新包**(均随 `arboard` 已在 lockfile)。真机采集需真实显示器(macOS 需屏幕录制权限,X11 需 X server),**Manual-required**;Wayland 仍标「不支持」(纯 Wayland 无 X11 根窗口可读,portal 路线属下一迭代)。

**压缩**：生产默认链为 `EncodedSource<SystemSource>`——`ZstdCodec`（无损，level 3）实现 `ScreenCodec` 端口，每帧压缩，压不小就发原帧（病态编码器不可能放大传输）；接入侧按 `raw_len` 校验 decompress 后长度。未来 OpenH264（BSD-3-Clause）/ rav1e（BSD-2-Clause/ISC）作为同一 `ScreenCodec` 端口替换——「**先无损帧 + 后接编码器**」的落点。不走 GPL/AGPL（x264/x265 明确出局）。

**呈现**：`ScreenSink` 端口在 `tetherly-core` 声明。CI 用 `MemoryScreenSink`（断言 seq 单调）。Windows 真机（v1.8 已落地）：`SystemSink` → GDI 窗口——`RegisterClassW`/`CreateWindowExW` + `CreateDIBSection`（32bpp top-down）+ 同步 `BitBlt`（此线程无消息泵，不能依赖 `WM_PAINT`，直接画到窗口 DC 立即显示）。**v1.19 落地 X11 呈现**:`SystemSink` → `x11rb` override-redirect 窗口 + `PutImage`(窗口深度经 `COPY_DEPTH_FROM_PARENT` 继承自根,`PutImage` 用**读回的根深度**而非硬编码 24,否则 32 位根上回 `BadMatch`)。macOS 呈现需 AppKit `NSWindow`(CoreGraphics 位图上下文无法触及屏幕),`present` 返回点名 `NSWindow` 的诚实 `ScreenRefused`;Wayland portal 与其余非 Windows/macOS/X11 目标同样诚实 `ScreenRefused`。

**RustDesk 借鉴范围**：架构借鉴仅限分离采集+编码器 swap+P2P 模式——这些都是公开工程常识，不构成知识产权。RustDesk 客户端与服务端均为 AGPL-3.0，`deny.toml` 已 ban `hbb_common` / `rustdesk` / `rustdesk-server` / `scrap`。不借鉴 RustDesk 的居会合（hbbs/hbbr）服务器——匹配 §2.3 禁云账号。

**v1.7 落地范围**：`tetherly-core::screen` 帧+控制编解码、`ScreenSender`/`ScreenReceiver`、强制本机同意 M5.2、45720 独立端口 + `ScreenRequiresTrust` 门、`tetherly-node` 14 处锚点（`bound_screen` / `screen_out` / `screen_engine` / `screen_sink` / `screen_ctl` / `spawn_listeners` / `spawn_overlay` / caps / `LivePeer.screen_port` / `open_screen` / `accept_screen` / `attach_screen_server` / `attach_screen_client` / `screen_send_frame`）、UI 路由 `/api/screen/allow|start|stop` + 前端「远程屏幕（实验）」区、`tests/phase5.rs` 4 个绿测试。真机采集 / zstd 编码 / BitBlt 呈现 / p95 计量其后由 **v1.8 真机闭环**落地(见 §12)。**v1.19 补齐 macOS/Linux 采集**（见本节「采集」与「呈现」段）：开放编码器（OpenH264/rav1e）与 macOS 呈现（NSWindow）仍 Manual-required。M5.5 硬件编码 / 真机注入「未做」。

---

## 10. 模式与反模式

用：Transport 策略、显式 `ConnState`、ANCS 串行队列、插件注册表、TOFU、指数退避。

禁止：core 里 `target_os`；自动填码；日志里的码/正文/剪贴板；执行对端 URL；JSON 鼠标坐标；BLE RPC；iOS VPN entitlement；虚网 mDNS 当唯一发现；把 EasyTier 链进二进制。

---

## 11. 安全工程

- CI：`cargo deny`（license + bans）+ `cargo audit`
- lockfile 入库
- Tauri updater 公钥内置；更新不上传通知
- SECURITY.md：禁止公开 issue 贴验证码
- `TETHERLY_INSECURE_LOG=1` 仅 debug profile，启动警告

---

## 12. 阶段与里程碑

未勾选退出标准不得开下一阶段功能代码（文档/UI 草图可以并行）。工期是上限。

### Phase 0 — 协议骨架（约 2 周）

四个 crate + CLI。无 GUI、无 BLE、无 EasyTier。

| ID | 验收 | 测法 |
|---|---|---|
| M0.1 | OTP 移植用例全绿 | `cargo test -p tetherly-core` |
| M0.2 | 双进程 SPAKE2 成功；错 PIN 5 次锁定 | CLI |
| M0.3 | 配对后 ping；杀一端退避重连 | CLI |
| M0.4 | 夹具改 Hello `id_pk`：SPAKE2 失败；线路上改 Hello `name`：Noise 失败。均不写信任库 | 测试 |
| M0.5 | `device_id` 与 `id_pk` 不一致则断开 | 单测 |
| M0.6 | 固定种子测试向量（Argon2+SPAKE2+Noise prologue） | `tetherly-crypto` |
| M0.7 | CI：fmt、clippy `-D warnings`、test、deny、audit | Actions |

退出：M0.1–M0.7。许可证调查已完成（本 ADR-006），不再作为退出条件。

### Phase 1 — LAN 桌面 + Android（约 4 周）

| ID | 验收 | 状态（v1.3） |
|---|---|---|
| M1.1 | Android 测试通知含 `524681` 到桌面弹窗，**p95 < 1.0s**（listener 回调 → 窗口可见） | loopback `notify.push` CI 绿、耗时 < 1s。**真机链已落地（v1.10）**：`libtetherly_android.so`（真 Ed25519 + Noise IK）+ `android_probe` 在两台 Galaxy Fold 完成配对 + notify 投递（桌面 `has_otp:true`）。物理 Android p95 计量 / 8h soak：**Manual-required**。macOS M1.1：**mac 下一迭代** |
| M1.1(v1.18 更新) | 同上 | **APK UI 全链路真机已验**(云手机 `app.tetherly` 前台 NLService → JNI → 实时 Noise 会话 → 桌面候选 `has_otp:true`,正文不入日志)。**p95 上机实测**:往返 `p50≈17.9ms / p95≈56.9ms / p99≈361.8ms`(经 `adb reverse` 云手机隧道;LAN 同机型约 15–16ms),p95 在 <1.0s 预算内。8h soak + LAN 直连双桌面往返仍 Manual-required |
| M1.2 | 复制后剪贴板为该码；120s 候选消失 | CI 绿（`tests/phase1.rs` + ManualClock） |
| M1.3 | 记事本可填；只读拒绝 | CI 绿（`MemoryInsertor` 空写/只读拒绝）。真记事本 UIA：**Manual-required** |
| M1.4 | Win↔mac 文本剪贴板（有双桌面时）；至少 Win 本机回环 | Win 双节点 loopback CI 绿。Win↔mac：**Manual-required** |
| M1.5 | 杀 Android 进程后自动重连，不重新配对 | persist trust + 无 PIN 重拨 CI 绿。**真机已验（v1.10）**：`android_probe` 二次运行以**错误 PIN** 仍连上（走 Noise IK resume 而非配对），身份持久化于 `identity.bin`。真杀 Android 进程：**Manual-required** |
| M1.6 | 运行日志 `grep 524681` 无命中 | CI 绿（tracing MakeWriter 捕获） |
| M1.7 | 未确认不收文件：确认后 10MB sha256 一致 | CI 绿（reject 无 inbox 文件；accept 10MiB sha256）。**v1.24**：声明超过 `FILE_MAX` 的报价在**用户看到之前**即被拒（`FileTooLarge`，候选不出现）且**不拆会话**，随后一笔正常传输仍成功 |
| M1.8 | 中断后**跨进程**续传：重启接收方，凭持久偏移只补发尾块，最终 sha256 一致 | **CI 绿（v1.12）**：`tests/phase1.rs::m1_8_file_resumes_from_persisted_offset`（预置 `.part`+侧车后重启 B → offer 回带 `resume_from==2*64KiB` → 接受后 sha256 一致 → 侧车清除）；无侧车则从 0 重传 |

退出：M1.1–M1.3、M1.5–M1.8。Win+Android 8h 无崩溃。macOS 能跑通 M1.1 或文档标明「mac 下一迭代」。

**v1.3 退出裁定（历史）**：CI 切片（协议、落盘、日志脱敏、文件确认、本机 UI、Android 工程骨架）已绿。物理 Android p95 / 8h / JNI `.so` / 真 UIA / Win↔mac 仍是 Manual-required。后续以用户「继续按文档开发」为豁免，进入 Phase 2 CI 切片。

### Phase 2 — EasyTier 侧车（约 3 周）

| ID | 验收 | 状态（v1.4） |
|---|---|---|
| M2.1 | Android 蜂窝网 ↔ 电脑，通知 p95 < 3s | **Manual-required**（真机蜂窝 + 用户 EasyTier soak） |
| M2.2 | 同 Wi-Fi 走 LAN 接口；断 Wi-Fi 后 **新通知** 走虚网。进行中文件允许失败并提示 | CI：已有 LAN 时丢弃 overlay attach，重复扫描仍保一条 LAN。真机断 Wi-Fi 切虚网：**Manual-required** |
| M2.3 | 停 EasyTier：UI「仅局域网」，不崩 | CI 绿（无 RPC/CLI/TUN → `lan_only`，shutdown 不崩） |
| M2.4 | 未装 EasyTier 时 Phase 1 不变 | CI 绿（隔离 CIDR 下仍可配对 + `notify.push`） |
| M2.5 | 虚网接口上关闭 mDNS 仍能靠 peer 列表连上 | CI 绿（`advertise=false` + `extra_peers` 单播 45717） |

**v1.4 退出裁定**：Phase 2 CI 切片（侧车探测、单播、LAN 优先、缺侧车仅局域网、UI 文案）已绿。M2.1 物理蜂窝 p95 与真机断 Wi-Fi 切路径仍是 Manual-required，**不得开 Phase 3 功能代码**，除非另下豁免。

### Phase 3 — 桌面键鼠（约 4 周）

| ID | 验收 | 状态（v1.5） |
|---|---|---|
| M3.1 | 光标过 Win 右缘到第二台桌面，键盘跟随；脚本往返 100 次丢失 0 | CI：45719 过边 + 按键进 `MemorySink`；core 100 次往返 0 丢失。**真机链已落地（v1.9）**：受控端 `SystemInputSink`（SendInput：1000 单位→65535 绝对坐标、按钮/滚轮/VK 键盘）+ 主控端 `spawn_input_capture`（WH_MOUSE_LL 恒挂过边检测、WH_KEYBOARD_LL 仅远程座位时挂、注入事件按 LLMHF/LLKHF_INJECTED 过滤防回环）；注入探针实测光标过边到左缘 (0,539) 与中心移动精确。**双桌面 100 次物理往返：仍 Manual-required** |
| M3.2 | 对端断开，光标回本机 | CI 绿（输入会话结束 `on_peer_gone`，座位回 Local） |
| M3.3 | 键鼠模式下剪贴板仍可用 | CI 绿（焦点在对端时 `clip.set` 仍走 45717） |
| M3.4 | 加分：DeskFlow 官方 client + TLS 能移动光标。失败则网关标 experimental | **未做**。网关保持关闭，不链 GPL 源码，不挡退出 |

Wayland 不挡退出。真机注入未落地前不得宣称可替代键鼠。

**v1.5 退出裁定**：Phase 3 CI 切片（resume-only Noise、二进制帧、过边、断线、剪贴板并存）已绿。物理双桌面与 OS 注入仍是 Manual-required。

**豁免记录（2026-10-10）**：上述「不得开 Phase 4」的闸门已由项目方豁免，Phase 4 ANCS CI 切片据此落地（见 v1.6）。物理双桌面与 OS 注入的 Manual-required 状态**不变**。

### Phase 4 — iPhone ANCS（约 4 周）

| ID | 验收 | 状态 |
|---|---|---|
| M4.1 | 系统蓝牙连接后短信通知 p95 < 2s 到 Win 或 mac | **CI 代理**：`tests/phase4.rs` 用注入时钟断言端到端预算 < 2s，且 app 名等待封顶 400ms。真机 p95 = Manual-required |
| M4.2 | 仅白名单显示「打开」；payload 带 url 也不打开 | **CI 绿**：`ancs_actions` 门控；`open_candidate` 只走白名单 url；载荷 url 从不读取 |
| M4.3 | 关蓝牙 30s 再开：重连且不连弹旧通知 | **CI 绿**：`PreExisting` 丢弃 + uid 去重；退避拒绝热重连 |
| M4.4 | 可选 iOS App：同 LAN 50MB sha256；无此 App 不挡退出 | 未做（不挡退出） |
| M4.5 | iOS App 无 VPN / Network Extension | 未做（随 M4.4） |

**v1.6 落地范围**：`tetherly-core::ancs`（Notification Source 解析、Control Point 编码、Data Source 按字节重组、`AncsIngress`）、`tetherly-node` 的 `ancs_*` 入口与 `Opener` 端口、UI 两步向导、`tests/phase4.rs`。**Manual-required**：真实 BLE GATT（Windows `windows` / CoreBluetooth / BlueZ）、物理 iPhone p95、蓝牙关 30s 真机复现。

**v1.23 落地范围（补上电脑侧那段真机传输栈）**：`tetherly-node::ancs`（装配点 + `AncsInbound` 通道）+ `tetherly-node::ancs::win_gatt`（Windows WinRT BLE GATT，自包含 `#[cfg(windows)]`）；`Node::spawn_ancs_pump` 把入站值接到 `ancs_*` 入口并驱动 `ancs_tick`；CLI `--ancs <addr>`。**Manual-required 不变**：macOS（CoreBluetooth）/ Linux（BlueZ）栈**仍未实现**（`open()` 诚实 `Ok(None)`），物理 iPhone 的运行时行为需真机验证。§9.4 硬约束在真机栈中逐条落实（订阅顺序、回调不写 Control Point、Control Point 串行 + 10s ack 上界、字节拼接、退避、uid 去重）。

### Phase 5 — 远程桌面（屏幕串流）（约 4 周）

| ID | 验收 | 状态（v1.8） |
|---|---|---|
| M5.1 | 已配对桌面间单向看屏，p95 帧间隔在预算内 | **已实现（真机闭环）**：Windows DXGI Desktop Duplication + zstd 无损 + GDI BitBlt；p95 帧间隔计量（`ScreenIntervalTracker`）+ UI 暴露。**跨设备真机已验（v1.8 AdbSource）**：云手机（Galaxy Fold 720x1280）→ Windows 双节点，1063 帧发出 / 1060 帧收达 / 0 丢帧，GDI 窗口画面与真机屏 ±8 容差 97.6% 一致（残差为动画相位 + DWM 色彩管理），p95 帧间隔 ≈0.9–1.45s（`adb screencap` 源速率主导，诚实计量）。**DXGI 采集端真实像素捕获已验（v1.9）**：注入制造的真实桌面变化（Win 键开/关开始菜单）被 DXGI 捕获为 1920x1080 真实帧（8,294,400 字节 = w*h*4 精确）。跨桌面 p95 数据仍 Manual-required。**macOS / Linux 已实现（v1.19）**:`MacSource`（CoreGraphics 采集）与 `X11Source`（MIT-SHM 采集 + `PutImage` 呈现）按目标编译验证；真机采集仍 Manual-required |
| M5.2 | **未本机点击允许，绝不推流**；载荷不含可执行指令 | **CI 绿**：`ScreenState` 门控；无 `allow` 时 `Start` 被拒并计数 |
| M5.3 | 对端断开 → 状态回 `Idle`，seq 清零，重连不重放旧帧 | **CI 绿**：`on_peer_gone` + seq 窗口（tests/phase5.rs） |
| M5.4 | 45720 拒绝配对；配对只在 45717 | **CI 绿**：`resume_only` 门（`ScreenRequiresTrust`） |
| M5.5 | 加分：硬件编码 / 真机注入 | **未做**，不挡退出。OpenH264/rav1e 端口已就位 |

**v1.7 落地范围/豁免（2026-10-10）**：远程桌面原为 §2.3 v1 非目标，本项目方要求引入 Phase 5 并同意**先无损帧 + 后接编码器**（编码器作为 `ScreenSource` 端口实现后接），构成 Phase 5 开工豁免（照 Phase 4 豁免样式）。本次交付为 CI 切片：`tetherly-core::screen` 帧+控制编解码、强制本机同意 M5.2、45720 独立端口 + `ScreenRequiresTrust` 门、`tetherly-node` 屏幕锚点、UI「远程屏幕（实验）」、`tests/phase5.rs`。RustDesk 仅作架构借鉴，AGPL 不可链，`deny.toml` 已 ban。**Manual-required 不变**：真机 DXGI 采集 / 编码器 / 呈现 / p95 真机验证。M5.5 未做，不挡退出。

**v1.8 真机闭环豁免续（2026-10-11）**：Phase 5 真机闭环落地——`DxgiSource`（DXGI Desktop Duplication 采集，`AcquireNextFrame` WAIT_TIMEOUT→`Ok(None)`）、`ZstdCodec` + `EncodedSource`（无损编码可换端口）、`SystemSink`（GDI BitBlt 同步呈现）、`TMF1` 分片 + `ScreenAssembler` 重组、接入侧 decompress 链、p95 计量。`tetherly run` 注入 `SystemSink` + 16ms 帧泵（非 Streaming 跳过 grab）。本机验证：DXGI 设备/adaptor/output/duplication 初始化成功、WAIT_TIMEOUT→None 空闲路径正常、GDI 呈现成功（60/60）。**AdbSource 真机 E2E（同日）**：云手机（Galaxy Fold 720x1280）屏幕经 `--screen-adb` 全链路推流至第二 Windows 节点 GDI 窗口——1063 帧 / 1060 收达 / 0 丢帧、每帧解压后精确 720x1280x4 BGRA、像素 ±8 容差 97.6% 一致、p95 ≈0.9–1.45s（源速率主导）。**UI 两处修复**：`/api/screen/start` 自动拨号 screen 通道（此前 visitor 从不调 `open_screen`，真机 Start 必失败）；visitor 接收计数（`screen_rx_stats`）接入 Snapshot（此前 B 端 UI 永远显示 0）。 同日性能专项：`ZstdCodec` 持久 bulk 上下文（1080p 压缩 16.1→12.6ms / 解压 4.7→4.1ms）、`ScreenAssembler` 偏移式重组（3.0→1.28ms）、`SystemSink` 单拷贝直写 DIB、`[profile.release]` lto=thin + codegen-units=1、`AdbSource` 持久长度前缀 PNG 流（每帧省 adb spawn，grab 543→436ms；带宽实测 overlay ~187KB/s ⇒ raw 3.7MB/帧=17.5s 不可用、PNG 31KB/帧=0.17s，故 PNG 为默认、`TETHERLY_ADB_RAW=1` 走 raw 仅限高速链路）；基准工具 `cargo run -p tetherly-node --example bench_screen --release`。注意：dedup 生效后静止屏幕的 p95 帧间隔反映的是**内容变化周期**而非传输成本。**Manual-required（收窄）**：DXGI 采集端实际像素捕获（需交互桌面）、Windows DXGI 跨桌面 p95、真实开放编码器 OpenH264/rav1e、macOS/Linux 实现。Wayland 仍标「不支持」。

**v1.19 macOS/Linux 采集落地(2026-10-15)**:补掉 §9.7 与 §14 长期标为「下一迭代」的macOS/Linux 采集缺口。`crates/tetherly-node/src/screen/macos.rs`(`MacSource`,CoreGraphics `CGDisplayCreateImage` → 显式 BGRA8 `CGBitmapContext`,请求 16 字节对齐 stride 并按框架实际 stride 去行尾填充；屏幕录制权限被拒→诚实 `ScreenRefused`而非空白帧；呈现需 AppKit `NSWindow`,因此诚实拒绝）与 `crates/tetherly-node/src/screen/x11.rs`(`X11Source`,MIT-SHM 快路径 `shm_create_segment`+`shm_get_image` + plain `GetImage` 回退,要求 ≥ 1.2 并按 `reply.size` 精确读取；呈现为 override-redirect 窗口 + `PutImage`，depth 取自根而非硬编码)。两者按 `cred.rs` 先例各为**自包含文件**（只 import 绝对路径），`Cargo.toml` 加目标依赖（macOS `objc2-core-*`(default-features=false)；Linux `x11rb`(shm)+`libc`），**未引入新包**（均随 `arboard` 已在 lockfile）。**验证**：两后端在本机向 `x86_64-apple-darwin`/`x86_64-unknown-linux-gnu` 以探针 crate（含整个 `screen.rs` + 桩 `zstd`）完成 `clippy -D warnings` 类型检查，探针用后即删；7+7 条后端单元测试为纯逻辑、不触显示器，由 CI 在 `ubuntu-latest`/`macos-latest` 原生执行。**Manual-required（不变）**：真机采集（macOS 需屏幕录制权限、X11 需 X server）、macOS 呈现（NSWindow）、Wayland portal、开放编码器（OpenH264/rav1e）、硬件编码。本次不宣称 macOS/Linux 真机远程桌面可用。

**v1.9 键鼠真机闭环豁免续（2026-10-11）**：Phase 3 真机链落地——`SystemInputSink`（SendInput 注入端口）+ `spawn_input_capture`（LL 钩子捕获，键盘 hook 座位门控）+ 受控端 trace 镜像转发 + `TIN1` 语义明确（VK 码/1000 单位/mask 位）。注入探针在本机实测：光标过边注入、中心移动、键盘 Win/Esc 注入全部生效，并以注入制造的真实桌面变化完成了 DXGI 像素捕获验证（M5.1 收窄）。`Cargo.toml` 新增 `Win32_UI_Input_KeyboardAndMouse` feature。**Manual-required（收窄）**：双桌面 100 次物理往返（M3.1 device）、macOS/Linux 注入与捕获（CGEvent/uinput/portal+libei）、跨桌面 p95。

**v1.10 Android 原生库落地（2026-10-11）**：补齐 Phase 1 的 §9.6 诚实缺口——新增 `crates/tetherly-android`（`cdylib`+`rlib`）产出 `libtetherly_android.so`（arm64-v8a）：**真 Ed25519 + X25519** 身份（`Identity::from_secrets`，取代 v1.3 `idPk=sha256(idSk)` 占位）、与桌面同构的 `identity.bin`/`trusted.json`、桌面**同一条** `dial_session`（SPAKE2 首配 + Noise IK resume）、`SessionConfig.caps` 显式广告 `["notify","clip","file"]`（永不 `input`/`screen`）、JNI 薄壳（`unsafe` 仅豁免 `jni.rs`）。**真机已验**：`android_probe` 推到两台 Galaxy Fold（SM-F900F / arm64-v8a / Android 13），经 `adb reverse` 打隧道到宿主桌面节点（45717）——首跑真实配对、双方 `trusted.json` 落盘、桌面候选 `has_otp:true`（正文不入快照/日志）；二次以**错误 PIN** 仍连上（证明 Noise IK resume）。CI `android` job 先 `cargo ndk` 出 `.so` 再跑 Gradle，`jniLibs/` 入 `.gitignore`。**Manual-required（收窄）**：物理 Android M1.1 **p95 计量**与 8h soak（需真通知链路 + 计时）；`.so` 缺失这一阻塞项已消除。APK 内 Kotlin→JNI→会话的 UI 全链路已于 **v1.18 真机实测关闭**(见 §12 M1.1(v1.18 更新) 与 1.18 变更块)。

### Phase 6 — 单独立项

Android 被控、WinFsp 挂载、文件持久续传、Linux ANCS 体验、嵌入 EasyTier FFI。(notify.reply 于 v1.11 结项;文件持久续传于 v1.12 结项;Windows 凭据库于 v1.13 结项;均不再列入 backlog。)

---

## 13. 测试

| 层 | 内容 |
|---|---|
| 单测 | OTP、帧、状态机、白名单、路径净化、去重、device_id |
| 向量 | Argon2+SPAKE2+Noise prologue |
| proptest | `tetherly-core/tests/decoders.rs`（27）：控制 JSON 全类型 + notify/clip/file/caps/input/screen 往返；每个入站解码器任意输入不 panic（帧、屏幕帧、控制 msg、屏幕重组、输入、ANCS、Hello、device_id、PIN、路径净化）。`tetherly-net/tests/decoders.rs`（7）：长度前缀/带标签/文件头任意字节不 panic、oversized 拒绝、合法头往返、TFL1 兼容。**v1.16 落地**；**v1.17 扩展到有状态状态机**：`tetherly-core/tests/state_machines.rs`（11 条 `proptest` + 7 条确定性见证）以任意**操作序列**驱动 `ReplayGuard`/`TokenBucket`/`NotifyLimiter`/`ClipLimiter`/`NotifyHub`/`FileHub`/`ResumeTable`/`ScreenSender`/`ScreenReceiver`/`ScreenAssembler`/`AncsIngress`，每步后断言安全契约（重放 id 不再接受、限流不越界、文件名安全且不自接受、续传偏移对齐且绑源、无本机同意不推流、非 `Ready` 的 ANCS 回调惰性且不暴露既有 uid） |
| fuzz | `tetherly-core/tests/fuzz_decoders.rs`（2）：确定性 xorshift 变异驱动（稳定工具链替代 libfuzzer）：合法语料 → 6 类变异 → 全部解码器不 panic，`catch_unwind` 捕获；固定种子可复现。**v1.16 落地**；**v1.17 把同一确定性驱动思路推广到状态机操作序列**（`state_machines.rs`）；libfuzzer/`cargo-fuzz`（nightly）仍 Manual-required |
| 集成 | `tests/loopback.rs`：配对、错误 PIN、MITM Hello。`tests/phase1.rs`：notify、clip TTL、insert 拒绝、persist 重拨、日志脱敏、文件拒绝/接受、**跨进程续传(M1.8)**、**`tetherly pin` 活节点回环 API 回归(v1.18:** `pin_shown_by_the_live_node_api_is_the_one_that_pairs`)**、**`tetherly notify` 活节点回环 API 投递回归(v1.20:** `notify_posted_to_the_live_node_api_reaches_the_paired_peer`,断言 `/api/notify` 文本抵达对端候选、未连接 `--peer` 被 400)**。`tests/phase2.rs`：缺侧车、停侧车、无 mDNS 单播、LAN 优先。`tests/phase3.rs`：45719 拒绝配对、过边按键、断线回光标、聚焦时剪贴板。`tests/phase4.rs`：M4.1 模拟时延、M4.2 白名单门控打开、M4.3 重连不连弹、Control Point 串行、分片重组、`Removed` 清候选、退避、**m4_5 入站泵端到端(v1.23:** `m4_5_the_transport_pump_subscribes_in_order_and_feeds_the_ingress`：断言订阅顺序 Data Source 先、回调只入队而 tick 才写 Control Point、DataSource 成候选、`Disconnected` 落 `Backoff`)**。`tests/phase5.rs`：M5.1 consent-then-start 帧序严格递增、M5.2 Start-before-allow 被拒且计数、M5.3 对端离开→`Idle` 重置 seq、M5.4 45720 拒绝未配对 dial 。**v1.19 后端单元测试**:`screen/macos.rs`(7 条:stride 16 字节对齐、去行尾填充、截断/窄 stride 拒绝、BGRA8 位型、`present` 点名 NSWindow)、`screen/x11.rs`(7 条:24 位 alpha 填充、去 stride、`to_frame` 打包/校验、非 Bgra8 拒绝)——纯逻辑、不触显示器,CI 在 `ubuntu-latest`/`macos-latest` 原生编译并执行。**v1.22 内层帧预算回归**:`tetherly-crypto/tests/limits.rs`(2 条:真实 IK 握手后实测明文上限 65519、满额内层帧可加密且多 1 字节被拒)、`tetherly-net/tests/screen_wire.rs`(1 条:4 MiB 屏幕帧分片经真实会话往返重组逐字节相等)、`tetherly-net/tests/backpressure.rs`(2 条:v1.21 写停滞——不读对端得 `WriteStalled`、读对端不误报)。**v1.23 WinRT 传输栈单测**:`tetherly-node/src/ancs/win_gatt.rs`(4 条:Windows 地址记法 `:`/`-`/裸十六进制解析且首字节为高位、6 类畸形输入拒绝、四个 ANCS UUID 的 LE 解码 ↔ 规范 GUID 逐字段一致、畸形 UUID 报错)——纯逻辑、不开蓝牙,`windows-latest` 原生编译并执行 **v1.24 文件/网卡边界回归**：`tetherly-core/src/bind.rs`（1 条：`classify` 九类原因 + 「只有 `Listen` 会 listen、非 `Listen` 必给 reason」的不变量）、`tetherly-core/src/filexfer.rs`（1 条：超限报价 `FileTooLarge` 且不入表、恰好等于上限可接受）、`tetherly-net/src/filechan.rs`（1 条：通道头声明超 `max_size` → `TooLarge` 且零字节缓冲）、`tetherly-node/src/lan.rs`（1 条：`interface_report` 与 `lan_ips()` 必须同一批地址）、`tests/phase1.rs::an_oversized_offer_is_refused_without_dropping_the_session`（端到端：超限报价不成为候选且会话存活） |
| 真机 | 发版清单：Win × Android；Phase 4 再加 iPhone |

PR 守门：loopback + clippy + deny。BLE 与 DeskFlow 不挡 PR。

---

## 14. CI、发版、默认端口

- **CI-First 开发与验证准则（v1.25 确立）**：本项目跨多端与复杂环境（Windows WinRT/DXGI/SendInput、macOS CoreGraphics、Linux glibc 2.31/X11、Android NDK JNI），多平台矩阵构建与安全合规**全面以 GitHub Actions 为核心准绳**。本地开发机保持纯净，仅负责代码实现与单机快速逻辑验证；所有的多平台矩阵测试（Windows/macOS/Ubuntu）、Android NDK 原生构建与 Gradle 打包、`cargo deny` 许可/漏洞守卫，以及 Linux 发布容器打包**一律由 GitHub CI 承担**。开发者通过 `git push` 触发，借助 GitHub CLI（`gh run list`、`gh run watch`）验收云端构建。
- **CI 工作流架构 (`.github/workflows/ci.yml`)**：
  1. `check` 矩阵作业：在 `ubuntu-latest`、`windows-latest`、`macos-latest` 三平台上并行执行 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings` 以及 `cargo test --workspace --locked`。任何平台格式、警告或测试不通过即时熔断。
  2. `android` 容器化作业：在 `ubuntu-latest` 上安装 Rust `aarch64-linux-android` 目标与 `cargo-ndk`，编译生成 `libtetherly_android.so`（arm64-v8a）放入 `android/app/src/main/jniLibs`；再经 Java 17 (Temurin) + Gradle 8.9 原生执行 `:app:testDebugUnitTest` 与 `:app:assembleDebug`。构建产物 `jniLibs/` 动态产生、绝不入版本库。
  3. `deny` 安全合规作业：使用 `EmbarkStudios/cargo-deny-action@v2` 针对 RustSec 漏洞数据库与 `deny.toml` 进行 `advisories`、`licenses`、`bans`、`sources` 四重核查，杜绝 AGPL/GPL 链入与已知 CVE 依赖。
- **发布产物与容器守卫 (`.github/workflows/release.yml`)**：
  - Linux 发布二进制在 **`debian:bullseye`(glibc 2.31) 容器**内构建并加 `GLIBC_2.32+` 守卫（`objdump -T` 打印实际地板），确保符号地板 **≤ 2.31**，完全覆盖 Debian 11 / Ubuntu 20.04 及更新主机。容器内强制 `bash` 且 `sources.list` 指向 `archive.debian.org`。
  - Windows 与 macOS 产物分别在 `windows-latest` 与 `macos-latest` 上编译为 `tetherly` 与 `tetherly-cli` 剥离二进制。
  - 打 tag `v*` 时触发 `publish` 作业，自动将三端产物归档并发布至 GitHub Releases。
- 版本号 workspace 统一 bump。
- 控制 **45717/tcp**，文件 **45718/tcp**，键鼠 **45719/tcp**，远程屏幕 **45720/tcp**。本机 UI **45716/tcp** 仅 127.0.0.1。DeskFlow 网关默认关。
- 本机 UI **45716** 同时是 CLI 的**控制面入口**(v1.20):`tetherly pin` / `dial` / `notify` / `send`(v1.25) 均以它为回环客户端目标(`POST /api/pin` / `/api/dial` / `/api/notify` / `/api/file/send`),不再自起节点。该 HTTP 面**永不**绑非回环地址,且请求须带 `Host: 127.0.0.1|localhost`,否则 403。
- 安装程序提示放行 45717–45720；默认 **LAN** 监听：`127.0.0.1` + RFC1918 / 链路本地，不含公网、**跳过** EasyTier `10.144.144.0/24`（避免 mDNS 打到 TUN）。Phase 2 另绑 overlay CIDR 内的本机 IP，仍永不 `0.0.0.0`/`::`。
- Phase 1 桌面不要求 WebView2（loopback HTTP）。Tauri 壳下一迭代才需要 WebView2、MSVC 运行库；ANCS 需要蓝牙。
- **单文件上限 `FILE_MAX = 512 MiB`（v1.24）**（§12 M1.7）：v1 文件路径**整文件驻留内存**（接收端 `Vec<u8>` 全量 + 发送端 `pending_out` 再存一份），故该上限是**内存边界**而非磁盘策略；在**报价时**按声明大小拒绝（`CoreError::FileTooLarge`，用户看不到候选），在 45718 通道以真实 `max_size` 再钉一次，发送端同向拒绝。流式化（边收边写 + 增量 sha256）完成后可抬高。
- **网卡分类可解释（v1.24）**：`tetherly-core::BindClass`/`classify` 是**唯一真理源**（`should_listen` 即 `classify(..).listens()`）；被跳过的地址在启动日志各占一行 `reason`，并随 `/api/state` 的 `lan` 字段回给本机 UI。关键一类：**CGNAT 段 `100.64.0.0/10`（Tailscale/NetBird）既非 RFC1918 也非链路本地**，v1 **不会**自动绑定，须显式 `--overlay-cidr 100.64.0.0/10`（reason `cgnat-overlay-needs-explicit-cidr`）；`127/8` 非规范别名（如解析器 stub 的 `127.0.0.53`）归 `LoopbackAlias`。

---

## 15. 风险

| ID | 风险 | 缓解 |
|---|---|---|
| R1 | EasyTier LGPL | 侧车，deny LGPL |
| R2 | GPL 键鼠实现 | 自写；网关实验 |
| R3 | iOS 审核 | 可无 App；有则无 VPN |
| R4 | Android 杀后台 | 前台服务 + ROM 说明 |
| R5 | Wayland | beta |
| R6 | 验证码 | 本机、TTL、点击才写、日志脱敏 |
| R7 | BLE 适配器 | 兼容表；向导失败可操作 |
| R8 | 四端成本 | iOS 最小；core 共享 |
| R9 | 8 位 PIN | Argon2id + 锁定 + 绑定身份 |
| R10 | snow/spake2 误用 | 测试向量；不手写曲线 |
| R11 | 虚网无组播 | 单播 peer 列表 |
| R12 | DeskFlow TLS 坑 | 不挡 Phase 3 |
| R13 | AGPL 屏幕采集库（scrap/captrs） | deny 列表 ban + 自有实现（DXGI/核心图形/x11rb） |

---

## 16. 参考仓库

| 仓库 | 用法 |
|---|---|
| sms-pop-rs MIT | OTP、ANCS 时序、UIA 边界 |
| EasyTier LGPL-3 | 侧车组网，不链代码 |
| LocalSend Apache-2.0 | 文件确认 UX |
| Sefirah MIT | Win/Android 体验 |
| DeskFlow GPL | 线格式与端口 24800/TLS，不链代码 |
| KDE Connect | 能力协商思路 |
| ancs4linux | Linux BLE 坑 |
| mcginty/snow | Noise 实现 |
| DecentPaste Apache-2.0 | 配对 UX，算法按本文 |

---

## 17. 载荷例（Noise 内）

```jsonc
{
  "app_id": "com.apple.MobileSMS",
  "app_name": "信息",
  "uid": "42",
  "title": "网易",
  "body": "【网易】验证码：868740，您正在登录网易手机账号",
  "ts": 1760054400,
  "actions": ["copy", "dismiss"]
}
```

不变式：未知 type 默认忽略；业务帧必在 Noise 后；路径与 URL 不来自对端；展示端本地提取 OTP。

---

## 18. Phase 0 开工清单

1. 在 `E:\code\tetherly` 建 workspace（§8 四件套）。
2. 文件头 `Apache-2.0 OR MIT`。
3. 移植 OTP 测试到 `tetherly-core`。
4. `tetherly-crypto`：Argon2id+SPAKE2+PairBind+Noise IK + 向量。
5. `tetherly-cli` + `tests/loopback.rs` 覆盖 M0.2–M0.5。
6. GitHub Actions：M0.7。
7. 不要加 EasyTier、不要加 GUI、不要加 BLE。

---

## 19. Phase 1 开工与落地清单

1. `tetherly-node`：persist identity/trust（temp+rename）、LAN bind 策略、mDNS `_tetherly._tcp`、session 循环、caps.update 广告 `fileport=`。
2. NotifyHub 本地 extract；`insert()` 只挂 UI 点击；候选 120s；剪贴板 OTP 60s 清。
3. 文件：控制面 offer/accept；数据面 45718 `TFL1` + HKDF token；默认确认才收。
4. 本机 UI：`ui/index.html` + `uihttp`，Host 必须是 127.0.0.1/localhost。
5. Android：NLService、前台服务、Wire JSON 无 url action、gradle CI。**JNI `.so` 已落地（v1.10）**：`crates/tetherly-android`（真 Ed25519 + Noise IK，`identity.bin`/`trusted.json` 与桌面同构）→ `libtetherly_android.so`；`jni` 门控 `cfg(target_os="android")`，`unsafe` 仅豁免 `jni.rs`。手机 caps 固定 `["notify","clip","file"]`。
6. `tests/phase1.rs` 覆盖 M1.2–M1.8 与模拟 M1.1/M1.6。
7. **不要**加 EasyTier crate、不要加 DeskFlow 源码、不要改 OTP 规则。Phase 1 清单至此为止；Phase 2 见 §20。

---

## 20. Phase 2 开工与落地清单

1. `tetherly-core::overlay`：CIDR、`PathKind`、JSON/文本 peer IPv4 扫描；默认虚网 `10.144.144.0/24`。
2. `tetherly-node::sidecar`：本机 RPC 15888 → `easytier-cli` → 接口 CIDR；`extra_peers` 作拨号目标。缺侧车返回空列表，不算错误。
3. overlay bind 与 LAN bind 分开；mDNS 只挂 LAN。已有 LAN 则丢 overlay attach。
4. UI `/api/state` 暴露 `lan_only` / `overlay_present` / `overlay_source` / peer `path`。
5. `tests/phase2.rs` 覆盖 M2.2–M2.5 的 loopback 切片；夹具 CIDR 不得撞本机真实 EasyTier TUN。
6. **不要**把 `easytier*` 写进依赖图、不要在 TUN 上发 mDNS、不要改 OTP 规则。Phase 3 见 §21。

---

## 21. Phase 3 开工与落地清单

1. `tetherly-core::input`：`TIN1` 二进制事件、seq 窗口、`InputServer` / `InputClient` / `MemorySink`。无 `cfg(target_os)`。
2. 45719 与 45717/45718 分开 bind（LAN + overlay，仍禁止 `0.0.0.0`）。`SessionConfig.resume_only` 拒绝在此端口配对。
3. `caps.update` 增加 `inputport=`。键鼠帧只走 45719 的独立发送通道，不进控制面 JSON。
4. `tests/phase3.rs` 覆盖未配对拒绝、M3.1 过边按键、M3.2 断线、M3.3 剪贴板。
5. **不要**链接 DeskFlow / Lan Mouse / Barrier / KDE 源码，不要调用 Win32 注入当作 CI，不要改 OTP 规则，不要开 Phase 4。

---

## 22. Phase 4 开工与落地清单

1. `tetherly-core::ancs`：UUID 解码、`NotificationEvent`（8 字节）、Control Point 编码器、`DataAssembler`（按字节重组）、`AncsIngress` 状态机、`AncsTransport` 端口、`MemoryAncsTransport`。**无 `cfg(target_os)`**。
2. 顺序与线程约束写进代码结构，不靠注释：`connect()` 先 `subscribe_data_source` 再 `subscribe_notification_source`；`on_notification_source` 只入队，`tick()` 才写 Control Point；`in_flight` 保证同一时刻一个未应答请求。
3. `AncsIngress` 只用注入的 `Clock`：静默一次后重试、再静默放弃；`APP_NAME_WAIT_MS` 封顶 app 名等待；`NOTIF_ASSEMBLE_DEADLINE_MS` 兜底半截属性；`on_disconnected` 退避 500ms→30s。
4. M4.2 动作只经 `ancs_actions(app_id, &OpenAllowlist)`；「打开」的 url 只来自 `allowlist.url_for`，`open_candidate` 在 `Opener` 端口上执行，**从不**读取通知里的 url。
5. `tests/phase4.rs` 覆盖 M4.1 模拟时延、M4.2、M4.3、串行/回调不写、分片重组、`Removed`、无传输惰性、退避。
6. **不要**在 `tetherly-core` 里放 BLE 栈；不要宣称真机 p95 已达标；不要执行对端 URL；不要改 OTP 规则；不要把 `otp.candidate` 放上线。
7. **v1.23**：真机栈落在 `tetherly-node/src/ancs/win_gatt.rs`（自包含、`#[cfg(windows)]`）——**任何 WinRT 对象都不离开专属 `tetherly-ancs` 线程**；`AncsTransport` 前端只持 `Sender`（按构造 `Send + Sync`）。`ValueChanged` 回调**只拷字节**，写 Control Point 只发生在该线程且**阻塞到 `GattWriteResult::Status == Success`**（前端 ack 等待设 **10s 上界**）；断言「订阅顺序 / 回调不写 / 串行」的回归在 `tests/phase4.rs::m4_5` 与 `win_gatt` 单测。macOS/Linux 栈 `open()` 诚实 `Ok(None)`。不要在 `tetherly-core` 引入 `windows`；不要谎报真机可用。

---

## 23. Phase 5 开工与落地清单

1. `tetherly-core::screen`（**无 OS API**，`#![forbid(unsafe_code)]`，无 `cfg(target_os)`）：`SCREEN_MAGIC=*b"TMV1"`、`SCREEN_PROTO=1`、`SCREEN_SEQ_WINDOW=65536`、`SCREEN_TRACE_MAX=64`；`PixelFormat{Bgra8,Rgba8}`；`ScreenRect`/`ScreenFrame` 编解码（header **31 字节**）；`ControlMsg{Start,Stop,Ack{seq}}`（14 字节）；`ScreenSender`（单调 seq + consent 门控 + push/drain/refused）；`ScreenReceiver`（seq 窗口丢旧/拒大跳/bounded trace）；`MemoryScreenSource`/`MemoryScreenSink`。`ScreenSink`/`ScreenSource` 端口进 `ports.rs`。
2. `tetherly-net`：`SCREEN_PORT=45720`，`NetError::ScreenRequiresTrust`；`session_config_inner` 加 `screen_only: bool`；`hello()` cap 广告 `"screen"`。hello 完只允许已配对对端，`HelloDisposition::Pair` 时 `screen_only` 会话返回 `ScreenRequiresTrust`。
3. `tetherly-node`：`NodeConfig.screen_port`（默认 45720）+ `screen_source: Option<Arc<dyn ScreenSource>>` + `screen_codec: Option<Arc<dyn ScreenCodec>>` + `screen_sink: Option<Arc<dyn ScreenSink>>`；`Shared` 加 `bound_screen`/`screen_out`/`screen_engine`/`screen_observer`/`screen_ctl`/`screen_source`/`screen_codec`/`screen_sink`/`screen_interval`；`spawn_listeners`/`spawn_overlay` 各加 45720 绑定与接受；`attach` 的 caps 加 `"screen"` + `screenport=`；`open_screen`/`accept_screen`/`attach_screen_server`/`attach_screen_client`/`screen_send_frame`；`Node::screen_allow/revoke/start/stop/state/stats/sink_snapshot/port/addr/latency_p95_us`。
4. **红线**：`open_screen` 只建 Noise 通道；开推只由 `screen_allow()` 后的 `screen_start()` 触发；`screen_revoke()` 回 `Idle`；`on_peer_gone` 强制回 `Idle` 并清零 seq。屏幕帧**不携带可执行指令**；被控侧不认识的控制消息一律拒绝，不落盘、不转发。
5. `crates/tetherly-node/src/screen.rs`：**v1.8 真机闭环已落地**——`DxgiSource`（DXGI Desktop Duplication，Windows）、`ZstdCodec` + `EncodedSource<S>`（无损可换端口）、`SystemSink`（GDI BitBlt 同步呈现）。**v1.19 已落地 macOS/Linux 采集**：`MacSource`(`screen/macos.rs`,CoreGraphics `CGDisplayCreateImage` → 显式 BGRA8 `CGBitmapContext` → 去 stride 填充)与 `X11Source`(`screen/x11.rs`,MIT-SHM 快路径 + plain `GetImage` 回退；呈现走 override-redirect 窗口 + `PutImage`,深度取自根而非硬编码)。两者各为自包含文件（只 import 绝对路径），`Cargo.toml` 加目标依赖（macOS `objc2-core-*`；Linux `x11rb`+`libc`）。macOS 呈现需 AppKit `NSWindow`（诚实 `ScreenRefused`）；其余非 Windows/macOS/X11 目标诚实 `ScreenRefused`；Wayland 标「不支持」。真机采集 Manual-required。**不要**链 AGPL/GPL（`deny.toml` 已 ban `hbb_common`/`rustdesk`/`rustdesk-server`/`scrap`）。
6. UI：`/api/screen/allow|start|stop` 路由 + `ui/index.html`「远程屏幕（实验）」区，措辞写清「① 需本机点击允许才推流；② Wayland 暂不支持」。Windows 真机链路已可用（v1.8）；macOS/Linux 与开放编码器仍 Manual-required。
7. `tests/phase5.rs` 覆盖 M5.1 帧序严格递增、M5.2 未 allow 的 Start 被拒且计数、M5.3 对端离开→`Idle` 重置 seq、M5.4 45720 拒未配对 dial。**不要**宣称 Windows DXGI 跨桌面远程桌面已达标：本机 DXGI 采集→zstd→BitBlt 呈现闭环与跨设备（云手机 AdbSource 源）真机闭环均已由 v1.8 落地并实测，DXGI 采集端的像素捕获与 Windows 跨桌面 p95 仍 Manual-required；M5.5（硬件编码 / 真机注入）未做。
8. **不越线**：不链接 RustDesk/AGPL/GPL/LGPL 源码；core 无 `cfg(target_os)`、保持 `#![forbid(unsafe_code)]`；真实采集/编码/注入落在 `tetherly-node` 平台模块并 `#[cfg]` 门控；不绑 `0.0.0.0`；配对永不在 45720；OTP 评分不动；日志与提交信息不含验证码/正文/剪贴板。
9. **v1.8 真机闭环**：`DxgiSource`（DXGI 采集，WAIT_TIMEOUT→`Ok(None)`）+ `ZstdCodec`/`EncodedSource<S>`（无损编码可换端口）+ `SystemSink`（GDI BitBlt 同步呈现）+ `TMF1` 分片 / `ScreenAssembler` 重组 + 接入侧 decompress 链 + `ScreenIntervalTracker`（wall-clock 微秒，bounded 1024，`screen_latency_p95_us()` 暴露 p95）。`tetherly run` 注入 `SystemSink` + 16ms 帧泵（非 Streaming 跳过 grab）。`--screen-adb <serial>` 选用 `AdbSource`（云手机屏幕，PNG→BGRA8，不变屏 `Ok(None)`）。非 Windows 返回诚实 `ScreenRefused`。UI：`/api/screen/start` 自动拨号 screen 通道；Snapshot 的 received/dropped/bytes 来自 visitor 侧 `screen_rx_stats`。
10. **v1.9 键鼠真机链**：受控端 `SystemInputSink`（SendInput：1000 单位→65535 绝对坐标、按钮 mask 位、滚轮、VK 键盘；`NodeConfig.input_sink` 注入，CI 缺省 None）+ 主控端 `spawn_input_capture`（`WH_MOUSE_LL` 恒挂过边检测、`WH_KEYBOARD_LL` 仅远程座位挂、`LLMHF/LLKHF_INJECTED` 过滤防回环）+ 受控端观察者 trace 镜像转发。注入探针实测：过边→光标左缘 (0,539)、中心移动精确、键盘 Win/Esc；DXGI 真实像素捕获 1920x1080（M5.1 收窄）。`TIN1` 语义：VK 码 / 1000 单位 / `BTN_LEFT=1,BTN_RIGHT=2,BTN_MIDDLE=4`。双桌面 100 次物理往返仍 Manual-required。

---

## 24. v1.25 开工与实施清单（文件发送闭环与 CI-First）

1. **GitHub CI-First 构建体系确立**：
   - 任何涉及 Windows（WinRT/DXGI/SendInput）、macOS（CoreGraphics）、Linux（X11/bullseye 容器）以及 Android（NDK arm64-v8a）的跨平台构建、格式检查（`cargo fmt`）、代码规范（`clippy -D warnings`）、单元/集成测试（`cargo test --workspace --locked`）与安全审计（`cargo deny check`）均以 GitHub Actions 为核心准绳。
   - 本地主机仅做轻量代码编写与局部功能调试，禁止在本地维护不可靠的交叉构建链。
   - 发布二进制打包必须由 `.github/workflows/release.yml` 驱动，Linux 严格受限于 `debian:bullseye`（glibc ≤ 2.31 守卫）容器。
2. **生产文件发送入口闭环 (v1.25 核心功能)**：
   - **回环 API 扩充**：在 `tetherly-node::uihttp` 新增 `POST /api/file/send` 路由，接收 JSON 载荷 `{ "path": "<file_path>", "peer": "<device_id>" }`。
   - **API 契约与校验**：
     - 校验 `path` 必须存在且为常规文件，读取长度不超过 `FILE_MAX`（512 MiB）；
     - 目标 `peer` 必须在当前活跃连接列表中（`live_peers`），若无对端或目标不在线返回诚实 400 错误；
     - 校验通过后，调用 `Node::send_file`，内部发起 `FileOffer` 并监听数据通道分块推送。
   - **CLI 客户端扩充**：在 `bins/tetherly-cli` 与 `tetherly` 二进制中增设 `tetherly send <file_path> [--peer <device_id>] [--ui-port <port>]` 子命令。与 `tetherly pin`/`dial`/`notify` 保持一致的 ADR-013 回环客户端机制，严格向 `127.0.0.1:45716` 发送请求，不得自起节点。
   - **UI 呈现**：在 `ui/index.html` 增设「发送文件」表单，允许用户输入或选择本机文件路径，指定目标设备发起直传。
3. **大文件流式架构演进设计 (Blueprint)**：
   - **核心痛点**：目前单文件上限 `FILE_MAX = 512 MiB` 受制于收发双端整文件全量驻留 `Vec<u8>` 内存缓冲。
   - **流式解耦目标**：
     - 发送端：将 `offer_and_send_bytes` 演进为 `offer_and_send_stream`，按 64 KiB 块流式读取磁盘并直接输入 ChaCha20-Poly1305 加密块，内存仅保留单块缓冲；
     - 接收端：分块验证通过后直接写盘 `.part` 文件，同时以 `ring::digest::Context` 流式累积计算 SHA-256 哈希；全部块传输完成后比较最终摘要并原子重命名；
     - 该架构落地后，可平滑将 `FILE_MAX` 提升至数十 GiB 乃至无限制（仅受目标磁盘容量限制）。
4. **测试与回归断言**：
   - `tests/phase1.rs` 新增 `file_sent_via_live_node_api_reaches_the_paired_peer` 端到端回归；
   - 验证通过 `POST /api/file/send` 触发真实双节点文件传输，断言对端接收端收到 offer、点击确认后接收完整、sha256 一致且双方会话存活；
   - 断言文件不存在、超限文件报价被拒且错误码可解释；
   - 维持全量 290+ 测试通过。
