# Plan 2b — End-to-End Echo Smoke Guide (manual verification)

This is a **runnable, step-by-step manual smoke test** for the Plan 2b milestone:

> browser (React control end) ↔ Rust agent ↔ Node signaling server, over WebRTC,
> exchanging a data-channel echo — send `hello`, expect `echo:hello`.

## Why this is manual, not automated

This guide is marked **manual verification** on purpose. The full loop requires:

- a **real browser** driving `RTCPeerConnection` / a WebRTC data channel (jsdom
  used by the unit tests has no WebRTC stack), and
- **multi-process orchestration** — Node server + coturn container + native Rust
  agent + Vite dev server + a browser tab, all live at once with real ICE
  negotiation.

The individual pieces are covered by automated tests (server route/signaling
tests, the agent's `echo_loopback` WebRTC test, the web unit tests), but the
end-to-end "real browser ↔ real agent over ICE" path is not something we run in
CI. This document is how a human verifies the milestone by hand.

Expected wall-clock: ~10 minutes on a machine that already has the toolchains.

---

## 1. Prerequisites

- **Node ≥ 20** and npm (workspace uses npm workspaces: `@rd/server`, `@rd/web`).
- **Rust** toolchain, cargo on `PATH` (`export PATH="$HOME/.cargo/bin:$PATH"`).
- **Docker** (with Compose) for coturn.
- Repo checked out at the root (`D:\Code\remote-desktop`) on branch
  `feat/plan2b-web-coturn`.
- Dependencies installed once from the repo root: `npm install`.

Open **five terminals** at the repo root (server, coturn, web, agent, and a
scratch terminal for `curl`). All commands below are run from the repo root
unless noted.

> Shell note: the env-var-prefix syntax (`JWT_SECRET=dev ... npm run ...`) is
> bash/zsh. On Windows PowerShell, set the vars first, e.g.
> `$env:JWT_SECRET="dev"; $env:ICE_SERVERS='...'; npm run dev -w @rd/server`.

---

## 2. Start the signaling / REST server (with coturn-aligned ICE_SERVERS)

The server **refuses to start without `JWT_SECRET`** outside test mode
(`packages/server/src/index.ts`). Give it the JWT secret plus the `ICE_SERVERS`
JSON that matches the coturn deployment.

Copy the ICE_SERVERS value **verbatim** from `infra/coturn/README.md`:

```bash
JWT_SECRET=dev \
ICE_SERVERS='[{"urls":"stun:127.0.0.1:3478"},{"urls":"turn:127.0.0.1:3478","username":"rduser","credential":"rdpass"}]' \
npm run dev -w @rd/server
```

- Default port is **8080** (`PORT` overrides — `packages/server/src/config.ts`).
- Optional: `RELAY_POLICY` (`direct-only` | `relay-fallback` | `force-relay`,
  default `relay-fallback`), `DB_PATH` (default `remote-desktop.db`).

**Expected:** the server logs `server on :8080`. The relay policy + these ICE
servers are what the server hands to both peers when a session starts
(`packages/server/src/signaling/hub.ts`, the `incoming` / `session-ready`
messages).

---

## 3. Start coturn

```bash
docker compose -f infra/coturn/docker-compose.yml up -d
```

The compose file uses `network_mode: host`, so coturn binds `3478` (UDP + TCP)
directly on the host with static credentials `rduser:rdpass`, realm
`remote.desktop` — matching the `ICE_SERVERS` from step 2.

**Expected (verify with `docker compose -f infra/coturn/docker-compose.yml logs -f`):**
a `Listener opened on ... port 3478` line and the configured realm.

> Note: for a **same-host** smoke test, STUN + host candidates are enough to
> establish the peer connection; TURN (relay) is there for real cross-NAT
> machines and won't necessarily be exercised locally. Running coturn anyway
> keeps the ICE config honest and lets you fall back to relay.

---

## 4. Register an account, start the web dev server, log in

### 4a. Register an account

Register via the API (scratch terminal). Password must be **≥ 6 chars**
(`packages/server/src/routes/auth.ts`):

```bash
curl -X POST http://127.0.0.1:8080/register \
  -H 'Content-Type: application/json' \
  -d '{"email":"a@b.com","password":"pw123456"}'
```

**Expected:** `{"token":"<jwt>"}`. (You can also register from the browser in the
next step — the login page has a **Register** button — but doing it here means
the agent and browser share the exact same credentials.)

### 4b. Start the web dev server

```bash
npm run dev -w @rd/web
```

Vite serves on **http://localhost:5173** (`packages/web/vite.config.ts`). The web
app talks to the server at `http://127.0.0.1:8080` by default; override with
`VITE_SERVER_URL` if the server isn't on the default host/port
(`packages/web/src/api.ts`).

### 4c. Log in in the browser

Open **http://localhost:5173**, enter `a@b.com` / `pw123456`, click **Log in**
(`packages/web/src/pages/LoginPage.tsx`).

**Expected:** you land on the **Devices** page. With no agent paired yet it shows
"No devices yet."

---

## 5. Pair the agent and confirm it goes online

The agent pairs by **interactive email/password login using the same account**,
then saves a local config and connects. (See the inconsistency note at the end:
there is currently **no** "type the device token into the agent" flow — the agent
provisions its own device token via `/login` + `/devices/pair`.)

From the repo root:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
RD_SERVER_URL=http://127.0.0.1:8080 cargo run --manifest-path agent/Cargo.toml
```

First run (no local config found) — the agent
(`agent/src/main.rs`, `agent/README.md`):

1. Prints `No config found. Log in to pair this device.`
2. Prompts `Email:` / `Password:` — enter the **same** `a@b.com` / `pw123456`
   (password is hidden, not echoed).
3. Calls `/login` for a JWT, then `/devices/pair` (device name = hostname),
   receiving `deviceId` + device token.
4. Writes the config file and prints `Paired as device <deviceId>`.
5. Connects the signaling WebSocket, sends `agent-online` with the device token,
   and logs **`agent online, waiting for sessions`**.

> `RD_SERVER_URL` only matters on the **first** (unpaired) run — after pairing,
> the server URL is baked into the config file. To force a re-pair, delete the
> config (`%APPDATA%\rd-agent\config.json` on Windows) or point `RD_AGENT_CONFIG`
> at a fresh path.

Subsequent runs skip the prompt and log `loaded config for device <deviceId>`
then go online.

### Confirm online in the browser

Back in the **Devices** page, refresh (or re-open) — the device should now show a
**green dot** and an **enabled "Connect" button**
(`packages/web/src/pages/DevicesPage.tsx`; online status comes from the server's
in-memory registry — `registry.isOnline`, `packages/server/src/signaling/hub.ts`).

**Expected:** device listed, green (online) indicator, Connect enabled.

---

## 6. Open the session and verify the echo

In the browser Devices page, click **Connect** on the online device → you enter
the **SessionView** (`packages/web/src/pages/SessionView.tsx`).

Under the hood, on connect the web client:

1. Opens a signaling WebSocket authenticated with `?token=<jwt>`.
2. Sends a `connect` message with the `deviceId`. The server validates ownership,
   creates a session, sends `incoming` (with `sessionId` + relay policy + ICE
   servers) to the agent and `session-ready` to the browser
   (`packages/server/src/signaling/hub.ts`).
3. Creates the WebRTC data channel, sends its SDP **offer**; the agent answers;
   ICE candidates are relayed both ways until connected.

In SessionView, type **`hello`** into the message box and send it.

**Expected:** the browser displays **`echo:hello`** — the agent's data-channel
echo (`echo:<msg>`) round-tripped back over WebRTC.

---

## 7. Expected logs / state at each side

| Where | What you should see |
|---|---|
| **Server** (terminal) | `server on :8080` at startup. (Logger is off by default, so no per-request lines.) |
| **coturn** (docker logs) | `Listener opened on ... port 3478`, realm `remote.desktop`. |
| **Agent** (terminal) | `Paired as device <id>` (first run) → `agent online, waiting for sessions` → on connect: `incoming session accepted, awaiting offer` → then it sends an SDP **answer** (`agent/src/signaling.rs`). |
| **Browser** — Devices | Target device with a **green** (online) dot; **Connect** enabled. |
| **Browser** — SessionView | Peer connection reaches **connected**; sending `hello` shows **`echo:hello`**. |

If all of the above hold, the Plan 2b milestone is verified.

---

## 8. Troubleshooting

**Device shows offline (grey dot, Connect disabled)**
- The agent isn't connected. Confirm the agent terminal shows
  `agent online, waiting for sessions`. If it exited or errored, re-run it.
- The web page caches the device list — leave and re-enter the Devices page (or
  refresh) to re-fetch online status.
- Online status is **in-memory** on the server: if you restarted the server, the
  agent must reconnect (re-run it) to re-register as online.
- Wrong account: the agent must pair with the **same** user whose token the
  browser is logged in with, or the device won't appear in that user's list.

**ICE fails / peer connection never reaches "connected"**
- Confirm coturn is up: `docker compose -f infra/coturn/docker-compose.yml ps`
  and check the logs for the `3478` listener.
- Confirm the server's `ICE_SERVERS` matches coturn exactly — the STUN/TURN URLs,
  `rduser`/`rdpass`, and port `3478` must line up with `infra/coturn/README.md`.
  A mismatch (e.g. server still on the default Google STUN) can stop candidates
  from forming.
- For a cross-machine test, replace `127.0.0.1` in `ICE_SERVERS` with the coturn
  host's reachable IP, and try `RELAY_POLICY=force-relay` to force the TURN path.

**CORS error in the browser console (request blocked)**
- The server only allows `http://localhost:<port>` and `http://127.0.0.1:<port>`
  origins (`packages/server/src/app.ts`). If you serve the web app from a
  different host/origin, the API call is blocked. Access the web app via
  `localhost` / `127.0.0.1`, and if you moved the server, set `VITE_SERVER_URL`
  to a matching allowed origin.

**Server won't start**
- `JWT_SECRET environment variable is required` → you didn't set `JWT_SECRET`
  (`packages/server/src/index.ts`). Prefix it as in step 2.
- `Invalid ICE_SERVERS: must be valid JSON` → the JSON got mangled by the shell.
  Keep it single-quoted exactly as in step 2 (or set `$env:ICE_SERVERS` in
  PowerShell).

---

## Teardown

- Stop the server / web / agent terminals (Ctrl-C).
- Stop coturn: `docker compose -f infra/coturn/docker-compose.yml down`.
- To reset state: delete the server DB (`remote-desktop.db`) and the agent config
  (`%APPDATA%\rd-agent\config.json`).
