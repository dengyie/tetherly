# Tetherly 开发文档

| 项 | 值 |
|---|---|
| 产品名 | **Tetherly** |
| 文档版本 | **1.5** |
| 日期 | 2026-10-09 |
| 状态 | Phase 3 CI 切片已落地（45719 Noise IK 只恢复、二进制 TIN1、MemorySink）；真机 SendInput / 双桌面 soak 为 Manual-required；DeskFlow 网关保持关闭；**未开 Phase 4** |
| crate / 二进制 | `tetherly-core`、`tetherly-crypto`、`tetherly-net`、`tetherly-node`（`tetherly` 二进制）、`tetherly-cli` |

本文是实现合同。与代码冲突时改文档或改代码，必须一致。旧名 InterLink 作废。

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
- M1.2 / M1.3（MemoryInsertor）/ M1.4 Win 回环 / M1.5 persist 重拨 / M1.6 日志脱敏 / M1.7 文件确认+10MiB sha256：CI 全绿。M1.1 物理 Android p95、Win+Android 8h、真记事本 UIA、macOS M1.1：**Manual-required** / 「mac 下一迭代」。

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
| 键鼠 | 桌面 ↔ 桌面；自有 Noise 端口。DeskFlow 仅实验网关 |
| iPhone | 电脑用 ANCS/BLE 收新通知，再经 IP 会话转给其他已配对电脑 |

### 2.3 非目标（v1）

- 远程桌面 / 屏幕镜像
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

密钥：系统凭据库优先，否则用户目录文件 ACL 仅本人；进程内 `zeroize`。

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
| `id_sk`、`n_sk` | Windows Credential Manager / macOS Keychain / libsecret；回退 `%LOCALAPPDATA%\tetherly\identity.bin` 等，ACL 仅当前用户 |
| 信任库 | `trusted.json`：`device_id, id_pk, n_pk, alias, paired_at, revoked` |
| 配置 | `config.toml` |

写入：临时文件 + `rename`。忘记设备：标 `revoked` 或删除行。Phase 1 `tetherly-node` 已接线文件回退（Windows `%LOCALAPPDATA%\tetherly`，Unix `~/.local/share/tetherly`，0600/0700）。系统凭据库（Credential Manager / Keychain / libsecret）仍是下一迭代。

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

内层 payload 上限 **64 KiB**。文件块走第二条 TCP：Hello 可省略，用 `file_token` 做 Noise PSK 或把 token 放在 IK payload。`file_token = HKDF-SHA256(ck, "tetherly-file-v1" || transfer_id)`，60s 过期。文件帧上限 256 KiB。

`flags` bit0 = `MUST_UNDERSTAND`：未知 type 则断开。

### 7.2 内层 type（v1）

| 符号 | 值（建议） | 载荷 |
|---|---|---|
| ping / pong | 0x0001 / 0x0002 | `unix_ms: u64` |
| notify.push | 0x0101 | JSON：uid, app_id, app_name, title, body, ts, actions[]（无 url） |
| notify.dismiss | 0x0102 | uid, app_id |
| notify.reply | 0x0103 | stub，可忽略 |
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

