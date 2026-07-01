# Plan 6 · Agent reliability + session lifecycle · 设计文档

- 日期：2026-07-02
- 状态：设计已确认（目标驱动；设计由总体设计 §7 明确，无开放分叉），待生成实现计划
- 范围：让 macOS 远程访问在真实网络下**一定可用**——agent 断线自动重连、任一端掉线时另一端被通知并干净收场（不再永久卡死/离线）。
- 上游：Plans 1–5 已合并。依赖 `agent/src/signaling.rs`、`agent/src/webrtc_peer.rs`（PeerSession + Plan 5 的注入器 release-on-drop）、`packages/server/src/signaling/{hub,registry}.ts`、`packages/web/src/rtc.ts`（已处理 `peer-left`）。

## 1. 目标

消除三个使真实使用不可靠的缺口（总体设计 §7「断线重连」「Agent 离线」要求）：

1. **Agent 自动重连**：signaling WS 断开后，agent 以指数退避重连并重新上线，而不是进程退出、设备永久离线。
2. **掉线通知对端**：任一端 WS 关闭时，服务端通知同会话的另一端 `peer-left`，让其停止等待并收场（web 已处理 `peer-left`；agent 需新增处理）。
3. **Agent 处理 `peer-left`**：收到后释放当前 `PeerSession`（触发 Plan 5 的注入器 release-on-drop，卡键也一并解决）。

**成功判据：**
- 拔网/服务端重启后，agent 在退避时间内自动重新上线，设备列表恢复在线，可再次发起会话。
- web 标签关闭/掉线 → agent 端会话释放（日志可见），被控端不残留输入。
- agent 掉线 → web 收到 `peer-left`，UI 从「已连接」转为「断开」，不再永久转圈。
- 永久性错误（device token 失效 = `bad-token`）→ agent 停止重连并提示重新配对（不无限空转打服务端）。

**非目标（YAGNI）：** 会话中 WebRTC 媒体断线的自动重连（设计 §7 明确 MVP 先手动重连）；服务端持久化在线状态；心跳/keepalive 调优。

## 2. 组件与改动

### 2.1 Agent 重连循环（`agent/src/signaling.rs` + `main.rs`）

- 把现有「连接 + 上线 + select 循环」抽成 `async fn run_session(config, ...) -> SessionOutcome`，负责单条连接的生命周期；连接关闭/出错即返回。
- 新增 `async fn run_agent(config)`（保留同名入口，`main` 不变）循环调用 `run_session`：
  - 正常关闭 / 连接错误 → 记日志，`sleep(backoff)`，退避后重连；重连成功后（连接稳定存活 ≥ `STABLE_RESET` 秒）把退避重置到基值。
  - 收到 `Error { code: "bad-token" }` 或上线被服务端拒绝 → 返回**致命**结果，跳出重连循环并提示重新配对（config 失效，重试无意义）。
- 退避策略（纯函数 `next_backoff(current: Duration) -> Duration`，可单测）：基值 1s，×2，上限 30s。加小抖动可选（先不加，保持可测）。
- `SessionOutcome` 枚举区分 `Retry`（可重连）与 `Fatal`（停止）。

### 2.2 Agent 处理 `peer-left`（`agent/src/signaling.rs`）

- 在 `run_session` 的 match 增加：
  ```rust
  SignalingMessage::PeerLeft { session_id } => {
      if let Some((sid, peer)) = &current {
          if *sid == session_id { let _ = peer.close().await; current = None;
              tracing::info!("peer left session {session_id}; released"); }
      }
  }
  ```
- `PeerSession` drop/close → 其内 `InputInjector` drop → Plan 5 的 release-on-close 释放已按下键（卡键随会话结束自动清）。

### 2.3 服务端掉线通知对端（`packages/server/src/signaling/{registry,hub}.ts`）

- 现状：`ws.on("close", () => registry.remove(conn))` 删除会话但**不通知对端**。
- 改：`Registry.remove(conn)` 返回受影响的对端列表 `{ sessionId, peer }[]`（该 conn 参与的每个会话的另一端），删除会话/agent 映射的逻辑不变。
- `hub.ts` 的 close 处理器：`const left = registry.remove(conn); for (const { sessionId, peer } of left) peer.send(JSON.stringify({ type: "peer-left", sessionId }));`
- 效果：web 掉线 → agent 收 `peer-left`（§2.2 释放会话）；agent 掉线 → web 收 `peer-left`（rtc.ts 已 `close()`）。

### 2.4 Web（`packages/web/src/rtc.ts`）——已就绪，微调可选

- 已处理 `peer-left`（`close()`）。可选：把 `peer-left` 关闭的状态文案区分为「对端断开」而非泛化 `closed`（非必需，先不做以免扩张范围）。

## 3. 错误处理与边界

- 重连风暴：退避上限 30s；`bad-token` 致命停止，避免无效重试。
- 重连时若有进行中的 `PeerSession`：连接关闭即 `run_session` 返回，`current` 随之释放（会话作废，媒体断，符合 §7「会话中断线先手动重连」）。
- 服务端 close 通知：仅通知**同会话**的对端；agent 掉线不影响其它设备/会话。
- 幂等：`peer-left` 与正常 `peer-left` 消息路径都调用同一收场逻辑；重复收到无害（`current` 已 None）。

## 4. 测试策略

- **Agent 纯单测：** `next_backoff`（1→2→4→…→30 cap；重置到基值）。`SessionOutcome` 分类（bad-token→Fatal，普通关闭→Retry）——把「从一次 run_session 的结束原因映射到 outcome」抽成可测点。
- **Agent 集成：** `peer-left` 释放会话——signaling 循环较难无服务器单测；核心收场逻辑（收到 PeerLeft → drop current）由代码审查 + 保持既有 echo/ice/input loopback 测试绿覆盖回归。（与既有 signaling 分支同为 inspection 级，符合 BACKLOG 既有约定。）
- **服务端集成（可测，重点）：** 复用 `packages/server/test/signaling.test.ts` 的 agent+web 模拟：建会话 → 关闭一端 WS → 断言另一端收到 `{type:"peer-left",sessionId}`。两个方向各一个用例。
- **回归：** agent `cargo test`（37+2 ignored 增长）、`cargo clippy` 干净；Node `npm test`（64 增长）、typecheck、web build 全绿。

## 5. 任务拆分与并行

- **Server 轨（`packages/server/`）：** `Registry.remove` 返回受影响对端 + hub close 发 `peer-left` + 两个集成测试。
- **Agent 轨（`agent/`）：** 重连循环 + `next_backoff`/`SessionOutcome` 单测 + `peer-left` 处理。
- 两轨目录不相交 → 可并行；各自独立审查 + 整分支终审。（web 无需改。）

## 6. 新增依赖

无。agent 用既有 `tokio`（`tokio::time::sleep`）；server 用既有 `ws`。
