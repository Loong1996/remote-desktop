# Plan 3 · 键鼠注入 · 设计文档

- 日期：2026-07-01
- 状态：设计已确认，待生成实现计划（writing-plans）
- 范围：MVP 第 4 步——控制端捕获鼠标/键盘，经已打通的 WebRTC data channel 传给被控端，Agent 用 `enigo` 注入到本地系统。
- 上游：总体设计见 `docs/superpowers/specs/2026-07-01-remote-desktop-design.md`（§4.1 `input`、§5⑤、§7 输入注入权限、§8 测试策略）。

## 1. 目标

复用 Plan 2b 已打通的 data channel（浏览器↔Agent↔服务端），让控制端的鼠标移动/点击/滚轮/键盘按下抬起，实时注入到被控端。本阶段**尚无视频画面**（那是 Plan 4），因此控制端用一个占位「远程屏幕」面板承载输入捕获，并在界面上回显最近发送的事件流作为验证手段。

**成功判据：**
1. 在 web 占位面板上移动鼠标 / 点击 / 滚轮 / 打字，被控端本机的真实光标随之移动、真实发生点击/滚动、真实输入字符。
2. web 面板实时回显最近若干条已发送事件（`mmove 0.42,0.15` / `mdown left` / `kdown KeyA`）。
3. Agent 端 `tracing` 日志记录收到并注入的事件；非法/非 UTF-8 帧记录告警而非静默丢弃。
4. macOS 未授予「辅助功能」权限时，Agent 启动即给出可操作指引（而非静默失效）。

**非目标（YAGNI）：** 视频画面、坐标随真实分辨率变化的重协商、剪贴板、多显示器、输入回执 ack、触摸/手势、输入法组合（IME composition）。

## 2. 组件与接口

系统分为互不重叠的三块，各有单一职责、明确接口、可独立测试。

### 2.1 协议镜像（Rust 端）——线契约的唯一真相是 TS

- **它做什么：** 把 `packages/protocol/src/input.ts` 的 `InputEvent`（6 个变体）在 Rust 端 serde 镜像，保证两端解析同一 JSON。
- **怎么用：** `serde_json::from_str::<InputEvent>(frame)` 解析入站帧。
- **依赖：** `serde`、`serde_json`（已在用）。
- **位置：** `agent/src/input.rs` 内定义（与注入器同模块）。

