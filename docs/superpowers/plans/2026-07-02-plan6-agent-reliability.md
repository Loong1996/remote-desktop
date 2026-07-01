# Plan 6 — Agent Reliability + Session Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Make macOS remote access survive network blips and peer drops: the agent auto-reconnects its signaling WebSocket with exponential backoff, and any connection close notifies the surviving peer with `peer-left` so nothing hangs.

**Architecture:** Server — `Registry.remove` returns the peers affected by a closed connection and the hub sends them `peer-left`. Agent — the connect+loop is extracted into `run_session` returning a `SessionOutcome`; `run_agent` loops it with backoff (fatal-stop on `bad-token`), and handles inbound `peer-left` by releasing the current `PeerSession` (which, via Plan 5, releases held keys). Web already handles `peer-left` — unchanged.

**Tech Stack:** Rust (`tokio` time/sleep, existing); TypeScript/Node (`ws`, Vitest).

## Global Constraints

- Toolchains: Node ≥ 20; `cargo` in `~/.cargo/bin` — prefix cargo commands with `export PATH="$HOME/.cargo/bin:$PATH"`.
- No new dependencies.
- Backoff constants: base 1s, ×2, cap 30s; reset to base after a connection that stayed up ≥ 60s.
- Fatal (stop reconnecting) only on a `bad-token` signaling error; all other closes/errors → retry.
- `peer-left` wire message: `{ type: "peer-left", sessionId }` (matches `@rd/protocol` and the existing voluntary path).
- TDD; commit messages English ending with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- Do not break baselines: agent `cargo test` (37 + 2 ignored) grows + stays green, `cargo clippy --all-targets` clean; Node `npm test` (64) grows + stays green; `npm run typecheck` + `npm run -w @rd/web build` clean.

## File Structure

```
packages/server/src/signaling/registry.ts   # Modify: remove() returns affected peers
packages/server/src/signaling/hub.ts         # Modify: send peer-left on close
packages/server/test/signaling.test.ts       # Modify: 2 peer-left-on-close tests
agent/src/signaling.rs                        # Modify: SessionOutcome + next_backoff + run_session/run_agent + peer-left arm
docs/superpowers/plan4-video-smoke.md         # Modify: reconnect + peer-left note
```

## Parallel Groups

- **Group A (server):** Task 1, under `packages/server/`.
- **Group B (agent):** Task 2, under `agent/`. Disjoint from A → concurrent.
- **Task 3 (docs):** after A + B.

---

## Task 1: Server — notify surviving peer with peer-left on close

**Files:** Modify `packages/server/src/signaling/registry.ts`, `packages/server/src/signaling/hub.ts`, `packages/server/test/signaling.test.ts`

**Interfaces:**
- Produces: `Registry.remove(conn: Conn): { sessionId: string; peer: Conn }[]` — removes the conn's agent mapping + sessions (as before) and returns, for each session the conn was part of, the surviving peer. The hub sends each a `peer-left`.

- [ ] **Step 1: Write the failing tests**

Add to `packages/server/test/signaling.test.ts` (reuse the existing `startHub`/`openWs`/`waitMsg` helpers):

```ts
test("agent WS close notifies web with peer-left", async () => {
  const { users, devices, port, teardown } = await startHub();
  const u = users.create("a@b.com", "h");
  const dev = devices.create(u.id, "PC");
  const jwt = signToken(u.id, JWT_SECRET);

  const agent = await openWs(`ws://localhost:${port}`);
  agent.send(JSON.stringify({ type: "agent-online", token: dev.token }));
  await new Promise((r) => setTimeout(r, 50));

  const web = await openWs(`ws://localhost:${port}?token=${jwt}`);
  web.send(JSON.stringify({ type: "connect", deviceId: dev.id }));
  await waitMsg(web); // session-ready

  const left = waitMsg(web); // arm listener before closing
  agent.close();
  const msg = await left;
  expect(msg.type).toBe("peer-left");
  expect(typeof msg.sessionId).toBe("string");
  teardown();
});

