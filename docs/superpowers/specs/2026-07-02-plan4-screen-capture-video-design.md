# Plan 4 · 屏幕捕获 + H.264 视频（macOS 先行）· 设计文档

- 日期：2026-07-02
- 状态：设计已确认，待生成实现计划（writing-plans）
- 范围：MVP 第 5 步——被控端 Rust agent 抓屏 → H.264 编码 → WebRTC video track；控制端把 Plan 3 的占位面板换成真实 `<video>`。**全项目最高风险项**（总体设计 §4.1 `capture`）。
- 上游：总体设计 `docs/superpowers/specs/2026-07-01-remote-desktop-design.md`（§2 视频编码、§4.1 capture、§5⑤、§7 分辨率/权限）。Plan 3（输入注入）已合并到 main，data channel 输入链路在此基础上保持不变。

## 1. 目标

被控端把主屏实时画面推到控制端浏览器：抓屏 → H.264 编码 → WebRTC video track（agent→web，sendonly）→ 浏览器 `<video>` 渲染。Plan 3 的鼠标/键盘 data-channel 注入与此并存于同一 PeerConnection，捕获表面从占位 div 迁到 `<video>` 元素。

**平台范围（MVP）：** 仅 macOS（Apple Silicon 开发机）打通整条链路并稳定出流。抓屏与编码都在 trait 之后，Windows/Linux 与硬件编码留作后续 Plan 的增量。

**成功判据：**
1. 浏览器 SessionView 里实时显示被控端 macOS 主屏画面（先测试图案，后真实桌面）。
2. Plan 3 的键鼠注入仍工作（video track 与 data channel 并存）。
3. macOS 未授予「屏幕录制」权限时，agent 启动给出可操作指引（而非静默黑屏）。
4. 编码/丢帧异常记 warn 不崩；video 协商失败时降级为「仅输入」（Plan 3 行为）。

**非目标（YAGNI）：** 码率/画质自适应、多显示器、音频、分辨率变化重协商、硬件编码、Windows/Linux 抓屏、录制/回放。

## 2. 降风险策略：分阶段「测试图案先行」

这是本 Plan 的核心方法论。整条管线的最硬风险不是抓屏，而是 **H.264 codec 协商 + Annex-B 打包 + SDP video m-line + 浏览器解码**。因此：

1. **阶段 1（先不接抓屏）：** `VideoPipeline` 由一个**程序合成的测试图案**（随帧移动的渐变，带一个走动的方块作为运动/时间参考）驱动 → I420 → openh264 编码 → `TrackLocalStaticSample` → 浏览器显示动图。跑通即退掉最硬的传输/编解码风险，且与抓屏解耦。
2. **阶段 2（接真实抓屏）：** 把测试图案源换成 `SckCapturer`（ScreenCaptureKit 抓主屏）。
3. 色彩转换（BGRA→I420）、IDR 周期、帧率节奏各自纯函数单测。

「浏览器先看到动图」是早期里程碑，出问题时能立刻定位是抓屏还是管线。

## 3. 组件与接口

新增 `agent/src/video/` 模块（拆分职责、trait 边界清晰、可独立测试）。所有平台/编码相关实现藏在 trait 后。

### 3.1 帧与 trait（`agent/src/video/mod.rs`）

```rust
/// 一帧原始像素（捕获源产出）。stride 允许行对齐 padding。
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub stride: usize,          // 每行字节数（>= width*4）
    pub data: Vec<u8>,          // BGRA8888
    pub ts_micros: u64,         // 采集时间戳（单调，微秒）
}

/// 抓屏源：start() 后通过通道产出帧；drop 停止。
pub trait ScreenCapturer: Send {
    /// 启动捕获，帧通过返回的 Receiver 产出。返回主屏尺寸。
    fn start(&mut self, sink: std::sync::mpsc::Sender<Frame>) -> anyhow::Result<()>;
}

/// H.264 编码器：I420 帧 → Annex-B NAL 字节（含周期 IDR 时的 SPS/PPS）。
pub trait VideoEncoder: Send {
    /// 编码一帧，返回 Annex-B 比特流（可能为空/多 NAL）。force_idr 请求关键帧。
    fn encode(&mut self, i420: &I420, force_idr: bool) -> anyhow::Result<Vec<u8>>;
}
```

- `I420`：三平面 YUV420（Y/U/V + strides + w/h）。BGRA→I420 转换用 `yuv` crate。
- trait 都是 `Send`，实现跑在专用线程。

### 3.2 合成测试图案源（`agent/src/video/testpattern.rs`）

- `TestPatternSource: ScreenCapturer`：无系统依赖，按固定帧率产出移动渐变 + 走动方块的 BGRA 帧（尺寸如 1280×720）。用于阶段 1 与集成测试（无显示、无浏览器）。

