# 远程桌面

[English](./README.md) · [中文](./README.zh.md)

一套跨互联网控制电脑的跨平台远程桌面系统。控制端当前用浏览器（全平台可用，也覆盖未来的 iOS），后续可扩展原生客户端；被控端运行原生 Agent。画面与键鼠通过 WebRTC 点对点直传，信令服务器只负责牵线建连。

> **状态：** Plan 1（共享协议 + Node 信令服务端）已完成并合并。WebRTC 媒体、Rust Agent、React 控制端进行中（Plan 2 起）。

## 功能

- **MVP 范围：** 实时画面传输 + 鼠标/键盘控制（逐步叠加）。
- **账号 + 设备列表：** 登录后查看自己的设备，点选在线设备发起连接。
- **P2P 传输：** WebRTC，支持 NAT 穿透（STUN/TURN，走 coturn）。媒体流不经过服务器。
- **可配置中继策略：** `direct-only`｜`relay-fallback`（默认）｜`force-relay`，按会话下发。
- **跨平台 Agent：** Windows / macOS / Linux（Rust）。

后续迭代：多显示器、声音、文件传输、剪贴板同步、多控制端会话。

## 架构

```
┌─────────────────┐        ┌──────────────────────┐        ┌─────────────────┐
│  React 控制端     │◄──────►│   Node/TS 服务端        │◄──────►│  Rust Agent 被控端 │
│  浏览器           │  HTTPS │  · REST(账号/设备)      │   WS   │  (Win/mac/Linux) │
│                 │  + WS  │  · WebSocket 信令中转     │        │                 │
└────────┬────────┘        └──────────────────────┘        └────────┬────────┘
         │                                                          │
         │           WebRTC (P2P 优先，穿透失败走 TURN 中继)          │
         └──────────── 画面(video track) + 键鼠(data channel) ───────┘
                                     │
                              ┌──────┴───────┐
                              │  coturn       │  STUN/TURN
                              └──────────────┘
```

信令走服务端，媒体走 P2P。完整设计见 [docs/superpowers/specs/](docs/superpowers/specs/)。

## 单仓结构

```
packages/
  protocol/   # @rd/protocol —— 共享 TS 类型 + 运行时校验
              #   信令消息、输入事件（语言无关的 JSON 协议）
  server/     # @rd/server —— Fastify(REST) + ws(信令) + better-sqlite3
              #   账号/JWT、设备列表/配对、WebSocket SDP/ICE 转发
docs/         # 设计文档 + 实现计划
```

规划中：`packages/web`（React 控制端）、`agent/`（Rust Agent）、`infra/`（coturn）。

## 技术栈

- **协议 / 服务端 / 前端：** TypeScript（Node ≥20）。Fastify、`ws`、better-sqlite3、bcryptjs、jsonwebtoken。React（控制端）。
- **Agent：** Rust（`webrtc-rs`、抓屏 + H.264、`enigo` 注入键鼠）。
- **传输 / 穿透：** WebRTC + coturn。
- **测试：** vitest。

## 快速开始

需要 Node.js ≥ 20。

```bash
npm install          # 安装 workspace 依赖
npm test             # 跑全部测试 (vitest)
npm run typecheck    # 严格类型检查（所有包）
```

启动服务端（开发）：

```bash
# 非测试环境必须提供 JWT_SECRET
JWT_SECRET=change-me npm run dev -w @rd/server
```

服务端配置（环境变量）：

| 变量 | 默认值 | 含义 |
|-----|--------|------|
| `PORT` | `8080` | HTTP/WS 端口 |
| `JWT_SECRET` | *(生产必填)* | JWT 签名密钥（HS256） |
| `RELAY_POLICY` | `relay-fallback` | `direct-only`｜`relay-fallback`｜`force-relay` |
| `ICE_SERVERS` | Google STUN | ICE 服务器 JSON 数组 |
| `DB_PATH` | `remote-desktop.db` | SQLite 文件路径 |

## 路线图

| 计划 | 里程碑 | 状态 |
|------|--------|------|
| 1 | 共享协议(TS) + Node 服务端（账号、设备、信令） | ✅ 完成 |
| 2 | WebRTC 空连接（最小 React 控制端 + Rust Agent + coturn），data channel 回显 | 🚧 进行中 |
| 3 | 鼠标/键盘注入 | ⏳ |
| 4 | 抓屏 + H.264 视频 | ⏳ |

## 许可证

待定。
