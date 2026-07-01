# 远程桌面系统 · 设计文档

- 日期：2026-07-01
- 状态：设计已确认，待生成实现计划
- 范围：MVP（画面传输 + 键鼠控制），后续迭代叠加其它能力

## 1. 目标与场景

跨互联网的远程桌面控制系统。被控端可能有公网 IP（直连），也可能在 NAT 后（需穿透/中继）。控制端第一版用浏览器（Web），后续可扩展为原生客户端（桌面 + iOS），形成混合形态。

**MVP 功能范围：**
1. 画面传输：被控端桌面实时显示到控制端。
2. 鼠标 + 键盘控制：控制端操作被控端。

后续迭代（不在本 MVP）：多显示器、声音、文件传输、剪贴板同步、多控制端会话。

## 2. 技术选型

| 组件 | 选型 | 说明 |
|------|------|------|
| 传输层 | **WebRTC** | NAT 穿透(ICE/STUN/TURN)、浏览器原生、原生客户端有成熟库 |
| 被控端 Agent | **Rust** | `webrtc-rs`、抓屏/编码、`enigo` 键鼠注入，跨平台、单文件分发 |
| 服务端 | **Node.js / TypeScript** | REST + WebSocket 信令，业务开发快，与前端同语言 |
| Web 控制端 | **React** | 生态大、WebRTC/Canvas 示例多 |
| NAT 穿透 | **coturn** | 现成开源 STUN/TURN |
| 存储 | **SQLite**（MVP） | 零运维，后续可换 Postgres |
| 视频编码 | **H.264**（一步到位） | 抓屏 → H.264 编码 → WebRTC video track，不走截图轮询简化路线 |

**配对/寻址模型：** 账号登录 + 设备列表。两端登录同一账号，控制端从设备列表点选在线设备发起连接。账号体系 MVP 做极简：邮箱/用户名 + 密码 + JWT。

## 3. 架构总览

```
┌─────────────────┐        ┌──────────────────────┐        ┌─────────────────┐
│  React Web 控制端 │◄──────►│   Node/TS 服务端        │◄──────►│  Rust Agent 被控端 │
│  (浏览器)         │  HTTPS │  · REST API (账号/设备)  │   WS   │  (Win/mac/Linux) │
│                 │  + WS  │  · WebSocket 信令中转     │        │                 │
└────────┬────────┘        └──────────────────────┘        └────────┬────────┘
         │                                                          │
         │              WebRTC (P2P 优先，穿透失败走 TURN)             │
         └──────────────── 画面(video track) + 键鼠(data channel) ────┘
                                     │
                              ┌──────┴───────┐
                              │  coturn       │  STUN/TURN
                              └──────────────┘
```

**核心原则：信令走服务端，媒体走 P2P。** 服务端只在 WebRTC 握手阶段牵线，之后画面与键鼠均端到端直传，不经过服务端——省带宽、低延迟。

## 4. 组件内部结构

### 4.1 Rust Agent（被控端）
- `auth`：首次运行输入账号邮箱+密码（或 Web 端生成的一次性配对码），换取长期 **device token** 存本地，之后据此连服务端。
- `signaling`：维护到服务端的长连 WebSocket，被动接收连接请求，收发 SDP / ICE。
- `capture`：跨平台抓屏（`scrap`/`xcap`）→ H.264 编码（`openh264` 等）→ WebRTC sample。**全项目最高风险项**，MVP 目标为稳定出流；画质/码率自适应留后续。
- `input`：`enigo` 注入键鼠，含坐标映射（相对坐标 → 本地分辨率）。
- `webrtc`：`webrtc-rs`，串起 video track（出）+ data channel（入）。

### 4.2 Node/TS 服务端
- `api`（REST，Express/Fastify）：`/register`、`/login`(签发 JWT)、`/devices`(列出设备+在线状态)、`/devices/pair`(生成配对码)。
- `signaling`（WebSocket）：Agent 与 Web 各连一条；维护在线表；纯转发 SDP/ICE，不解析媒体。
- `store`：SQLite（`better-sqlite3` 或 Prisma），存用户、设备、token。
- `turn-config`：下发 coturn 地址与临时凭证及中继策略。

### 4.3 React Web 控制端
- 页面：`LoginPage`、`DeviceListPage`（在线状态更新）、`SessionView`（`<video>` + 键鼠捕获）。
- `rtc/`：封装 WebRTC 连接与 data channel 输入编码，对齐 Rust 端输入协议。

### 4.4 共享协议
信令消息（JSON）与输入事件格式写成独立协议定义文档，Rust 端与 TS 端各自实现，保证未来原生客户端照此接入。