### 3.3 openh264 编码器（`agent/src/video/openh264_encoder.rs`）

- `Openh264Encoder: VideoEncoder`：封装 `openh264` crate 的 `Encoder`。
- 输入 I420，输出 Annex-B。配置固定码率、帧率、GOP（周期 IDR，例如每 `KEYFRAME_INTERVAL` 帧或 ~2s 一次；`force_idr` 时立即插 IDR）。
- 保证首帧与每个 IDR 附带 SPS/PPS（in-band），满足浏览器 WebRTC H.264 解码要求。

### 3.4 BGRA→I420 转换（`agent/src/video/convert.rs`）

- `bgra_to_i420(frame: &Frame) -> I420`：用 `yuv` crate 做颜色空间转换 + 可选缩放到目标分辨率（如 720p）。
- 纯函数，已知像素单测（纯色 BGRA → 期望 Y/U/V 值，容忍舍入）。

### 3.5 SCK 抓屏（`agent/src/video/sck_capturer.rs`，`#[cfg(target_os="macos")]`）

- `SckCapturer: ScreenCapturer`：用 `screencapturekit` crate 建 `SCStream` 抓主显示器，回调里把 CVPixelBuffer（BGRA）拷成 `Frame` 发进 sink。
- 采集队列由 SCK 管理；本组件只做「SCK 帧 → `Frame`」搬运。
- 权限不足或建流失败 → 返回错误，上层降级 + 指引。

### 3.6 视频管线（`agent/src/video/pipeline.rs`）

- `VideoPipeline::start(capturer, encoder, track) -> VideoPipeline`：起一条专用线程，`capturer` 产帧 → `bgra_to_i420`（+缩放）→ `encoder.encode` → `track.write_sample(Sample{ data: annexb, duration })`。按目标帧率节奏；掉帧时丢旧帧。
- `Drop` 停线程（sink sender 关闭）。
- 编码错误/写样本失败 → `tracing::warn!` 跳过，不崩。
- 与 Plan 3 的 `InputInjector` 同构（专用线程 + 通道 + 生命周期绑 PeerSession）。

### 3.7 接入 PeerSession（`agent/src/webrtc_peer.rs`）

- `accept_offer` 前（或 `build` 里）创建 H.264 `TrackLocalStaticSample`（`RTCRtpCodecCapability { mime_type: "video/H264", .. }`）并 `pc.add_track(track)`。仅当远端 offer 含 video m-line 时该 track 生效为 sendonly。
- 连通后（或 track 就绪即）启动 `VideoPipeline`，pipeline 持有该 track 的写入端；`VideoPipeline` 存进 `PeerSession`（类比 Plan 3 的 `_injector`），随会话释放。
- 源的选择由环境变量 `RD_VIDEO_SOURCE` 决定：`testpattern` → `TestPatternSource`，`screen`（默认）→ `SckCapturer`。阶段 1 的端到端用 `testpattern`（也是集成测试与无显示环境的默认走法）；阶段 2 完成后默认 `screen`。这样测试图案不是被删掉而是可随时回退定位问题。
- data channel 输入注入（Plan 3）保持不变。

### 3.8 权限检测（`agent/src/permission.rs` 扩展 或 video 内）

- macOS「屏幕录制」（Screen Recording）权限：首次检测并给指引（「系统设置 → 隐私与安全性 → 屏幕录制」）。复用 Plan 3 `check_input_permission` 的模式（`#[cfg(target_os="macos")]`，非 macOS no-op）。检测方式在 writing-plans 阶段定（SCK 建流失败即视为无权限并指引，或用 CoreGraphics 预检）。

### 3.9 控制端（`packages/web/`）

- `rtc.ts`：`startPeer` 里在建 data channel 后加 `pc.addTransceiver("video", { direction: "recvonly" })`，使 offer 含 video m-line。新增 `pc.ontrack` → 把 `event.streams[0]`（或用 `event.track` 组 `MediaStream`）通过回调 `onRemoteStream(stream)` 交给 UI。
- `SessionView.tsx`：占位「远程屏幕」div → `<video autoPlay muted playsInline>`；`onRemoteStream` 时 `videoEl.srcObject = stream`。Plan 3 的鼠标/键盘/滚轮捕获处理器（含原生非被动 wheel 监听、rAF 节流、事件日志）**迁到 `<video>` 元素**，相对坐标基于 video 的 `getBoundingClientRect()`——逻辑不变。未出流时显示占位提示。

## 4. 数据流（一次出流）

