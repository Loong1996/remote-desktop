# Plan 2b: React 控制端 + coturn + Agent ICE 双向 + 端到端回显 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan. Steps use checkbox (`- [ ]`) syntax. **Subagents run on Opus 4.8. Tasks in the same "并行组" touch disjoint files and MUST be dispatched concurrently.**

**Goal:** 完成"WebRTC 空连接打通"里程碑的另一半：一个最小 React 控制端（登录→设备列表→连接被控端→data channel 发消息看回显），一套可部署的 coturn，以及把 Rust Agent 补上双向 trickle ICE，最终浏览器↔Agent↔服务端全链路收到 `echo:<msg>`。

**Architecture:** 新增 `packages/web`（Vite + React + TS 控制端）与 `infra/coturn`（docker-compose）。Rust Agent 补 `ice` 信令分支 + 本地 candidate 外发（`add_remote_ice` 脚手架已在 Plan 2a 就位）。服务端加 CORS 允许 web dev origin。信令服务端不变（已能转发 sdp/ice、已做设备归属校验）。

**Tech Stack:** Vite、React 18、TypeScript、原生 WebRTC (`RTCPeerConnection`)；coturn(docker)；已有 Node 服务端加 `@fastify/cors`；Rust webrtc-rs（已在用）。

## Global Constraints

- 三个子系统目录互不重叠：`infra/`（coturn）、`agent/`（Rust ICE）、`packages/web/`（React，含对 `packages/server` 的 CORS 小改）。**Task 1/2/3 属并行组，同一条消息并发派发。**
- Rust 工具链在 `~/.cargo/bin`；cargo 命令前加 `export PATH="$HOME/.cargo/bin:$PATH"`。
- 信令 JSON 必须与 `@rd/protocol`（`packages/protocol/src/signaling.ts`）一致；web 端复用该 TS 包（`@rd/protocol`）做类型/校验，不另写一套。
- web 端连信令 WS 用 `ws://<host>?token=<jwt>`（服务端已按此认证 web 并校验设备归属）；agent 端仍用 `agent-online{token}`。
- ICE candidate 线格式：浏览器 `candidate.toJSON()` = `{candidate, sdpMid, sdpMLineIndex, usernameFragment}`；Rust `RTCIceCandidateInit`（camelCase）与之一致——直接透传即可。
- data channel 名 `"echo"`；web 创建 channel + offer（trickle），agent 回显 `echo:<msg>`。
- 每个 Task 结束必须相关测试/构建通过后再 commit。提交信息英文，附 `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`。
- 不破坏既有 39 个 Node 测试与 9 个 agent 测试。

## File Structure

```
infra/
  coturn/
    docker-compose.yml
    turnserver.conf
    README.md
agent/src/
  signaling.rs        # 改：加 Ice 分支 + 本地 candidate 外发
  webrtc_peer.rs      # 改：on_ice_candidate → 通过 mpsc 把本地 candidate 交给 signaling
packages/server/src/
  app.ts              # 改：注册 CORS（允许 web dev origin）
packages/web/
  package.json  vite.config.ts  index.html  tsconfig.json
  src/
    main.tsx  App.tsx
    api.ts            # REST：register/login/devices/pair（fetch + JWT）
    rtc.ts            # WebRTC + 信令 WS 封装（连接、offer、data channel、trickle ICE）
    pages/LoginPage.tsx  pages/DevicesPage.tsx  pages/SessionView.tsx
    api.test.ts  rtc.test.ts
docs/superpowers/
  plan2b-e2e-smoke.md # 端到端手动冒烟步骤（Task 6）
```

---

## 并行组 A（Task 1 / 2 / 3 同时派发 —— 目录互不重叠）

### Task 1: coturn infra（`infra/coturn/`）

**Files:** Create `infra/coturn/docker-compose.yml`, `infra/coturn/turnserver.conf`, `infra/coturn/README.md`

**Interfaces:** Produces 一套可 `docker compose up` 的 coturn；对外 STUN/TURN 在 `3478`（udp/tcp）；静态用户 `rduser:rdpass`、realm `remote.desktop`。服务端 `ICE_SERVERS` 与之对齐的 JSON 写进 README。

- [ ] **Step 1: 写 turnserver.conf**

```conf
listening-port=3478
fingerprint
lt-cred-mech
realm=remote.desktop
user=rduser:rdpass
# 生产请改为 use-auth-secret + static-auth-secret，并配公网 IP/证书
no-tls
no-dtls
log-file=stdout
verbose
```

- [ ] **Step 2: 写 docker-compose.yml**

```yaml
services:
  coturn:
    image: coturn/coturn:4.6
    network_mode: host
    volumes:
      - ./turnserver.conf:/etc/coturn/turnserver.conf:ro
    command: ["-c", "/etc/coturn/turnserver.conf"]
    restart: unless-stopped
```

