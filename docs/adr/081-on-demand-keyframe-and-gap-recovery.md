# ADR-081 按需关键帧请求与序列断洞自愈恢复（On-Demand Keyframe / IDR Request & Gap Recovery）

| 项 | 值 |
|---|---|
| 状态 | **已采纳并落地（2026-10-25，v1.95）**——线上控制帧 Tag 3（`KeyframeRequest`）、原子锁存重建编码器强制 IDR、收端序列断洞与解码容错自愈恢复 |
| 日期 | 2026-10-25 |
| 关联 | §9.7「压缩」、ADR-079「屏幕有损编码器与线上协商」、ADR-080「双向帧确认与自适应拥塞背压」 |

## 动机

在 ADR-079（v1.90）引入 H.264 有损编码后，高动态桌面屏幕流在有限带宽下的可用性显著增强。然而 H.264 的 GOP 预测编码高度依赖参考帧链条（Reference Frame Chain）：
1. **背压与网络丢包断链**：在 ADR-080（v1.93 / v1.94）自适应拥塞背压生效时，发送端会根据在途队列及 RTT 超时主动丢弃新捕获帧（`record_drop()`），或者套接字拥塞导致个别帧在传输中掉落。此时后续的 P 帧将失去先验参考底图。
2. **解码雪崩与花屏卡死**：缺少参考帧的 H.264 解码器会解析失败或引发严重的马赛克花屏（Visual Artifacts）。此前查看端只要遇到解码失败就会退出循环（`break`）中断看屏。
3. **缺乏线上逆向自愈信令**：原有的控制报文仅支持 `Start (0)`、`Stop (1)` 和 `Ack { seq } (2)`，接收端即使察觉到帧序列跳跃（Sequence Gap）或解码损坏，也无法通知发送端立即重置编码器生成完整的 IDR 关键帧（带 SPS/PPS），导致必须手动断开重新开启屏幕流。

## 决定

1. **协议层新增线上信令（Tag 3）**：
   - 在 `tetherly-core` 的 `ControlMsg` 中增加枚举项：`KeyframeRequest`。
   - 线上编码：14 字节标准控制帧，Tag = `0x03`，seq 字段填 0。
   - 解码端匹配 Tag 3 并反序列化为 `ControlMsg::KeyframeRequest`。
2. **硬件/编码器端口原子锁存与 IDR 重建**：
   - `ScreenCodec` 抽象端口扩展默认方法 `fn force_idr(&self) {}`。
   - `H264Codec` 维护 `force_idr: AtomicBool`。当接收到 `KeyframeRequest` 或发送端主动丢弃帧触发背压时，将该原子标记置为 `true`。
   - 在下一次 `encode_with` 执行时，若检测到 `force_idr == true`，编码器立即销毁旧状态并重新构建 `openh264::encoder::Encoder` 实例，从而向码流中强制写入 SPS/PPS 参数集与 NAL Type 5（IDR 关键帧），完成解码基准重置并重置该标记。
3. **接收端序列断洞感知与韧性容错**：
   - `ScreenReceiver::apply` 新增断洞检测：当 `last_seq != 0` 且 `frame.seq > last_seq + 1` 时，判定出现丢帧断洞，自增统计 `stats.gaps`。
   - 在 H.264 模式下检测到断洞时，接收端会通过 45720 会话异步向发送端发回 `ControlMsg::KeyframeRequest`。
   - 接收端查看泵在解码错误时，不再盲目终止会话退出，而是记录警告日志、丢弃坏帧、向主机发送 `ControlMsg::KeyframeRequest` 并继续处理下一帧，实现完全无感知的自愈自恢复。
4. **遥测与多维控制接口同构暴露**：
   - `ScreenStats` 与 `ScreenStatusReport` 新增 `keyframes_requested` 与 `gaps` 计数器。
   - 回环 HTTP API 新增 `POST /api/screen/keyframe`。
   - 统一 CLI 提供 `tetherly screen keyframe` 触发命令，且 `tetherly screen status` 终端透出 `Keyframe Req` 与 `Gaps` 指标。

## 落地记录（v1.95）

- `crates/tetherly-core/src/screen.rs`:
  - `ControlMsg::KeyframeRequest`（Tag 3）。
  - `ScreenStats` 新增 `keyframes_requested: u64` 和 `gaps: u64`。
  - `ScreenSender::on_control_with_time` 拦截 Tag 3 累加请求指标。
  - `ScreenReceiver::apply` 断洞判定并累加 `gaps`。
  - 核心单测 `keyframe_request_and_gap_detection_tracking`。
- `crates/tetherly-core/src/ports.rs`:
  - `ScreenCodec` 增补 `fn force_idr(&self)`。
- `crates/tetherly-node/src/screen.rs`:
  - `H264Codec` 与 `NegotiatedCodec`、`ScreenCodecPort` 实现 `force_idr()`。
  - `encode_with` 重建 openh264 编码器生成 IDR。
- `crates/tetherly-node/src/runtime.rs`:
  - `ScreenStatusReport` 扩充 DTO 字段。
  - 监听服务收到 `KeyframeRequest` 驱动 `force_idr()`。
  - 查看端解码失败发送 `KeyframeRequest` 且不 break。
  - 发送端背压主动丢帧时自动置位 `force_idr()`。
- `crates/tetherly-node/src/uihttp.rs` & `bin/tetherly.rs`:
  - 路由 `POST /api/screen/keyframe`。
  - 命令 `ScreenCmd::Keyframe` 与 `ScreenCmd::Status` 指标扩展。
- `tests/phase5.rs`:
  - E2E 集成测试 `m5_5_keyframe_request_triggers_idr_recovery`，验证真实 45720 传输、Tag 3 交互与 IDR 恢复全链路。