Rust 侧类型：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton { Left, Right, Middle }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t")]
pub enum InputEvent {
    #[serde(rename = "mmove")] MMove { x: f64, y: f64 },
    #[serde(rename = "mdown")] MDown { button: MouseButton },
    #[serde(rename = "mup")]   MUp   { button: MouseButton },
    #[serde(rename = "wheel")] Wheel { dx: f64, dy: f64 },
    #[serde(rename = "kdown")] KDown { code: String },
    #[serde(rename = "kup")]   KUp   { code: String },
}
```

- 坐标范围校验（`x,y ∈ [0,1]`）在**注入时**做（收到越界 `mmove` 记 warn 并 clamp），不放在 deserialize——保持 serde 类型纯粹、round-trip 对称。
- TS `input.ts` 的线格式即真相：字段名 `t`/`x`/`y`/`button`/`dx`/`dy`/`code`，变体标签 `mmove/mdown/mup/wheel/kdown/kup`，按钮 `left/right/middle`。以上 serde 属性与之逐一对齐。

### 2.2 注入器 `InputInjector`（Rust 端，enigo）

- **它做什么：** 接收 `InputEvent`，注入到本地系统。
- **怎么用：** `InputInjector::start() -> InputInjector`，返回持有 `Sender<InputEvent>` 的句柄；`handle.send(ev)` 入队；`Drop` 时结束 worker 线程。
- **依赖：** `enigo`（新增）；macOS 下 accessibility 检测的小依赖（见 §2.3）。

**线程模型（架构决策 A1）：** `enigo::Enigo` 是 `!Send`，各平台倾向单线程使用。故：
- `start()` 起一条专用 OS 线程（`std::thread::spawn`），线程内 `Enigo::new(&Settings::default())` 并循环从 `std::sync::mpsc::Receiver<InputEvent>` 取事件、同步注入。
- data channel 回调（async、可能跑在任意 tokio worker 线程）只负责「解析帧 → `sender.send(ev)`」，不碰 `Enigo`。
- 事件在单线程串行注入，天然保序；阻塞式系统调用不占用 tokio worker。
- 若 `Enigo::new` 失败（如 Linux/Wayland 不支持、权限缺失导致初始化失败）：worker 线程打印一次带指引的 `tracing::error`，随后**排空并丢弃**队列事件——会话仍连通，只是无注入，进程不崩。

**事件 → enigo 动作映射：**
| 事件 | enigo 调用 |
|------|-----------|
| `MMove{x,y}` | 取主屏 `(w,h)`（`enigo.main_display()`），`move_mouse((x·w) i32, (y·h) i32, Coordinate::Abs)`；`x,y` 先 clamp 到 `[0,1]` |
| `MDown{button}` / `MUp{button}` | `button(map_button(button), Direction::Press/Release)` |
| `Wheel{dx,dy}` | `scroll(round(dy), Axis::Vertical)` + `scroll(round(dx), Axis::Horizontal)`（非零轴才发；步长取整） |
| `KDown{code}` / `KUp{code}` | `key(code_to_key(code)?, Direction::Press/Release)`；`code_to_key` 返回 `None` 时记 warn 跳过 |

**键位映射 `code_to_key(&str) -> Option<Key>`（完整标准键位表）：**
- 字母 `KeyA`..`KeyZ` → `Key::Unicode('a')`..`('z')`（小写；大小写由另发的 Shift 修饰键自然决定，符合物理键盘语义）。
- 数字 `Digit0`..`Digit9` → `Key::Unicode('0')`..`('9')`。
- 符号键（`Minus`/`Equal`/`BracketLeft`/`BracketRight`/`Backslash`/`Semicolon`/`Quote`/`Backquote`/`Comma`/`Period`/`Slash`）→ 对应**未按 Shift 的**字符 `Key::Unicode`。
- 功能键 `F1`..`F12` → `Key::F1`..`F12`。
- 方向键 `ArrowUp/Down/Left/Right` → `Key::UpArrow/DownArrow/LeftArrow/RightArrow`。
- 修饰键 `ShiftLeft/ShiftRight` → `Key::Shift`；`ControlLeft/Right` → `Key::Control`；`AltLeft/Right` → `Key::Alt`；`MetaLeft/Right` → `Key::Meta`（macOS 上即 Command）。左右不区分（enigo 抽象层不保证区分，MVP 不需要）。
- 编辑/导航：`Enter`→`Return`、`Tab`→`Tab`、`Escape`→`Escape`、`Backspace`→`Backspace`、`Delete`→`Delete`、`Space`→`Space`、`Home/End/PageUp/PageDown/Insert`→ 对应 `Key::*`、`CapsLock`→`CapsLock`。
- 小键盘 `Numpad0`..`Numpad9` → 对应数字 `Key::Unicode`（MVP 简化，不区分小键盘语义）。
- 未收录的 `code` → `None`，记 `tracing::warn!` 跳过（不崩、不猜）。
- **修饰键组合**（如 `Cmd+C`、`Shift+A`）**无需特殊处理**：控制端忠实地按序发 `kdown ShiftLeft` → `kdown KeyA` → `kup KeyA` → `kup ShiftLeft`，Agent 忠实重放 Press/Release，组合在系统层自然生效。

### 2.3 权限检测与指引（Rust 端）

- **它做什么：** 启动时探测输入注入是否可用，不可用时给出平台相关的可操作指引。
- **接口：** `check_input_permission()`，在 Agent 启动流程调用一次，返回是否可用并已打印指引。
- macOS：`#[cfg(target_os = "macos")]` 用轻量 accessibility 检测（`AXIsProcessTrusted`，经 `macos-accessibility-client` 或等价小 crate）。未授权则 `tracing::warn!`：「鼠标/键盘可能无响应：请在 系统设置 → 隐私与安全性 → 辅助功能 中授权本程序」。
- Linux：`#[cfg(target_os = "linux")]` 仅打印一次说明——X11 可注入；Wayland 对合成事件有限制，若 `Enigo::new` 失败会在 §2.2 的 worker 线程再给指引。
- 其他平台：no-op。
- 该检测不阻断连接，只提示。

### 2.4 接入 PeerSession（Rust 端）