## 5. 数据流（一次会话时序）

```
① 注册/登录
   Web ──POST /login──► 服务端 ──签发 JWT──► Web
   Agent 启动 ──device token 连 WS──► 服务端  (标记"在线")

② 选设备发起
   Web ──GET /devices──► 服务端 ──► 设备列表(含在线状态)
   用户点在线设备 ──WS {type:"connect", deviceId}──► 服务端
   服务端 ──WS {type:"incoming", sessionId}──► 目标 Agent

③ WebRTC 握手 (服务端只转发)
   Web  创建 offer  ──WS {sdp:offer}──► 服务端 ──► Agent
   Agent 创建 answer ──WS {sdp:answer}──► 服务端 ──► Web
   双方 ──WS {ice:candidate}──► 服务端 ──► 对端 (来回若干次)

④ 通道建立 (P2P，绕过服务端)
   ICE 成功 → 直连；失败按策略走 coturn TURN 中继
   建立 video track (Agent→Web) + data channel (Web→Agent)

⑤ 实时运行
   Agent: 抓屏→H.264→video track  ──► Web: <video> 渲染
   Web: 键鼠→JSON事件→data channel ──► Agent: enigo 注入

⑥ 结束
   任一端关闭 → 通知对端 → 释放资源；Agent 回"在线待命"
```

**要点：**
- 在线状态靠 Agent 长连 WS 的存活判定，断线即离线。
- `sessionId` 贯穿一次会话，便于日志追踪与未来多控制端。
- 输入事件为结构化 JSON（`{type:"mouse"|"key", ...}`），坐标用 0~1 相对值传输，Agent 按本地分辨率还原。

## 6. 中继策略（可配置）

三档策略，随会话在信令阶段由服务端下发 `{ relayPolicy, iceServers }`：

- `direct-only`：仅 P2P 直连（不下发 TURN，`iceTransportPolicy` 只用直连 candidate）。适合有公网 IP、省中继带宽。打不通即失败提示。
- `relay-fallback`（默认）：优先直连，打洞失败自动降级 TURN。通用性最好。
- `force-relay`：强制 TURN 中继（`iceTransportPolicy:"relay"`）。适合隐藏真实 IP 或强限制网络。

**配置粒度：** 全局默认在服务端配；同时允许每台设备/每次连接覆盖（Web 端发起时可指定，服务端决定是否下发 TURN 凭证）。

## 7. 错误处理与边界情况

- **Agent 离线**：列表显示离线，禁止发起；发起中掉线 → `error:offline`。
- **打洞失败**：`direct-only` 报"无法直连"；`relay-fallback` 转 TURN；TURN 也失败 → 明确报错+可能原因。
- **认证失败/过期**：device token 失效 → 重新配对；JWT 过期 → 跳登录页。
- **信令超时**：SDP/ICE 交换设超时，卡住则中止并提示。
- **断线重连**：Agent↔服务端 WS 自动重连（指数退避）；会话中 WebRTC 断线 MVP 先提示手动重连，自动重连留后续。
- **输入注入权限**：macOS 需辅助功能授权、Linux/Wayland 有注入/抓屏限制——首次运行检测并给授权指引。
- **分辨率/多屏变化**：MVP 单主屏；分辨率变化时重协商 video track 尺寸。

## 8. 测试策略

- **协议层（共享）**：信令消息、输入事件编解码单元测试，Rust 与 TS 各一套，保证两端对齐同一协议——防止两端各写各的的关键测试。
- **Node 服务端**：单元（JWT、在线表、配对码）+ 集成（模拟 Agent+Web 验证信令转发 offer→answer→ice 走通）；存储用内存 SQLite。
- **Rust Agent**：单元（坐标映射、输入解析、中继策略选择）；`capture`/`input` 涉真实系统调用，本机集成测试（CI 条件运行）。
- **React 控制端**：组件测试（登录、设备列表、在线状态）+ `rtc/` 输入编码单元测试。
- **端到端冒烟**：脚本本机同起服务端+Agent+无头浏览器控制端，验证"能连、有画面、键鼠有反应"，作为每次大改动验收基线。

## 9. 开发顺序（降风险路径）

1. 定义共享协议（信令 + 输入事件）。
2. Node 服务端：账号 + 设备列表 + WebSocket 信令转发。
3. Web↔Agent 的 WebRTC 空连接（data channel 收发假消息跑通）。
4. `input` 键鼠注入接入。
5. 攻 `capture` 视频编码（最硬骨头），实现实时画面。

**纪律：** 每一步做完必须严格测试通过后再提交 git。所有设计/进度/规划文件维护在工程目录内。
