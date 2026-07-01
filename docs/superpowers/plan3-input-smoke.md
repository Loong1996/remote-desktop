# Plan 3 — Input injection e2e smoke

Prereq: complete the Plan 2b bring-up (server + coturn + agent + web) from
`plan2b-e2e-smoke.md` so a session connects. macOS: grant the agent binary
Accessibility permission (System Settings → Privacy & Security → Accessibility)
and restart it — otherwise injection is silently disabled (the agent logs a
warning at startup).

---

## Plan 2b Bring-Up (prerequisite)

### Prerequisites

- **Node ≥ 20** and npm (workspace uses npm workspaces: `@rd/server`, `@rd/web`).
- **Rust** toolchain, cargo on `PATH` (`export PATH="$HOME/.cargo/bin:$PATH"`).
- **Docker** (with Compose) for coturn.
- Repo checked out at the root on branch `plan3-input-injection`.
- Dependencies installed once from the repo root: `npm install`.

Open **five terminals** at the repo root (server, coturn, web, agent, and a
scratch terminal for `curl`). All commands below are run from the repo root
unless noted.

> Shell note: the env-var-prefix syntax (`JWT_SECRET=dev ... npm run ...`) is
> bash/zsh. On Windows PowerShell, set the vars first, e.g.
> `$env:JWT_SECRET="dev"; $env:ICE_SERVERS='...'; npm run dev -w @rd/server`.

### 1. Start the signaling / REST server (with coturn-aligned ICE_SERVERS)

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

### 2. Start coturn

```bash
docker compose -f infra/coturn/docker-compose.yml up -d
```

The compose file uses `network_mode: host`, so coturn binds `3478` (UDP + TCP)
directly on the host with static credentials `rduser:rdpass`, realm
`remote.desktop` — matching the `ICE_SERVERS` from step 1.

**Expected (verify with `docker compose -f infra/coturn/docker-compose.yml logs -f`):**
a `Listener opened on ... port 3478` line and the configured realm.

> Note: for a **same-host** smoke test, STUN + host candidates are enough to
> establish the peer connection; TURN (relay) is there for real cross-NAT
> machines and won't necessarily be exercised locally. Running coturn anyway
> keeps the ICE config honest and lets you fall back to relay.

### 3. Register an account, start the web dev server, log in

#### 3a. Register an account

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

#### 3b. Start the web dev server

```bash
npm run dev -w @rd/web
```

Vite serves on **http://localhost:5173** (`packages/web/vite.config.ts`). The web
app talks to the server at `http://127.0.0.1:8080` by default; override with
`VITE_SERVER_URL` if the server isn't on the default host/port
(`packages/web/src/api.ts`).

#### 3c. Log in in the browser

Open **http://localhost:5173**, enter `a@b.com` / `pw123456`, click **Log in**
(`packages/web/src/pages/LoginPage.tsx`).

**Expected:** you land on the **Devices** page. With no agent paired yet it shows
"No devices yet."

### 4. Pair the agent and confirm it goes online

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

#### Confirm online in the browser

Back in the **Devices** page, refresh (or re-open) — the device should now show a
**green dot** and an **enabled "Connect" button**
(`packages/web/src/pages/DevicesPage.tsx`; online status comes from the server's
in-memory registry — `registry.isOnline`, `packages/server/src/signaling/hub.ts`).

**Expected:** device listed, green (online) indicator, Connect enabled.

---

## Plan 3 Verification Steps

Once a session is connected and green "Connected" badge shows, verify input injection:

### 1. Open a session to the online device

In the browser Devices page, click **Connect** on the online device → you enter
the **SessionView** (`packages/web/src/pages/SessionView.tsx`).

**Expected:** "Connected" badge is green.

### 2. Focus the remote screen panel

Click the dashed "Remote screen" panel to focus it (cursor becomes a crosshair).

### 3. Move the mouse

Move the mouse across the panel → the controlled device's real cursor moves; the
"Sent events" log shows `mmove x,y` lines (coalesced to ~one per frame).

### 4. Click / right-click / middle-click

Click / right-click / middle-click → real clicks on the controlled device; log
shows `mdown`/`mup` with the button. Right-click does not open the browser menu.

### 5. Scroll the wheel

Scroll the wheel over the panel → the controlled device scrolls; log shows
`wheel dx,dy`.

### 6. Type text

Type (e.g. open a text editor on the controlled device first, focus the panel,
type "Hello") → text appears on the controlled device; log shows `kdown`/`kup
KeyH` … Shift combos capitalize. Press Esc to release capture.

### 7. Check agent logs

Agent logs (`RUST_LOG=info`) show received events; malformed/non-utf8 frames
log a warning rather than crashing.

---

## Expected

Real cursor movement, clicks, scroll, and typing on the controlled device,
mirrored by the event log on the control end. No ack is sent back
(fire-and-forget).