- [ ] **Step 3: 写 README.md**（如何起、如何验证、与服务端对齐的 ICE_SERVERS）

要点：
- 启动：`docker compose -f infra/coturn/docker-compose.yml up -d`。
- 验证：`docker compose ... logs` 看到监听 3478；可选 `turnutils_uclient` 或浏览器 trickle-ice 页面测。
- 服务端对齐（本机测）：
  ```
  ICE_SERVERS='[{"urls":"stun:127.0.0.1:3478"},{"urls":"turn:127.0.0.1:3478","username":"rduser","credential":"rdpass"}]'
  ```
  说明 `network_mode: host` 便于本机 P2P；生产需公网 IP + TLS + auth-secret。
- 说明 Plan 2b 的自动化 e2e 用 STUN/host-candidate 即可在本机打通，TURN 主要为跨 NAT 真机准备。

- [ ] **Step 4: 校验 compose 语法**

Run: `docker compose -f infra/coturn/docker-compose.yml config`
Expected: 输出规范化后的配置，无错误（不要求真的 `up`，避免占端口）。

- [ ] **Step 5: Commit**

```bash
git add infra/
git commit -m "feat(infra): coturn docker-compose + config for STUN/TURN"
```

---

### Task 2: Agent 双向 trickle ICE（`agent/`）

> 背景：Plan 2a 终审 finding 1。agent 现为非 trickle（answer 内嵌 candidate），且信令循环无 `Ice` 分支、不发本地 candidate。浏览器会 trickle，故必须补双向。`PeerSession::add_remote_ice` 已存在。

**Files:** Modify `agent/src/webrtc_peer.rs`, `agent/src/signaling.rs`; Test: `agent/tests/echo_loopback.rs`（增 trickle 变体）或新增 `agent/tests/ice_trickle.rs`

**Interfaces:**
- Consumes: `PeerSession`（Plan 2a）、`SignalingMessage::Ice`（协议已有）。
- Produces:
  - `PeerSession::new` 增参：接收一个本地 candidate 外发 sink（`tokio::sync::mpsc::UnboundedSender<serde_json::Value>`），在 `on_ice_candidate` 里把 `candidate.to_json()` 序列化后发出。
  - `PeerSession::add_remote_ice(candidate: serde_json::Value)` 已存在，保持。
  - `signaling.rs run_agent`：为当前 session 建 candidate channel；收到 `Ice{session_id, candidate}` 且匹配当前 session → `add_remote_ice`；从 candidate channel 收到本地 candidate → 发 `Ice{session_id, candidate}` 给对端（经服务端转发）。

- [ ] **Step 1: 写/改测试（进程内双向 trickle 环回）**

新增 `agent/tests/ice_trickle.rs`：两个 `PeerSession`/PeerConnection，**不**等待 gathering，而是把两端 `on_ice_candidate` 产生的本地 candidate 通过测试内的 channel 互相 `add_remote_ice`，offer/answer 只交换 SDP 不含全部 candidate；最终仍断言收到 `echo:hello`。（若纯环回 trickle 不稳，可保留 Plan 2a 的 gathering 环回测试不动，另加一个"agent 至少产生并外发了 ≥1 个本地 candidate"的断言测试，验证 sink 被调用。）

> 实现者可择其一，但必须有测试证明：(a) `add_remote_ice` 能被调用且不 panic；(b) agent 会通过 sink 外发本地 candidate。

- [ ] **Step 2: 跑测试确认失败**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml --test ice_trickle`
Expected: 编译失败（`PeerSession::new` 新签名未实现）。

- [ ] **Step 3: 改 webrtc_peer.rs**

- `PeerSession::new(ice_servers, local_ice_tx: UnboundedSender<serde_json::Value>)`：
  ```rust
  pc.on_ice_candidate(Box::new(move |c| {
      let tx = local_ice_tx.clone();
      Box::pin(async move {
          if let Some(c) = c {
              if let Ok(init) = c.to_json() {
                  if let Ok(v) = serde_json::to_value(init) {
                      let _ = tx.send(v);
                  }
              }
          }
      })
  }));
  ```
- `accept_offer` 保持（可不再强制等 gathering；trickle 下应尽快返回 answer 再补 candidate。为兼容两种 web 端，保留"设置 answer 即返回"，candidate 靠 on_ice_candidate 补）。实现者按需调整，但环回/trickle 测试须过。
- `add_remote_ice` 保持。

- [ ] **Step 4: 改 signaling.rs run_agent**

- 建 `let (ice_tx, mut ice_rx) = mpsc::unbounded_channel();`，`PeerSession::new(ice_servers, ice_tx)`。
- 用 `tokio::select!` 同时处理：WS 读入的信令 + `ice_rx` 收到的本地 candidate（→ 发 `Ice{session_id, candidate}`）。
- 新增 `SignalingMessage::Ice{session_id, candidate}` 分支：匹配当前 session → `peer.add_remote_ice(candidate).await`（失败 log-and-continue）。

- [ ] **Step 5: 跑测试确认通过 + 全量**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml`
Expected: 新 ICE 测试 + 既有 9 个全过，无警告。