- `agent/src/webrtc_peer.rs`：把 `wire_echo` 换成 `wire_input(dc, injector_handle)`。
- `on_data_channel` 收到帧：
  1. `String::from_utf8` 失败 → `tracing::warn!("non-utf8 input frame")` 跳过（补上 BACKLOG 的静默丢弃日志项）。
  2. `serde_json::from_str::<InputEvent>` 失败 → `tracing::warn!("bad input event: {e}")` 跳过。
  3. 成功 → `injector.send(ev)`。
- `InputInjector` 在 `PeerSession::new` 时 `start()`，随 `PeerSession` 生命周期存活；`PeerSession` 释放时注入器 `Drop`，worker 线程收到通道关闭而退出。
- data channel 名从 `"echo"` 改为 `"input"`（两端同步）；Agent 侧不校验 label（`on_data_channel` 接受任意 channel），改名只影响 web 侧 `createDataChannel`。

### 2.5 ICE 缓冲前置保险（Rust 端，BACKLOG 项 A）

- `agent/src/webrtc_peer.rs`：`add_remote_ice` 目前直接 `pc.add_ice_candidate`，若远端 candidate 早于 offer 的 remote description 到达会报错丢弃。
- 改为 `PeerSession` 内维护：`remote_desc_set: 标志` + `pending: Mutex<Vec<RTCIceCandidateInit>>`。
  - `add_remote_ice`：若 remote desc 未 set → 入 `pending`；否则直接 add。
  - `accept_offer`：`set_remote_description` 成功后，置 `remote_desc_set` 并排空 `pending` 逐个 add，再继续原有 answer 流程。
- 独立于键鼠功能，先做，作为真实输入流量前的保险。

### 2.6 控制端捕获面板（Web 端）

- **它做什么：** 提供占位「远程屏幕」，捕获鼠标/键盘并编码为 `InputEvent` 经 data channel 发送；回显最近事件。
- **接口：** `Session.sendInput(ev: InputEvent)`（`rtc.ts` 新增）；纯编码辅助 `mouseCoords/mouseButtonName`。
- `packages/web/src/pages/SessionView.tsx`：把回显聊天 UI 换成——
  - 一个带边框、`tabIndex=0`、可聚焦的「远程屏幕」占位 `div`（`ref` 取 `getBoundingClientRect`）。未连接时禁用捕获并提示。
  - 捕获（仅在面板聚焦/悬停时生效）：
    - `onMouseMove`：`x=(clientX-rect.left)/rect.width`、`y=(clientY-rect.top)/rect.height`，clamp `[0,1]`，**rAF 合帧节流**（每动画帧只发最新位置）→ `mmove`。
    - `onMouseDown`/`onMouseUp`：`e.button` 映射 `0→left,1→middle,2→right` → `mdown`/`mup`；`onContextMenu` `preventDefault` 屏蔽右键菜单。
    - `onWheel`：`dx=e.deltaX,dy=e.deltaY` → `wheel`；`preventDefault` 防页面滚动。
    - `onKeyDown`/`onKeyUp`（面板聚焦时）：`e.code` → `kdown`/`kup`；`preventDefault` 拦浏览器快捷键；`Escape` 释放焦点（`blur`）作为「脱离捕获」的出口。
  - **事件回显面板：** 滚动展示最近 N 条（如 20）已发事件的简短文本。fire-and-forget，Agent 不回 ack。
- `packages/web/src/rtc.ts`：
  - `createDataChannel("echo")` → `createDataChannel("input")`。
  - 移除/停用 `onEcho` 文本回显路径（Plan 3 取代它）；新增 `Session.sendInput(ev)`，`channel.readyState==="open"` 时 `channel.send(JSON.stringify(ev))`。
  - 纯辅助 `mouseCoords(clientX,clientY,rect)→{x,y}`、`mouseButtonName(button)→MouseButton|null`，产物用 `@rd/protocol` 的 `parseInputEvent` 校验。
  - 保留既有纯函数（`deriveWsUrl/buildConnect/buildOffer/buildIce`）与其测试不变。

## 3. 数据流（一次注入）

