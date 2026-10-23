# ADR-079 屏幕有损编码器与线上协商（草案）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-24，v1.90）**——zstd 保持默认，H.264 显式 opt-in（`--screen-codec h264`）+ 线上协商回退 |
| 日期 | 2026-10-24 |
| 关联 | §9.7「压缩」、M5.5 加分项、ADR-003 单层 Noise、ADR-078 带宽画像 |

## 动机

zstd 无损在 1080p 动态内容上带宽仍高（v1.8 实测静止 PNG 级 ~31KB/帧，动态原始级不可用）。
OpenH264（BSD-3-Clause）/ rav1e（BSD-2/ISC）是 §9.7 预留的两个落点。**接真有损编码器不是
`ScreenCodec` 端口的本地替换**：收端 `SystemSink` 消费 BGRA，发端换 H264 后收端必须先解码，
而双方能力（编解码器集合）在会话建立时并不已知——这**改动线上协议**，按「ADR 不经修订记录
不得违反」必须先定契约。

## 不可直接怼代码的三个原因

1. **线上协商缺失**：当前 `TMV1` 帧头 `compress` 字节只标识 zstd/none；需要
   Hello cap 扩展（如 `screencodecs=h264,zstd`）+ 发端按收端能力选择 + 降级路径
   （对端不识 → 回退 zstd）。这是 §6.5 caps 语义变更。
2. **帧间状态**：H264 P 帧依赖 GOP 与参考帧；断线重连「不重放旧帧」（M5.3）要求
   每次重连强制 IDR；seq 窗口与帧类型需联动（丢帧容忍策略：屏幕流可丢不可等）。
3. **验证边界**：真机 p95 与主观画质无 CI 代理；带宽画像（ADR-078）应作为码率
   自动选择的一等输入，避免又一个人工参数。

## 决定（拟）

1. 协商：Hello cap `screencodecs=`（逗号分隔、字典序稳定），收发两端取交集；
   空集回退 zstd。45720 `resume_only` 语义不变。
2. 首个实现选 **OpenH264**（openh264 crate，BSD-3；asm feature 关闭以保 CI 无 nasm），
   rav1e 因全量编译成本（>10min）暂缓——端口不变，后补即可。
3. `compress` 字节新增 `0x03 = H264`（annex-B，无 B 帧、baseline、每重连 IDR）；
   帧头不变长，旧收端遇未知 compress 按 `MUST_UNDERSTAND` 语义丢弃整帧不拆会话。
4. 码率 = `PeerBandwidthProfile`（ADR-078）档位映射，人工 `--screen-bitrate` 仅覆盖。
5. 落地形态（v1.90）：**zstd 仍为默认**；操作者显式 `--screen-codec h264 --screen-bitrate <kbps>`（默认 4000）才启用意图，且仅当对端 `screencodecs` 含 `h264` 时实际升级——否则逐对端回退 zstd。带宽画像自动选档（拟 3）留待后续版本，人工 `--screen-bitrate` 先行。

## 落地记录（v1.90）

- `tetherly-core`：`SCREEN_COMPRESS_H264=3`；`ScreenFrame.compressed: bool` 重构为 `codec_tag: u8`（`compressed()` 保留为方法）；解码端接受 1|3、未知 tag 维持拒绝。
- `ScreenCodec` 端口新增 `wire_tag/compress_frame/decompress_frame`（带默认实现的维度感知变体），zstd 走默认路径零改动。
- `tetherly-node`：`I420::from_bgra`（BT.601 limited-range 定点 + 2×2 盒式色度抽样）、`H264Codec`（几何变化重建编码器=强制 IDR；解码 `write_rgba8`+原地 RGBA→BGRA）、`NegotiatedCodec`（want × peer 双开关）；收端按帧 tag 分发 zstd/H264 解码器。
- 协商：Hello/caps.update 广告 `screencodecs=zstd,h264`；caps 合并钩子置位对端支持；`--screen-codec/--screen-bitrate` CLI 注入意图。
- 测试：转换灰度精确性、往返 PSNR ≥27dB（4:2:0 物理水平与回归裕量平衡）、几何切换 IDR、协商回退矩阵、phase5 E2E `m5_1_h264_stream_roundtrips_through_the_wire`（真 45720 通道 tag-3 往返，呈现帧为解码 BGRA）。

## 验收（剩余）

- CI：编解码往返（固定种子帧序列 → encode → decode → 逐帧 PSNR ≥ 阈值）、
  未知 compress 丢弃不拆会话、协商交集与回退矩阵。
- 真机（Manual-required）：跨桌面 p95 帧间隔、带宽实测对比 zstd、弱网降档。
