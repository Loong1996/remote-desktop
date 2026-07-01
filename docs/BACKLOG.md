# Remote Desktop — Backlog & Handoff

> Durable handoff doc (in-repo, so it travels with the code). Records project status, the roadmap, all deferred/carry-over items, and how to resume on a fresh machine. Last updated: 2026-07-01.

## Status

Milestone reached: **browser ↔ Rust agent ↔ Node server exchange a data-channel `echo:<msg>` over WebRTC.**

| Plan | Scope | Status |
|------|-------|--------|
| 1 | `@rd/protocol` (TS types+validation) + `@rd/server` (Fastify+ws+sqlite: accounts/JWT, device list/pair, WS SDP/ICE relay) | ✅ merged |
| 2a | `agent/` Rust `rd-agent` (config persist, protocol serde mirror, interactive-login provision, WebRTC answerer + data-channel echo, signaling WS loop) | ✅ merged |
| 2b | `@rd/web` (React: login, device list, WebRTC session + echo UI), `infra/coturn`, agent bidirectional trickle ICE, server CORS | ✅ merged |
| 3 | **Mouse/keyboard injection** (next) | ⏳ |
| 4 | Screen capture + H.264 video | ⏳ |

Tests currently green: Node `npm test` → 54; agent `cargo test` → 10; `npm run typecheck` clean; web `vite build` clean.

Design: `docs/superpowers/specs/2026-07-01-remote-desktop-design.md`. Plans: `docs/superpowers/plans/`. E2E smoke: `docs/superpowers/plan2b-e2e-smoke.md`.

## Next: Plan 3 — mouse/keyboard injection

Reuse the SAME data channel that Plan 2b opened. The `InputEvent` type is already defined in `packages/protocol/src/input.ts` (mouse move w/ 0..1 relative coords, buttons, wheel, key down/up) with a `parseInputEvent` validator.

- **Web** (`packages/web/src/pages/SessionView.tsx` + rtc): capture mouse move/click/wheel and keyboard on the remote-view surface → serialize `InputEvent` JSON → send over the data channel.
- **Agent** (`agent/`): new `input` module using the `enigo` crate to inject received `InputEvent`s; map 0..1 relative coords → local screen resolution; add a Rust serde mirror of `InputEvent`. Handle platform permission prompts (macOS Accessibility; Linux/Wayland limits) — detect + guide on first run.
- **Do first (insurance before real traffic):** buffer inbound remote ICE candidates until the remote description is set — see "Known issues" #A below.

## Deferred / carry-over items (fix opportunistically)

### Must consider before Plan 3 traffic
- **A. Pre-offer ICE candidates dropped** — `agent/src/signaling.rs`: a remote `ice` arriving before the offer's remote description is set fails `add_remote_ice` and is dropped (log-and-continue). Benign for the ordered single-WS echo flow, but buffer-until-remote-desc-set is cheap insurance once the channel carries real input.

### Server (`packages/server`)
- `/register` `findByEmail`→`create` not atomic → concurrent duplicate email surfaces a raw 500 instead of 409. Wrap `create` in try/catch mapping the SQLite UNIQUE error → 409.
- Email lookup is case-sensitive. Normalize to lowercase on register + login (and store normalized).
- Agent mid-session disconnect doesn't notify the surviving peer (no `peer-left`/`error`). Design §5⑥/§7 require it — wire `Registry.remove`/close to emit to the peer. Web/agent then consume it (stop hanging).
- Minor test gaps: repo `findById`/not-found paths, garbage-`Bearer`→401, `agent-online`-with-bad-token WS test.
- CORS is dev-only (`http://localhost|127.0.0.1:\d+`). Add a config-driven allowlist (incl. https) for production.

### Agent (`agent/`)
- No reconnect loop — `run_agent` returns `Ok(())` and the process exits when the signaling socket closes. Add exponential-backoff reconnect.
- Superseded `PeerSession` (new `incoming` while one is active) is dropped without `close()`. Call `close()` on the old one.
- Superseded-session candidate mis-tagging (a queued candidate from the old session can be tagged with the new session id). Benign (browser rejects mismatched-cred candidates); tag candidates with their session id at emit time to be safe.
- `accept_offer` still waits for full ICE gathering before returning the answer (latency; candidates also trickle). Return the answer sooner for lower connect latency.
- Silent-drop paths want `tracing` logs: non-UTF8 data-channel frame, data-channel send failure, malformed ICE `urls`.
- 5 of 8 `SignalingMessage` variants lack serde round-trip tests (connect/session-ready/ice/peer-left/error) — verified by inspection only. Add tests to guard the wire contract.
- `provision` JSON parse errors lack `.context`; `/devices/pair` non-2xx path untested.
- `hostname_or` reads only `COMPUTERNAME`/`HOSTNAME` env, not a real hostname syscall.

### Web (`packages/web`)
- Device online status is fetched once on mount — no polling/refresh (and stays stale). Add polling or a WS-driven update.
- JWT stored in `localStorage` (XSS-exposable) — standard SPA tradeoff; revisit before public deployment.
- FIFO echo pairing in `SessionView`; `offer.sdp ?? ""` masks an invariant; `String(ArrayBuffer)` fallback for non-text frames (never triggers today).

### Pairing model
- Split flow: the web "Pair" button issues a device token, but the agent pairs via its own interactive login and has **no token-entry path** (DevicesPage copy now discloses this). Unify: either add token-based provisioning to the agent (paste the web-issued token) or make agent self-pairing the canonical flow and repurpose the web token.

### Infra (`infra/coturn`)
- `turnserver.conf` lacks `external-ip` / relay port range and uses static `rduser:rdpass` + `no-tls`. Fine for local dev; production needs public IP, `use-auth-secret`, and TLS (README documents this).

## Resuming on a fresh machine

1. **Clone:** `git clone https://github.com/Loong1996/remote-desktop.git && cd remote-desktop`
2. **Toolchains:** Node ≥20 (repo built on v24), Rust (rustup — `cargo`/`rustc` on PATH), Docker + Compose (for coturn).
3. **Install + verify:**
   ```bash
   npm install
   npm test           # expect 54 passing
   npm run typecheck  # clean
   cargo test --manifest-path agent/Cargo.toml   # expect 10 passing (first build pulls webrtc-rs — slow)
   ```
4. **Try the e2e echo:** follow `docs/superpowers/plan2b-e2e-smoke.md` (server + coturn + agent + web → browser shows `echo:hello`).
5. **Read before coding:** the design spec, then `docs/superpowers/plans/`, then this backlog.

### Project conventions (were in local machine memory — re-apply on the new machine)
- **Test before commit:** every step must pass its tests before it is committed.
- **Keep planning in-repo:** design docs, plans, progress live under `docs/` in this repo.
- **Brainstorm → write spec → write plan → execute** (superpowers skills). Each plan produces independently testable software.
- **Subagent-driven execution:** fresh subagent per task + independent review per task + a final whole-branch review; **parallelize independent tasks** (disjoint dirs → dispatch concurrently; controller commits each separately to avoid git index races).
- Commit messages in English, ending with the Co-Authored-By trailer.