- [ ] **Step 6: Commit**

```bash
git add agent/
git commit -m "feat(agent): bidirectional trickle ICE (relay remote + emit local candidates)"
```

---

### Task 3: React 控制端脚手架 + 登录 + 设备列表（`packages/web/` + server CORS）

**Files:** Create `packages/web/{package.json,vite.config.ts,index.html,tsconfig.json}`, `packages/web/src/{main.tsx,App.tsx,api.ts,pages/LoginPage.tsx,pages/DevicesPage.tsx}`, `packages/web/src/api.test.ts`; Modify `packages/server/src/app.ts`（CORS）、根 `package.json`（workspace 已含 `packages/*`，无需改）

**Interfaces:**
- Produces:
  - `@rd/web` 包（Vite dev server，默认 5173）。
  - `api.ts`：`register/login(email,password)->token`、`listDevices(token)->Device[]`、`pairDevice(token,name)->{deviceId,token}`；基址 `import.meta.env.VITE_SERVER_URL ?? "http://127.0.0.1:8080"`。
  - 服务端 `app.ts` 注册 `@fastify/cors`（允许 `http://localhost:5173`）。
- Consumes: `@rd/protocol`（类型）。

- [ ] **Step 1: 服务端加 CORS（先写测试）**

在 `packages/server/test/` 加断言：带 `Origin: http://localhost:5173` 的请求响应含 `access-control-allow-origin`。实现：`npm i -w @rd/server @fastify/cors`，`app.ts` 内 `await app.register(cors, { origin: [/localhost:\d+$/] })`（注意 register 是异步，`buildApp` 若为同步需调整为返回前 await，或用 `app.register` 的同步排队特性——实现者确保 inject 测试能拿到 CORS 头）。

Run: `npm test -w @rd/server` → 新 CORS 用例 + 既有全过。

- [ ] **Step 2: web 脚手架**

`packages/web/package.json`:
```json
{
  "name": "@rd/web",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "test": "vitest run"
  },
  "dependencies": { "@rd/protocol": "*", "react": "^18.3.0", "react-dom": "^18.3.0" },
  "devDependencies": {
    "@types/react": "^18.3.0", "@types/react-dom": "^18.3.0",
    "@vitejs/plugin-react": "^4.3.0", "typescript": "^5.5.0",
    "vite": "^5.4.0", "vitest": "^2.0.0", "jsdom": "^24.0.0"
  }
}
```
`vite.config.ts`、`tsconfig.json`（jsx: react-jsx, strict）、`index.html`、`main.tsx`（挂载 App）。

- [ ] **Step 3: 写 api.ts 测试（vitest + mock fetch）**

`api.test.ts`：mock 全局 `fetch`，验证 `login` POST `/login` 带正确 body 并返回 token；`listDevices` 带 `Authorization: Bearer`；`pairDevice` POST `/devices/pair`。

- [ ] **Step 4: 实现 api.ts + LoginPage + DevicesPage + App**

- `api.ts`：fetch 封装（上述接口）。
- `LoginPage`：邮箱/密码表单，登录/注册按钮 → 存 token（内存/localStorage）→ 跳设备页。
- `DevicesPage`：拉 `listDevices`，列出 `{name, online}`；"配对新设备"按钮调用 `pairDevice` 显示 token（供 agent 用）；点在线设备 → 进 SessionView（Task 4 实现，先占位）。
- `App`：极简状态路由（未登录→Login，已登录→Devices）。

- [ ] **Step 5: 跑测试 + 构建**

Run: `npm test -w @rd/web && npm run build -w @rd/web`
Expected: api 测试通过；`vite build` 成功。

- [ ] **Step 6: Commit**

```bash
git add packages/ 
git commit -m "feat(web): react scaffold, login + device list; server CORS"
```

---

## 顺序组 B（并行组 A 完成后）

### Task 4: React WebRTC 连接 + data channel 回显（`packages/web/`）

**Files:** Create `packages/web/src/rtc.ts`, `packages/web/src/pages/SessionView.tsx`, `packages/web/src/rtc.test.ts`