test("web WS close notifies agent with peer-left", async () => {
  const { users, devices, port, teardown } = await startHub();
  const u = users.create("a@b.com", "h");
  const dev = devices.create(u.id, "PC");
  const jwt = signToken(u.id, JWT_SECRET);

  const agent = await openWs(`ws://localhost:${port}`);
  agent.send(JSON.stringify({ type: "agent-online", token: dev.token }));
  await new Promise((r) => setTimeout(r, 50));

  const web = await openWs(`ws://localhost:${port}?token=${jwt}`);
  web.send(JSON.stringify({ type: "connect", deviceId: dev.id }));
  await waitMsg(agent); // incoming
  await waitMsg(web); // session-ready

  const left = waitMsg(agent); // arm listener before closing
  web.close();
  const msg = await left;
  expect(msg.type).toBe("peer-left");
  teardown();
});
```

- [ ] **Step 2: Run to verify they fail**

Run: `npm test -- signaling`
Expected: FAIL — the surviving peer never receives `peer-left` (times out / no message).

- [ ] **Step 3: Make `Registry.remove` return affected peers**

In `packages/server/src/signaling/registry.ts`, replace `remove`:

```ts
  /** Remove a connection: drop its agent mapping and any sessions it was part
   *  of, returning the surviving peer of each removed session so callers can
   *  notify them (peer-left). */
  remove(conn: Conn): { sessionId: string; peer: Conn }[] {
    const deviceId = this.agentByConn.get(conn);
    if (deviceId) {
      this.agents.delete(deviceId);
      this.agentByConn.delete(conn);
    }
    const affected: { sessionId: string; peer: Conn }[] = [];
    for (const [sid, s] of this.sessions) {
      if (s.web === conn || s.agent === conn) {
        affected.push({ sessionId: sid, peer: s.web === conn ? s.agent : s.web });
        this.sessions.delete(sid);
      }
    }
    return affected;
  }
```

(Existing callers that ignore the return value — e.g. the `agent online/offline tracked` unit test — keep working.)

- [ ] **Step 4: Send peer-left on close in the hub**

In `packages/server/src/signaling/hub.ts`, replace the close handler:

```ts
    ws.on("close", () => {
      for (const { sessionId, peer } of registry.remove(conn)) {
        try {
          peer.send(JSON.stringify({ type: "peer-left", sessionId }));
        } catch {
          /* peer already gone; ignore */
        }
      }
    });
```

- [ ] **Step 5: Run to verify they pass + full suite**

Run: `npm test`
Expected: the 2 new tests pass; all prior Node tests (64) still green.

- [ ] **Step 6: Typecheck + commit**

Run: `npm run typecheck`
Expected: clean.

```bash
git add packages/server/src/signaling/registry.ts packages/server/src/signaling/hub.ts packages/server/test/signaling.test.ts
git commit -m "feat(server): notify surviving peer with peer-left on connection close

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Agent — reconnect loop + backoff + handle peer-left

**Files:** Modify `agent/src/signaling.rs`