```
web 面板聚焦
  鼠标/键盘事件 → 编码 InputEvent（相对坐标/按钮/code）
    ├─ rAF 节流(mmove) → channel.send(JSON)  ──(data channel "input")──►  Agent
    └─ 事件回显面板追加一行
Agent on_data_channel:
  帧 → UTF-8 → serde_json::InputEvent → injector.send(ev)   （失败则 warn 跳过）
InputInjector worker 线程:
  recv(ev) → enigo 注入（mmove 映射到主屏像素 / 按钮 / 滚轮 / key）
本机系统: 真实光标移动 / 点击 / 滚动 / 字符输入
```

## 4. 错误处理与边界

- 非 UTF-8 帧 / JSON 解析失败 / 未知 `code` → `tracing::warn!` 跳过，不崩。
- `mmove` 坐标越界 → clamp 到 `[0,1]` 并 warn（不拒绝，容忍浮点边界误差）。
- `Enigo::new` 失败（Wayland/权限）→ worker 打印指引、排空丢弃，会话仍连通。
- macOS 未授权 → 启动检测给指引；用户授权后需重启 Agent 生效（macOS 权限模型限制，文档说明）。
- data channel 关闭 / 会话结束 → `PeerSession` 释放，注入器线程随通道关闭退出。
- 无视频阶段：占位面板坐标基于面板尺寸而非真实远端分辨率——由 Agent 侧按本地主屏还原，语义正确；Plan 4 接入视频后面板换成 `<video>`，捕获逻辑不变。

## 5. 测试策略（对齐总体设计 §8）

- **协议层（两端一致性红线）：** Rust `input.rs` 对 6 个变体做 serde round-trip 单测，并断言与 TS 线格式一致（字段名、变体标签、按钮枚举）。对照 `packages/protocol/test/input.test.ts` 的用例。
- **Agent 单元：**
  - `code_to_key` 纯函数：字母/数字/符号/功能键/方向键/修饰键/编辑键/未知 code 的映射。
  - 坐标映射纯函数 `map_coord(x,y,w,h)→(px,py)`：边界 0/1、越界 clamp、典型值。
  - `mouseButton` 映射。
  - 真实注入（需真实显示器/权限）：`#[ignore]` 集成测试，比照总体设计「本机集成、CI 条件运行」。
- **Agent 集成：** ICE 缓冲——candidate 早于 remote desc 到达时进缓冲、`accept_offer` 后被 flush（可在现有 webrtc 测试风格下加一个用例）。
- **Web 单元（vitest）：** `mouseCoords`/`mouseButtonName`/`sendInput` 编码产物用 `parseInputEvent` 校验；rAF 节流的合帧逻辑（抽成纯函数或用 fake timer）。
- **回归：** 保持 Node `npm test`、`npm run typecheck` 全绿；Agent `cargo test` 既有 10 个不破。web 回显路径变更——当前无 SessionView 组件测试，故不涉及既有用例回归（新增 rtc 编码测试）。
- **端到端冒烟：** 更新 `docs/superpowers/plan2b-e2e-smoke.md`（或新增 `plan3-input-smoke.md`）：起服务端+coturn+Agent+web，在面板上移动/点击/打字，观察被控端真实光标与输入 + Agent 日志。

## 6. 任务拆分与并行

目录互不重叠 → 并行组（子代理并发派发，控制端逐个分别提交避免 git 抢锁）：

- **Task A（`agent/`，单子代理顺序做）：** ICE 缓冲（§2.5）→ 协议镜像（§2.1）→ 注入器 + 键位表（§2.2）→ 权限检测（§2.3）→ 接入 PeerSession + 改名 channel（§2.4）→ 相关单测。同一目录内串行，避免文件级抢锁。
- **Task B（`packages/web/`，另一子代理）：** rtc `sendInput` + 编码辅助 + 改名 channel（§2.6）→ SessionView 捕获面板 + 事件回显 → rtc 编码单测。
- **Task C（`docs/`，控制端收尾）：** e2e 冒烟文档更新。

A 与 B 唯一契约：channel 名 `"input"` + 已固定的 `InputEvent` JSON 线格式（`@rd/protocol` 为准）。每任务独立审查 + 整分支终审。

## 7. 新增依赖

- Agent：`enigo`（键鼠注入）；macOS 下 `macos-accessibility-client`（或等价，仅 `cfg(macos)`）用于权限检测。
- Web：无新增运行时依赖（原生 DOM 事件 + 既有 `@rd/protocol`）。