```
Web offer: data channel + addTransceiver(video, recvonly) → SDP 含 video m-line
Agent accept_offer: add_track(H264 sendonly) → answer 含 sendonly video
连通后 VideoPipeline(专用线程):
  capturer → Frame(BGRA) → bgra_to_i420(+缩放720p) → openh264.encode(周期IDR)
    → track.write_sample(Annex-B)  →(webrtc-rs H264 payloader 分片 RTP)→ Web
Web: pc.ontrack → video.srcObject = stream → <video> 渲染
（并行）Web 键鼠 → data channel "input" → Agent enigo 注入（Plan 3，不变）
```

## 5. 错误处理与边界

- macOS 屏幕录制未授权 → 检测 + 指引；SCK 建流失败 → 降级为「仅输入」（不推 video），会话仍可用。
- 编码失败 / `write_sample` 失败 / 转换异常 → `tracing::warn!` 跳过该帧，不崩。
- 远端 offer 无 video m-line → 不启动 pipeline（纯 Plan 3 行为）。
- 掉帧：pipeline 按帧率节奏，落后时丢旧帧（不堆积延迟）。
- 分辨率/多屏变化：MVP 固定主屏 + 固定目标分辨率；变化重协商留后续。
- 关闭：`PeerSession` 释放 → pipeline 线程随通道关闭退出，SCK 停流。

## 6. 测试策略（对齐总体设计 §8）

- **纯单元（Rust）：** `bgra_to_i420` 已知像素（纯红/绿/蓝/灰 → 期望 YUV，容忍舍入）；IDR 周期判定；帧率节奏（给定时间戳序列 → 应发/应丢）；缩放尺寸计算。
- **编码器单元（可能需 openh264 运行时）：** 合成 I420 → `encode` 产出非空 Annex-B，首帧含 SPS/PPS（NAL 类型 7/8）+ IDR（5）。
- **管线集成（无显示、无浏览器，in-process）：** `TestPatternSource` → pipeline → `TrackLocalStaticSample.write_sample` 不报错、能取到至少一个 RTP 样本（类比 Plan 3 `input_loopback`）。
- **SDP 断言：** web 端加了 recvonly video 后，offer SDP 含一条 video m-line；agent answer 含 sendonly H264（可在 agent 侧集成测试断言 answer 含 `m=video` 且方向 sendonly）。
- **SCK 真实抓屏：** `#[ignore]` 本机集成（需屏幕录制权限）。
- **Web 单元（vitest）：** rtc 的 `ontrack`→stream 回调装配；`addTransceiver` 调用（对纯逻辑部分抽函数或用 mock RTCPeerConnection——jsdom 无真实 WebRTC，沿用 Plan 2b/3 的「纯逻辑抽出来测」策略）。
- **回归：** 不破坏 Plan 3 的 agent（23+1 ignored）、Node（57）、typecheck/clippy/web build。
- **手动冒烟：** 更新/新增 `docs/superpowers/plan4-video-smoke.md`：先浏览器看到测试图案，再（阶段 2）看到真实桌面；键鼠注入仍生效。

## 7. 任务拆分与并行

目录互不重叠 → 并行组（agent 轨 ∥ web 轨；控制端逐个分别提交避免 git 抢锁）。**agent 轨内部严格「测试图案端到端先通」再接抓屏。**

- **Agent 轨（`agent/`，顺序）：**
  1. `video` 模块骨架：`Frame`/`I420`/trait + `TestPatternSource`（纯单元）。
  2. `Openh264Encoder`（+ 编码单元测试）。
  3. `bgra_to_i420` 转换（+ 已知像素单元测试）。
  4. `VideoPipeline` + 接入 PeerSession + H264 track，用 `TestPatternSource` 端到端（集成测试 write_sample）。
  5. `SckCapturer` + 屏幕录制权限检测（`#[ignore]` 集成），把默认源切到真实抓屏。
- **Web 轨（`packages/web/`，与 agent 轨并行）：** rtc 加 recvonly transceiver + `ontrack`/`onRemoteStream`（+单测）；`SessionView` `<video>` + 捕获处理器迁移。
- **Docs：** 冒烟文档。

agent 轨与 web 轨唯一契约：SDP video m-line（web 发 recvonly、agent 加 sendonly H264 track）+ H.264 编解码参数（webrtc-rs 默认 codec capability）。每任务独立审查 + 整分支终审。

## 8. 新增依赖

- Agent：`openh264 = "0.9"`（软件 H.264 编码）；`yuv = "0.8"`（BGRA→I420 + 缩放）；`screencapturekit = "8"`（`#[cfg(target_os="macos")]`，SCK 抓屏）。
- Web：无新增运行时依赖（原生 `<video>` + 既有 WebRTC）。
