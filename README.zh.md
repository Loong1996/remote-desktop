# 远程桌面

[English](./README.md) · [中文](./README.zh.md)

一个从**浏览器**操控的跨平台远程桌面——控制端**免安装**。打开网页、登录、点一台在线设备,就能开始操控。被控机器上跑一个小巧的原生 **Rust agent**。画面和输入通过 **WebRTC 点对点(P2P)** 传输;服务器只负责牵线(信令),**不经手你的媒体流**。

因为控制端就是一个网页,所以哪里有现代浏览器就能用(现在是桌面,将来手机/iOS 也无需额外 App)。原生控制端可以后续再加。

## 功能

- **实时屏幕流 + 键鼠控制**,WebRTC 低延迟。
- **账号 + 设备列表**——注册、登录、看到自己的设备,点在线的那台即可连接。
- **剪贴板同步**——关闭 / 单向 / 双向,每会话可选(默认关闭,保护隐私)。
- **实时画质调节**——码率滑杆 + 预设(流畅/均衡/高清/超清/原画),**不断线**即时生效。
- **分辨率热切换**——720p / 显示器逻辑分辨率 / 视网膜,实时切换(画面停顿约 0.5 秒,会话不断)。
- **特殊组合键**——复制、粘贴、Spotlight、应用切换、调度中心、截图,以正确的组合键下发。
- **连接统计浮层**——帧率、码率、RTT、分辨率。
- **全屏 / 填充窗口** 观看。
- **macOS 硬件编码**(VideoToolbox H.264),各平台均有 openh264 软件回退。
- **P2P 传输** 带 NAT 穿透(coturn 提供 STUN/TURN)。中继策略每会话可配:`direct-only` | `relay-fallback`(默认)| `force-relay`。

尚未实现:音频、文件传输、多显示器、Windows/Linux 硬件编码、VP9/AV1 编解码。

## 架构

```
┌──────────────────┐        ┌────────────────────────┐        ┌──────────────────┐
│  React 网页(控制) │◄──────►│    Node/TS 服务器       │◄──────►│   Rust agent      │
│  浏览器           │  HTTPS │  · REST(账号/设备)     │   WS   │  (mac / Win / …)  │
│                  │  + WS  │  · WebSocket 信令       │        │                  │
└────────┬─────────┘        └────────────────────────┘        └────────┬─────────┘
         │                                                              │
         │        WebRTC(优先 P2P,失败回退 TURN 中继)                 │
         │   • 视频 track:H.264(VideoToolbox / openh264)             │
         │   • "input" 数据通道:鼠标/键盘事件                          │
         │   • "control" 数据通道:剪贴板 · 画质 · 分辨率               │
         └──────────────────────────────────────────────────────────────┘
                                     │
                              ┌──────┴───────┐
                              │  coturn       │  STUN/TURN
                              └──────────────┘
```

信令(SDP/ICE 交换)走服务器;一旦连上,画面 + 输入就是点对点。完整设计文档见 [docs/superpowers/specs/](docs/superpowers/specs/),实现计划见 [docs/superpowers/plans/](docs/superpowers/plans/)。

## 仓库结构(monorepo)

```
packages/
  protocol/   # @rd/protocol —— 共享 TS 线格式类型 + 运行时校验
              #   信令、输入事件、控制消息(剪贴板/画质/分辨率)
  server/     # @rd/server —— Fastify(REST)+ ws(信令)+ better-sqlite3
              #   账号/JWT、设备列表/配对、WebSocket SDP/ICE 转发
  web/        # @rd/web —— React(Vite)控制端:登录、设备列表、会话视图
agent/        # rd-agent —— Rust:抓屏、H.264 编码、输入注入、WebRTC answerer
infra/coturn/ # coturn(STUN/TURN)docker-compose + 配置
docs/         # 设计规格 + 实现计划
```

## 技术栈

- **协议 / 服务器 / 网页:** TypeScript(Node ≥ 20)。Fastify、`ws`、better-sqlite3、bcryptjs、jsonwebtoken;React + Vite。测试用 Vitest。
- **Agent:** Rust(edition 2021)。`webrtc-rs`、ScreenCaptureKit(macOS)/ `xcap`(Windows)抓屏、`openh264` + VideoToolbox H.264 编码、`enigo` 输入注入。
- **传输 / NAT:** WebRTC + coturn。