**Interfaces:**
- Consumes: `@rd/protocol` 信令类型、`api` 的 token、Task 3 的 DevicesPage 入口。
- Produces:
  - `rtc.ts`：`connectSession(serverUrl, token, deviceId, onEcho, onState)`：开 WS `?token`，发 `connect{deviceId}`，收 `session-ready` → 建 `RTCPeerConnection(iceServers)`、`createDataChannel("echo")`、`createOffer/setLocalDescription`、发 `sdp{offer}`；收 `sdp{answer}` → `setRemoteDescription`；`onicecandidate` → 发 `ice`；收 `ice` → `addIceCandidate`；data channel `onopen` 后可 `send`；`onmessage` → `onEcho(text)`。
  - `SessionView`：连接某设备，提供输入框发消息，显示回显与连接状态。

- [ ] **Step 1: 写 rtc.ts 可测纯逻辑的测试**

WebRTC 在 jsdom 无原生实现，故**只单测可纯化的部分**：信令消息构造（`connect`/`sdp`/`ice` 的 JSON 形状用 `@rd/protocol` 的 `parseSignalingMessage` 反向校验）、状态机转换函数。真实 RTCPeerConnection 流程留给 Task 6 的浏览器 e2e。测试断言：给定 deviceId，`buildConnect(deviceId)` 产出 `{type:"connect",deviceId}` 且能被 `parseSignalingMessage` 接受；offer→`sdp` 消息形状正确。

- [ ] **Step 2: 跑测试确认失败** → 实现 → 通过（`npm test -w @rd/web`）。

- [ ] **Step 3: 实现 rtc.ts + SessionView**（完整 WebRTC 逻辑；把可纯化的消息构造抽成被测函数）。

- [ ] **Step 4: 构建校验**

Run: `npm test -w @rd/web && npm run build -w @rd/web`
Expected: 通过。

- [ ] **Step 5: Commit**

```bash
git add packages/web/
git commit -m "feat(web): webrtc session + data-channel echo UI"
```

---

### Task 5: 端到端手动冒烟 + 文档（`docs/`）

**Files:** Create `docs/superpowers/plan2b-e2e-smoke.md`

**Interfaces:** Produces 一份可照做的 e2e 步骤，把 server + coturn + agent + web 串起来，验证浏览器收到 `echo:<msg>`。

- [ ] **Step 1: 写 e2e 冒烟文档**

步骤（用真实命令）：
1. 起服务端（含对齐的 ICE_SERVERS）：`JWT_SECRET=dev ICE_SERVERS='[{"urls":"stun:127.0.0.1:3478"},{"urls":"turn:127.0.0.1:3478","username":"rduser","credential":"rdpass"}]' npm run dev -w @rd/server`。
2. 起 coturn：`docker compose -f infra/coturn/docker-compose.yml up -d`。
3. web 注册账号（登录页或 curl），起 web：`npm run dev -w @rd/web`，浏览器登录。
4. 设备页"配对新设备"拿 token；起 agent：`RD_SERVER_URL=http://127.0.0.1:8080 cargo run --manifest-path agent/Cargo.toml`，首次用同账号登录配对（或用配对 token）。
5. 设备页点在线设备 → SessionView 输入 "hello" → 应显示 `echo:hello`。
6. 记录预期日志（agent "agent online"、"incoming session"；web 连接状态 connected）。

- [ ] **Step 2: （可选）实际执行一次并记录结果**

若环境允许，实际跑一遍并把关键输出/截图要点记入文档；否则标注为"待真机验证"，并说明自动化受限原因（浏览器+多进程编排）。

- [ ] **Step 3: Commit**

```bash
git add docs/
git commit -m "docs: plan 2b end-to-end echo smoke guide"
```

---

## Self-Review

**Spec coverage：**
- 设计 §4.3 React 控制端（登录/设备列表/SessionView）→ Task 3、4 ✅
- 设计 §3 coturn / §6 中继 → Task 1 ✅
- 里程碑"浏览器↔agent data channel 回显"→ Task 4 + Task 5 ✅
- Plan 2a 终审 finding 1（ICE 双向）→ Task 2 ✅
- 复用 `@rd/protocol`（web 侧）→ Task 3/4 ✅
- **不在本计划**：抓屏/键鼠（Plan 3/4）、多显示器/声音/文件（后续）。

**Placeholder scan：** SessionView 在 Task 3 占位、Task 4 实现，已注明。无 TBD。

**Type consistency：** web 复用 `@rd/protocol` 类型；信令消息形状与 agent/server 同源；`Ice` 分支两端字段一致（`sessionId`/`candidate`）。

**并行安全：** Task 1(`infra/`)、Task 2(`agent/`)、Task 3(`packages/web/` + `packages/server/app.ts`) 文件集互不相交，可并发。Task 4 依赖 Task 3（同在 `packages/web/`，顺序）。Task 5 依赖全部。

## Execution Handoff

计划保存到 `docs/superpowers/plans/2026-07-01-plan2b-web-coturn-e2e.md`。执行：subagent-driven-development，子代理用 Opus 4.8，并行组 A 的 Task 1/2/3 同一消息并发派发。