默认必须点确认。自动接收默认关。文件名只取 basename，拒绝 `..`。v1 无跨进程续传；同连接可重发块。断线 offer 作废。控制面 offer/accept 走 45717；数据走对端 `caps.update` 广告的 **45718**（`fileport=`）。`file_token = HKDF-SHA256(handshake_hash, "tetherly-file-v1" || transfer_id)`；块 ChaCha20-Poly1305，魔数 `TFL1`，块 64KiB。

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
│   └── tetherly-node/      # LAN 节点、落盘、hubs、本机 HTTP UI、EasyTier sidecar 发现、tetherly 二进制
├── bins/tetherly-cli/      # Phase 0 配对夹具仍保留
├── tests/loopback.rs       # Phase 0
├── tests/phase1.rs         # Phase 1 loopback 双节点
├── tests/phase2.rs         # Phase 2 sidecar unicast / LAN 优先 / 缺侧车
├── ui/index.html           # 本机 UI（loopback HTTP）
├── android/                # NLService + gradle CI
└── docs/
```

其后按阶段加：`tetherly-ancs`、`tetherly-input`、`tetherly-ffi`、`src-tauri`、`ios/`。`android/` 已存在但 JNI `.so` 未构建。

红线：`tetherly-core` 无 `cfg(target_os)`、无 `windows`/`objc`/`jni`。`tetherly-net` 无 `easytier` crate。

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

### 9.5 键鼠（Phase 3）

拓扑在 `config.toml`。断线光标回 server。Wayland 走 portal+libei，不行就标不支持。DeskFlow 网关另开端口、默认关、TLS 未实现就不要宣称兼容。

**v1.5 已落地**：`tetherly-core::input` 编解码与边缘状态机；`tetherly-node` 在 LAN 与 overlay 上另听 45719，会话 `resume_only`。CI 注入是 `MemorySink`，不调用 Win32 `SendInput`。进入帧落在 `x=0` 时保持焦点；离开只在已经进入内部后再回到左缘时发生。剪贴板仍走 45717，不进键鼠帧。

### 9.6 Android（Phase 1）

`NotificationListenerService` + 前台服务。Android 13+ `POST_NOTIFICATIONS`。帮助页教各 ROM 自启动，不搞黑保活。不读短信库。Play 上架需通知使用权声明，内部测试可先 sideload。

**诚实缺口（v1.3）**：Kotlin 侧可截获通知并入队；`NativeBridge` 加载 `libtetherly_android`。无 `.so` 时**禁止**自己拼 SPAKE2/Noise TCP。占位身份 `idPk = sha256(idSk)` **不是**生产 Ed25519。CI 跑 `gradle :app:testDebugUnitTest` + `assembleDebug`。物理机 M1.1 p95 / 8h soak 标 Manual-required。协议与 OTP 路径由桌面双节点 loopback 覆盖。

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
| M1.1 | Android 测试通知含 `524681` 到桌面弹窗，**p95 < 1.0s**（listener 回调 → 窗口可见） | loopback `notify.push` CI 绿、耗时 < 1s。物理 Android p95：**Manual-required**（JNI `.so` 未构建）。macOS M1.1：**mac 下一迭代** |
| M1.2 | 复制后剪贴板为该码；120s 候选消失 | CI 绿（`tests/phase1.rs` + ManualClock） |
| M1.3 | 记事本可填；只读拒绝 | CI 绿（`MemoryInsertor` 空写/只读拒绝）。真记事本 UIA：**Manual-required** |
| M1.4 | Win↔mac 文本剪贴板（有双桌面时）；至少 Win 本机回环 | Win 双节点 loopback CI 绿。Win↔mac：**Manual-required** |
| M1.5 | 杀 Android 进程后自动重连，不重新配对 | persist trust + 无 PIN 重拨 CI 绿。真杀 Android 进程：**Manual-required** |
| M1.6 | 运行日志 `grep 524681` 无命中 | CI 绿（tracing MakeWriter 捕获） |
| M1.7 | 未确认不收文件；确认后 10MB sha256 一致 | CI 绿（reject 无 inbox 文件；accept 10MiB sha256） |

退出：M1.1–M1.3、M1.5–M1.7。Win+Android 8h 无崩溃。macOS 能跑通 M1.1 或文档标明「mac 下一迭代」。

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
| M3.1 | 光标过 Win 右缘到第二台桌面，键盘跟随；脚本往返 100 次丢失 0 | CI：45719 过边 + 按键进 `MemorySink`；core 100 次往返 0 丢失。真机 SendInput / 双桌面：**Manual-required** |
| M3.2 | 对端断开，光标回本机 | CI 绿（输入会话结束 `on_peer_gone`，座位回 Local） |
| M3.3 | 键鼠模式下剪贴板仍可用 | CI 绿（焦点在对端时 `clip.set` 仍走 45717） |
| M3.4 | 加分：DeskFlow 官方 client + TLS 能移动光标。失败则网关标 experimental | **未做**。网关保持关闭，不链 GPL 源码，不挡退出 |

Wayland 不挡退出。真机注入未落地前不得宣称可替代键鼠。

**v1.5 退出裁定**：Phase 3 CI 切片（resume-only Noise、二进制帧、过边、断线、剪贴板并存）已绿。物理双桌面与 OS 注入仍是 Manual-required，**不得开 Phase 4**，除非另下豁免。

### Phase 4 — iPhone ANCS（约 4 周）

| ID | 验收 |
|---|---|
| M4.1 | 系统蓝牙连接后短信通知 p95 < 2s 到 Win 或 mac |
| M4.2 | 仅白名单显示「打开」；payload 带 url 也不打开 |
| M4.3 | 关蓝牙 30s 再开：重连且不连弹旧通知 |
| M4.4 | 可选 iOS App：同 LAN 50MB sha256；无此 App 不挡退出 |
| M4.5 | iOS App 无 VPN / Network Extension |

### Phase 5 — 单独立项

Android 被控、notify.reply、WinFsp 挂载、文件持久续传、Linux ANCS 体验、嵌入 EasyTier FFI。

---

## 13. 测试

| 层 | 内容 |
|---|---|
| 单测 | OTP、帧、状态机、白名单、路径净化、去重、device_id |
| 向量 | Argon2+SPAKE2+Noise prologue |
| proptest | 控制 JSON 往返 |
| fuzz | 内层帧、ANCS tuple（Phase 4） |
| 集成 | `tests/loopback.rs`：配对、错误 PIN、MITM Hello。`tests/phase1.rs`：notify、clip TTL、insert 拒绝、persist 重拨、日志脱敏、文件拒绝/接受。`tests/phase2.rs`：缺侧车、停侧车、无 mDNS 单播、LAN 优先。`tests/phase3.rs`：45719 拒绝配对、过边按键、断线回光标、聚焦时剪贴板 |
| 真机 | 发版清单：Win × Android；Phase 4 再加 iPhone |

PR 守门：loopback + clippy + deny。BLE 与 DeskFlow 不挡 PR。

---

## 14. CI、发版、默认端口

- CI：Windows、macOS、Ubuntu（fmt / clippy `-D warnings` / test `--locked`）。Phase 1 另加 Ubuntu `android` job（Java 17 + Gradle 8.9：`:app:testDebugUnitTest`、`assembleDebug`）与 `deny`（licenses/bans/sources + audit）。
- 版本号 workspace 统一 bump。
- 控制 **45717/tcp**，文件 **45718/tcp**，键鼠 **45719/tcp**。本机 UI **45716/tcp** 仅 127.0.0.1。DeskFlow 网关默认关。
- 安装程序提示放行 45717–45719；默认 **LAN** 监听：`127.0.0.1` + RFC1918 / 链路本地，不含公网、**跳过** EasyTier `10.144.144.0/24`（避免 mDNS 打到 TUN）。Phase 2 另绑 overlay CIDR 内的本机 IP，仍永不 `0.0.0.0`/`::`。
- Phase 1 桌面不要求 WebView2（loopback HTTP）。Tauri 壳下一迭代才需要 WebView2、MSVC 运行库；ANCS 需要蓝牙。

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
5. Android：NLService、前台服务、Wire JSON 无 url action、gradle CI。JNI `.so` 下一刀。
6. `tests/phase1.rs` 覆盖 M1.2–M1.7 与模拟 M1.1/M1.6。
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
