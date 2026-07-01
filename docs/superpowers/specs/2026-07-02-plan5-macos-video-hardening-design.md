# Plan 5 · macOS 视频硬化 pass · 设计文档

- 日期：2026-07-02
- 状态：设计已确认（目标驱动自主推进），待生成实现计划
- 范围：修 Plan 4 终审/审查发现的真问题，让 macOS 视频路径**性能优秀 + 一定可用**。纯 bug 修复级，无新能力。
- 上游：Plan 3/4 已合并到 main。本 pass 依赖它们的结构（`agent/src/video/*`、`agent/src/input.rs` 的 `InputInjector`、`packages/web/src/pages/SessionView.tsx`、`packages/web/src/rtc.ts`）。

## 1. 目标

消除 Plan 4 终审列出的真问题，使 macOS 远程访问在真实使用下稳定、省资源、无卡键：

1. 关闭一次会话后**抓屏真正停止**，重连不累积 `SCStream`（当前 `mem::forget` 泄漏 + 多流并发抓屏 → CPU/内存无界增长）。
2. 抓屏源头就是 **720p/30fps**，不再原生 retina + 60fps 硬转（省 CPU；`Sample.duration` 与真实帧率一致）。
3. 控制端指针坐标在 `<video>`（`objectFit:contain` 有黑边）上**准确映射**，不在黑边/拉伸处错位。
4. **不卡键**：按住修饰键/鼠标键后失焦/移出/断线，被控端不会残留按下状态。

**非目标（YAGNI）：** 跨平台、硬件编码、码率自适应、多显示器、音频。这些在本 pass 之后按 BACKLOG 推进。

## 2. 组件与改动

### 2.1 SCStream 生命周期（`agent/src/video/sck_capturer.rs`）

- **问题：** `start()` 用 `std::mem::forget(stream)` 保活，流永不停；每次 `PeerSession`→新 `SckCapturer`→新流，重连累积、并发抓屏。
- **要求：** capture 后端拥有自己的流，**会话结束时 `stop_capture()` 并释放**。`VideoPipeline` 的 worker 线程持有 `_capturer`（`Box<dyn ScreenCapturer>`）直到线程退出（Plan 4 已如此），所以只要 `SckCapturer` 的 `Drop` 停流即可级联正确。
- **技术分叉（plan 定）：** `screencapturekit` 的 `SCStream` 是否 `Send`（`ScreenCapturer: Send` 要求）。
  - 若 `Send`：`SckCapturer { fps, stream: Option<SCStream> }`，`start()` 存流不 forget，`impl Drop for SckCapturer { stop_capture() }`。
  - 若非 `Send`（更可能，含 ObjC 对象）：`SckCapturer` 起一个**owner 线程**在其上创建并持有流，暴露一个 stop 通道；`SckCapturer::start` 返回后流由 owner 线程持有，`Drop` 时通过 stop 通道通知 owner 线程 `stop_capture()` 后退出。帧仍经 `sink` 送出（SCK 回调线程 → `Sender<Frame>`）。
- **验收：** 无 `mem::forget`；`Drop` 路径调用 `stop_capture()`；真实停流用 `#[ignore]` 集成或手动冒烟验证（无显示器无法断言）。

### 2.2 SCK 抓屏配置（`agent/src/video/sck_capturer.rs`）

- `SCStreamConfiguration` 设置目标分辨率与帧率：`with_width(1280)`、`with_height(720)`、`with_minimum_frame_interval(CMTime)`（30fps，即 `CMTime{ value:1, timescale:30 }` 或等价构造）。exact API 名 plan 里对 `screencapturekit` 8.0 核实。
- 效果：SCK 直接产 720p、≤30fps，`bgra_to_i420` 的缩放变为轻量或恒等；与 `Sample.duration = 1/30` 对齐。
- `SckCapturer::new(fps)` 接收 fps；分辨率常量与 pipeline 的 `dst_w/dst_h` 一致（1280×720）。

### 2.3 Agent 侧卡键兜底（`agent/src/input.rs`）

- `InputInjector` 的 worker 线程跟踪当前按下集合：`kdown`→加 `code`、`kup`→移除；`mdown`→加 `button`、`mup`→移除。
- **worker 退出前（`rx.recv()` 返回 `Err`，即所有 `Sender` drop = 输入通道关闭/会话结束/崩溃）释放所有仍按下的键与鼠标键**（enigo `key(.., Release)` / `button(.., Release)`）。
- 覆盖：web 崩溃、标签关闭、断线——任何让 data channel 关闭的情况。
- 纯逻辑（按下集合的增删 + 退出时的释放列表）可抽出单测；真实 enigo 释放走现有 `#[ignore]` 风格。用 Plan 3 的测试 sink 无法直接测 enigo 释放，但可测「worker 退出时对 injector 的按下集合产出正确的释放序列」——把释放逻辑抽成纯函数 `fn pending_releases(&PressedState) -> Vec<InputEvent>` 单测。