**Interfaces:**
- Produces: `enum SessionOutcome { Retry, Fatal(String) }`; `fn next_backoff(current: Duration) -> Duration`; `async fn run_session(config: &AgentConfig) -> SessionOutcome` (one connection's lifetime); `run_agent(config)` now loops `run_session` with backoff. `run_agent`'s signature is unchanged (`main.rs` calls it).

- [ ] **Step 1: Write the failing unit tests**

Add to the `#[cfg(test)] mod tests` in `agent/src/signaling.rs`:

```rust
    use std::time::Duration;

    #[test]
    fn backoff_doubles_and_caps_at_30s() {
        assert_eq!(next_backoff(Duration::from_secs(1)), Duration::from_secs(2));
        assert_eq!(next_backoff(Duration::from_secs(2)), Duration::from_secs(4));
        assert_eq!(next_backoff(Duration::from_secs(16)), Duration::from_secs(30)); // 32 capped
        assert_eq!(next_backoff(Duration::from_secs(30)), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn run_session_retries_when_server_unreachable() {
        // A refused connection must yield Retry (not Fatal, not a panic).
        let cfg = AgentConfig {
            server_url: "http://127.0.0.1:9".to_string(), // discard port, refused
            device_id: "d".to_string(),
            device_token: "t".to_string(),
        };
        assert!(matches!(run_session(&cfg).await, SessionOutcome::Retry));
    }
```

Note: match the `AgentConfig` construction to its actual fields (read `agent/src/config.rs`; the struct has `server_url`, `device_id`, `device_token` — adapt if there are more/named differently).

- [ ] **Step 2: Run to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib signaling`
Expected: FAIL to compile — `next_backoff`/`run_session`/`SessionOutcome` not found.

- [ ] **Step 3: Refactor into `run_session` + `run_agent` with backoff, add the peer-left arm**

In `agent/src/signaling.rs`, add imports and the new items, and refactor. Add `use std::time::Duration;` at the top (module scope). Add the constants + helpers + `SessionOutcome` above `run_agent`:

```rust
const BASE_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const STABLE_RESET: Duration = Duration::from_secs(60);

/// Result of one signaling connection's lifetime.
enum SessionOutcome {
    /// Transient close/error — reconnect after a backoff.
    Retry,
    /// Permanent failure (e.g. the device token was rejected) — stop.
    Fatal(String),
}

/// Exponential backoff: double, capped at MAX_BACKOFF.
fn next_backoff(current: Duration) -> Duration {
    current.saturating_mul(2).min(MAX_BACKOFF)
}
```

Replace the existing `pub async fn run_agent(config: AgentConfig) -> Result<()> { ... }` with a thin reconnect loop plus a `run_session` holding the old body. `run_session` returns `SessionOutcome` instead of using `?`/`break`:

```rust
pub async fn run_agent(config: AgentConfig) -> Result<()> {
    let mut backoff = BASE_BACKOFF;
    loop {
        let started = std::time::Instant::now();
        match run_session(&config).await {
            SessionOutcome::Fatal(msg) => {
                tracing::error!("agent stopped: {msg}. Re-pair the device (delete its config) and restart.");
                return Err(anyhow::anyhow!(msg));
            }
            SessionOutcome::Retry => {
                // A connection that stayed up a while resets the backoff.
                if started.elapsed() >= STABLE_RESET {
                    backoff = BASE_BACKOFF;
                }
                tracing::warn!("signaling disconnected; reconnecting in {:?}", backoff);
                tokio::time::sleep(backoff).await;
                backoff = next_backoff(backoff);
            }
        }
    }
}

async fn run_session(config: &AgentConfig) -> SessionOutcome {
    let url = ws_url_from(&config.server_url);
    let (ws, _) = match connect_async(&url).await {
        Ok(x) => x,
        Err(e) => {
            tracing::warn!("connect failed: {e}");
            return SessionOutcome::Retry;
        }
    };
    let (mut write, mut read) = ws.split();

    let online = match serde_json::to_string(&SignalingMessage::AgentOnline {
        token: config.device_token.clone(),
    }) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("failed to serialize agent-online: {e}");
            return SessionOutcome::Retry;
        }
    };
    if let Err(e) = write.send(Message::Text(online)).await {
        tracing::warn!("failed to send agent-online: {e}");
        return SessionOutcome::Retry;
    }
    tracing::info!("agent online, waiting for sessions");

    let mut current: Option<(String, PeerSession)> = None;
    let (ice_tx, mut ice_rx) = mpsc::unbounded_channel::<serde_json::Value>();

    loop {
        tokio::select! {
            Some(candidate) = ice_rx.recv() => {
                if let Some((sid, _)) = &current {
                    let out = SignalingMessage::Ice { session_id: sid.clone(), candidate };
                    match serde_json::to_string(&out) {
                        Ok(txt) => {
                            if let Err(e) = write.send(Message::Text(txt)).await {
                                tracing::error!("failed to send local ice candidate: {e}");
                                return SessionOutcome::Retry;
                            }
                        }
                        Err(e) => tracing::warn!("failed to serialize local ice candidate: {e}"),
                    }
                }
            }

            item = read.next() => {
                let item = match item {
                    Some(item) => item,
                    None => return SessionOutcome::Retry,
                };
                let msg = match item {
                    Ok(Message::Text(t)) => t,
                    Ok(Message::Close(_)) | Err(_) => return SessionOutcome::Retry,
                    _ => continue,
                };
                let parsed: SignalingMessage = match serde_json::from_str(&msg) {
                    Ok(m) => m,
                    Err(e) => { tracing::warn!("bad signaling msg: {e}"); continue; }
                };
                match parsed {
                    SignalingMessage::Incoming { session_id, ice_servers, .. } => {
                        if current.is_some() {
                            tracing::info!("incoming session {session_id} supersedes existing current session");
                        }
                        let peer = match PeerSession::new(ice_servers, ice_tx.clone()).await {
                            Ok(p) => p,
                            Err(e) => { tracing::error!("failed to construct peer session for {session_id}: {e}"); continue; }
                        };
                        current = Some((session_id, peer));
                        tracing::info!("incoming session accepted, awaiting offer");
                    }
                    SignalingMessage::Sdp { session_id, sdp } if sdp.kind == "offer" => {
                        if let Some((sid, peer)) = &current {
                            if *sid == session_id {
                                let answer_sdp = match peer.accept_offer(&sdp.sdp).await {
                                    Ok(a) => a,
                                    Err(e) => { tracing::error!("failed to accept offer for session {session_id}: {e}"); continue; }
                                };
                                let reply = match serde_json::to_string(&SignalingMessage::Sdp {
                                    session_id,
                                    sdp: SdpDesc { kind: "answer".into(), sdp: answer_sdp },
                                }) {
                                    Ok(r) => r,
                                    Err(e) => { tracing::error!("failed to serialize answer: {e}"); continue; }
                                };
                                if let Err(e) = write.send(Message::Text(reply)).await {
                                    tracing::error!("failed to send answer: {e}");
                                    return SessionOutcome::Retry;
                                }
                            }
                        }
                    }
                    SignalingMessage::Ice { session_id, candidate } => {
                        if let Some((sid, peer)) = &current {
                            if *sid == session_id {
                                if let Err(e) = peer.add_remote_ice(candidate).await {
                                    tracing::warn!("failed to add remote ice candidate for session {session_id}: {e}");
                                }
                            }
                        }
                    }
                    SignalingMessage::PeerLeft { session_id } => {
                        if let Some((sid, peer)) = &current {
                            if *sid == session_id {
                                let _ = peer.close().await;
                                current = None;
                                tracing::info!("peer left session {session_id}; released");
                            }
                        }
                    }
                    SignalingMessage::Error { code, message } => {
                        tracing::error!("signaling error {code}: {message}");
                        if code == "bad-token" {
                            return SessionOutcome::Fatal(format!("device token rejected: {message}"));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
```

(Note: the old `run_agent` ended with `Ok(())` after `break`; now `run_session` never returns `Ok(())` — it returns a `SessionOutcome`, and `run_agent` owns the `Result`. Keep the existing `ws_url_from` + its test unchanged.)

- [ ] **Step 4: Run to verify tests pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib signaling`
Expected: PASS (`ws_url_http_to_ws`, `backoff_doubles_and_caps_at_30s`, `run_session_retries_when_server_unreachable`).

- [ ] **Step 5: Full suite + clippy + commit**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets`
Expected: all green (Plan 3/4/5 tests intact), clippy clean.

```bash
git add agent/src/signaling.rs
git commit -m "feat(agent): auto-reconnect with backoff + handle peer-left

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Docs — reliability smoke note

**Files:** Modify `docs/superpowers/plan4-video-smoke.md`

- [ ] **Step 1: Append a reliability section**

Add to `docs/superpowers/plan4-video-smoke.md`:

```markdown
## Reliability (Plan 6) — what to verify
- **Agent auto-reconnect:** with a session idle, restart the signaling server (or briefly drop the network). The agent logs `signaling disconnected; reconnecting in …` and comes back `agent online` within the backoff window (≤ 30s); the device returns to online in the web device list and a new session can be started. No manual agent restart needed.
- **Peer-left both ways:** close the browser tab mid-session → the agent logs `peer left session …; released` (and the被控端 releases any held keys). Kill/disconnect the agent mid-session → the web session flips from Connected to Disconnected instead of hanging.
- **Fatal stop:** an invalid device token logs `agent stopped: device token rejected …` and the agent exits (re-pair needed) rather than looping forever.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/plan4-video-smoke.md
git commit -m "docs: Plan 6 reliability smoke notes (reconnect, peer-left, fatal stop)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## After all tasks
- Whole-branch review; update `docs/BACKLOG.md` (mark reconnect + peer-left resolved, refresh counts); `superpowers:finishing-a-development-branch`.

## Self-Review (spec coverage)
- Spec §2.1 reconnect → Task 2 (run_session/run_agent/backoff/SessionOutcome). §2.2 agent peer-left → Task 2 (PeerLeft arm). §2.3 server peer-left on close → Task 1. §2.4 web → unchanged (already handles peer-left). §4 tests → next_backoff + connect-retry unit (agent), 2 peer-left-on-close integration (server). §5 split → server ∥ agent.
- Type consistency: `SessionOutcome`/`next_backoff`/`run_session` used only in Task 2; `Registry.remove` return shape consumed by hub in Task 1.
- Leaf risk: `AgentConfig` field names in the agent test must match `config.rs` (flagged in Task 2 Step 1).