## 快速开始

前置:**Node.js ≥ 20** 和 **Rust**(stable,用 rustup)。coturn 只在无法直连(公网)时需要;局域网无需它。

```bash
npm install          # 安装 workspace 依赖(protocol、server、web)
npm test             # 所有 TS 测试(Vitest)
npm run typecheck    # 类型检查 protocol + server(web 通过其 build 检查)
npm run -w @rd/web build   # 类型检查 + 构建网页控制端
```

Agent(Rust):

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test  --manifest-path agent/Cargo.toml
cargo build --release --manifest-path agent/Cargo.toml
```

### 运行(三个进程)

1. **服务器** —— 用 5181 端口,网页客户端会默认找它:

   ```bash
   JWT_SECRET=change-me PORT=5181 npm run dev -w @rd/server
   ```

2. **网页控制端:**

   ```bash
   npm run dev -w @rd/web        # Vite 开发服务器,http://localhost:5173
   ```

   打开网页,**注册账号**并登录。(网页默认请求 `<页面主机>:5181` 的服务器;可用 `VITE_SERVER_URL` 覆盖。)

3. **Agent** 跑在被控机器上。首次运行会把它配对到你的账号:

   ```bash
   RD_SERVER_URL=http://127.0.0.1:5181 \
     ./agent/target/release/rd-agent          # 提示输入邮箱/密码,保存配置
   ```

   配对后,该设备会在网页设备列表里显示**在线**——点 **Connect** 即可。

   > **macOS:** 给 agent 所在终端授予 **屏幕录制** 和 **辅助功能**
   > (系统设置 → 隐私与安全性)。否则 agent 仍能连上,但画面空白 / 无法注入输入(会打警告日志)。

## 配置

**服务器**(环境变量):

| 变量 | 默认 | 含义 |
|-----|------|------|
| `PORT` | `8080` | HTTP/WS 端口(用 `5181` 以匹配网页默认) |
| `JWT_SECRET` | *(测试外必填)* | JWT 签名密钥(HS256) |
| `RELAY_POLICY` | `relay-fallback` | `direct-only` \| `relay-fallback` \| `force-relay` |
| `ICE_SERVERS` | Google STUN | ICE 服务器 JSON 数组 |
| `DB_PATH` | `remote-desktop.db` | SQLite 文件路径 |

**Agent**(环境变量):

| 变量 | 默认 | 含义 |
|-----|------|------|
| `RD_AGENT_CONFIG` | 平台配置目录 | JSON 配置路径(`server_url`、`device_id`、`device_token`) |
| `RD_SERVER_URL` | `http://127.0.0.1:8080` | 首次配对使用的服务器 |
| `RD_VIDEO_SOURCE` | `screen` | `screen`(真实抓屏)或 `testpattern`(合成画面,免权限) |
| `RD_VIDEO_ENCODER` | *(自动)* | 设为 `openh264` 强制软件编码(跳过 VideoToolbox) |
| `RUST_LOG` | — | 日志过滤,如 `info` |

**网页**(构建期环境变量):`VITE_SERVER_URL`(完整服务器 URL,优先)或 `VITE_SERVER_PORT`(默认 `5181`)。

## 安全

**未针对公网暴露做加固。** 认证只是基础的邮箱/密码 + JWT,没有二次验证、限流或设备审批。请在可信局域网或 VPN 后运行。若要端口转发到公网,请用强密码,并在用完后关闭转发。

## 路线图

| 里程碑 | 状态 |
|-------|------|
| 共享协议(TS)+ Node 服务器(账号、设备、信令) | ✅ |
| WebRTC 媒体 + Rust agent + React 网页端(端到端连通) | ✅ |
| 键鼠注入(物理键位映射) | ✅ |
| 抓屏 + H.264 视频(openh264) | ✅ |
| 剪贴板同步 · 实时画质/码率 · 分辨率热切换 · 组合键 · 统计 | ✅ |
| macOS 硬件编码(VideoToolbox)+ 实时帧节奏 + PLI 关键帧 | ✅ |
| Windows/Linux 硬件编码(NVENC/QSV/AMF) | ⏳ |
| VP9/AV1 编解码 + 编解码器协商 | ⏳ |
| 音频 · 文件传输 · 多显示器 | ⏳ |

## 许可

待定。