### 2.4 env 测试串行化（`agent/src/video/mod.rs`）

- `testpattern_env_forces_synthetic_source` 改写 `RD_VIDEO_SOURCE` 全局 env，加 `#[serial]`（`serial_test` 已是 dev-dep），避免与其它读该 env 的测试并行竞争。

### 2.5 convert 硬化（`agent/src/video/convert.rs`）

- `bgra_to_i420`：`dst_w==0 || dst_h==0` 时提前返回空 `I420`（各平面空 Vec、stride 0），不再 `bgra_to_yuv420(..).expect(..)` panic。
- 加一个 **padded-source-stride** 的已知像素测试（`stride > width*4`），验证 `resize_bgra` 的 stride 处理路径（当前只有 `stride==width*4` 被测）。

### 2.6 Letterbox 坐标换算（`packages/web/src/rtc.ts` + `SessionView.tsx`）

- 抽纯函数 `contentRect(el: {width,height}, videoW: number, videoH: number) -> { left, top, width, height }`：给定 `<video>` 元素框与视频固有尺寸，算出 `objectFit:contain` 实际显示内容框（黑边偏移 + 等比缩放后的宽高）。`videoW/H<=0`（无流）时返回整个元素框。
- `SessionView` 的 `mouseCoords` 改为：先取 `videoRef` 的 `videoWidth/videoHeight` + `getBoundingClientRect()`，用 `contentRect` 求内容框，再在内容框内换算 0..1；指针落在黑边则 clamp 到 [0,1]（边缘）。
- 纯函数 `contentRect` + 内容框内的相对坐标换算单测（宽屏视频窄元素、竖屏、等比几种）。

### 2.7 Web 侧卡键兜底（`packages/web/src/pages/SessionView.tsx`）

- 跟踪已按下：`pressedKeys: Set<string>`（`e.code`）、`pressedButtons: Set<number>`。`kdown`/`mdown` 时加入，`kup`/`mup` 时移除。
- **`blur` / `mouseleave` / 组件卸载 时**：对 `pressedKeys` 里每个补发 `kup`、`pressedButtons` 里每个补发 `mup`，然后清空。Esc 释放焦点会触发 `blur`，自然走这条兜底。
- 抽纯函数 `releaseEvents(pressedKeys, pressedButtons) -> InputEvent[]` 单测（用 `parseInputEvent` 校验产物）。

## 3. 错误处理与边界

- SCStream 停流失败 → 记 warn，不崩。
- injector 退出释放：即使 enigo 释放某键失败也继续释放其余（best-effort）。
- Letterbox：无流（videoWidth=0）回退元素框；指针在黑边 → clamp。
- 兜底重复：web 主防线 + agent 兜底可能对同一键都发 release —— 幂等（enigo 对已释放键再 Release 无害；被控端对重复 kup 无副作用）。

## 4. 测试策略

- **Agent 纯单测：** `pending_releases`（按下集合→释放序列）；`bgra_to_i420` 0×0 + padded-stride 已知像素。
- **Web 纯单测：** `contentRect`（几种宽高比）+ 内容框相对坐标；`releaseEvents`（产物过 `parseInputEvent`）。
- **Agent 集成：** injector worker 在通道关闭时产出释放（对纯函数 `pending_releases` 的单测即可覆盖逻辑；真实 enigo 释放 `#[ignore]`）。
- **`#[serial]`/手动：** SCStream 真实停流 + SCK 720p/30fps 出流走手动冒烟（更新 `plan4-video-smoke.md` 或加备注）。
- **回归：** agent `cargo test`（33+2 ignored 增长）、`cargo clippy --all-targets` 干净、Node `npm test`（59 增长）、typecheck、`@rd/web build` 全绿。

## 5. 任务拆分与并行

目录不相交 → agent 轨 ∥ web 轨。

- **Agent 轨（`agent/`，顺序）：** SCStream 生命周期（§2.1）→ SCK 720p/30fps 配置（§2.2）→ injector 卡键兜底（§2.3）→ env `#[serial]`（§2.4）+ convert 硬化（§2.5）。
- **Web 轨（`packages/web/`，并行）：** `contentRect` 坐标换算（§2.6）→ web 卡键兜底（§2.7）。
- **Docs：** 冒烟备注更新。

每任务独立审查 + 整分支终审。

## 6. 新增依赖

无。`serial_test` 已是 dev-dep；其余用既有 crate（`screencapturekit`、`yuv`、原生 DOM）。
