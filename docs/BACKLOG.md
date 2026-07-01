# Remote Desktop — Backlog & Handoff

> Durable handoff doc (in-repo, so it travels with the code). Records project status, the roadmap, all deferred/carry-over items, and how to resume on a fresh machine. Last updated: 2026-07-02.

## Status

**MVP functionally complete (macOS):** the browser shows the被控端's live macOS screen (`<video>`) AND mouse/keyboard injection works — both媒体 flows over one WebRTC PeerConnection (video track agent→web, input data channel web→agent). This is the design's MVP scope (画面传输 + 键鼠控制). Remaining work is hardening + cross-platform, not new MVP capability.

| Plan | Scope | Status |
|------|-------|--------|
| 1 | `@rd/protocol` (TS types+validation) + `@rd/server` (Fastify+ws+sqlite: accounts/JWT, device list/pair, WS SDP/ICE relay) | ✅ merged |
| 2a | `agent/` Rust `rd-agent` (config persist, protocol serde mirror, interactive-login provision, WebRTC answerer + data-channel echo, signaling WS loop) | ✅ merged |
| 2b | `@rd/web` (React: login, device list, WebRTC session + echo UI), `infra/coturn`, agent bidirectional trickle ICE, server CORS | ✅ merged |
| 3 | **Mouse/keyboard injection** (`enigo` injector thread + full `KeyboardEvent.code` keymap, macOS Accessibility check, web capture panel + event log, pre-offer ICE buffer) | ✅ merged |
| 4 | **Screen capture + H.264 video** (ScreenCaptureKit capture + openh264 software encode behind traits, VideoPipeline thread → sendonly H264 track, web recvonly transceiver + `<video>`, macOS Screen-Recording check; test-pattern-first de-risking) | ✅ merged, macOS only |
| 5 | **macOS video hardening** (SckCapturer owns/stops its SCStream on session end + 720p/30fps capture; agent+web stuck-key release; letterbox coord mapping; convert 0×0 guard) | ✅ merged |
| 6 | **Agent reliability + session lifecycle** (agent WS auto-reconnect w/ exponential backoff, fatal-stop on bad-token; server sends `peer-left` on any connection close; agent handles `peer-left` → releases session) | ✅ done (branch `plan6-agent-reliability`) |

Tests currently green: Node `npm test` → 66; agent `cargo test` → 39 (+2 `#[ignore]` real-hardware: SCK capture, enigo injection); `npm run typecheck` clean; `cargo clippy --all-targets` clean; web `@rd/web build` clean.

Plan 6 docs: `docs/superpowers/specs/2026-07-02-plan6-agent-reliability-design.md`, `docs/superpowers/plans/2026-07-02-plan6-agent-reliability.md`; reliability smoke notes in `plan4-video-smoke.md`.

Plan 5 docs: `docs/superpowers/specs/2026-07-02-plan5-macos-video-hardening-design.md`, `docs/superpowers/plans/2026-07-02-plan5-macos-video-hardening.md`; smoke notes appended to `plan4-video-smoke.md`.

Design/plan for Plan 4: `docs/superpowers/specs/2026-07-02-plan4-screen-capture-video-design.md`, `docs/superpowers/plans/2026-07-02-plan4-screen-capture-video.md`. Manual smoke: `docs/superpowers/plan4-video-smoke.md` (two-stage: test pattern, then real screen).
Plan 3 docs: `docs/superpowers/specs/2026-07-01-plan3-input-injection-design.md`, `docs/superpowers/plans/2026-07-01-plan3-input-injection.md`, `docs/superpowers/plan3-input-smoke.md`.
Overall design: `docs/superpowers/specs/2026-07-01-remote-desktop-design.md`. E2E smoke: `docs/superpowers/plan2b-e2e-smoke.md`.

## Cross-platform status

**CI (`.github/workflows/ci.yml`) is green on macOS + Linux + Windows.** Per-platform screen-capture status behind the `ScreenCapturer` trait:
- **macOS** — ScreenCaptureKit (`SckCapturer`), full remote desktop, live-validated.
- **Windows** — `xcap` (`XcapCapturer`); compiles + links in CI. Full pipeline (capture+input+encode) is buildable; needs a functional smoke on a real Windows box (`#[ignore]` real-capture test + browser).
- **Linux** — capture NOT wired yet: `make_source` falls back to the test pattern. `xcap`'s Linux/pipewire build failed in CI (deps installed, but `cargo test` failed at the xcap build — needs log access / a Linux box to diagnose, or swap to an X11-only crate like `scrap`).

Input injection (`enigo`) and openh264 software encode already compile + pass on all three OSes.

## Next: finish cross-platform

1. **Linux screen capture** — get `xcap` building on Linux (diagnose the CI `cargo test` failure — likely a missing pipewire/SPA/portal dev dep or an xcap 0.9.6 Linux issue), or implement an X11 capturer via `scrap`/`x11cap` behind `ScreenCapturer`, wired into `make_source`'s Linux branch (currently test-pattern). Same rigor bar as macOS: CI compile + `#[ignore]` real test + manual smoke.
2. **Windows functional smoke** — run the agent on Windows, confirm the browser shows the Windows screen + input works (the `#[ignore]` `xcap_capturer::captures_a_real_frame` + the e2e smoke).
3. **Hardware encode** (perf): VideoToolbox (macOS), NVENC/QSV (Windows), VAAPI (Linux) behind the `VideoEncoder` trait — optional; software openh264 is the portable baseline.
2. **Bitrate/resolution/fps adaptation** (design §4.1 deferred): react to congestion; multi-monitor; resolution-change renegotiation.
3. **Server/pairing hardening** (below): reconnect loop, peer-left notification, atomic register, case-insensitive email, token-based agent pairing.
4. Remaining **Plan 3/5 minor follow-ups** below.

## Resolved in Plan 5 (macOS video hardening)
- ~~SCStream leaked per session~~ — FIXED: `SckCapturer` owns its `SCStream` on a dedicated thread and `stop_capture()`s on `Drop`.
- ~~SCK native retina + 60fps~~ — FIXED: captures at 1280×720/30fps at the source (`with_width/with_height/with_fps`).
- ~~Letterbox coordinate skew~~ — FIXED: `contentRect` maps pointer to the `object-fit: contain` content box.
- ~~`RD_VIDEO_SOURCE` test not serialized~~ — FIXED: `#[serial]`.
- ~~convert 0×0 panic / padded-stride untested~~ — FIXED: 0×0 guard + strengthened padded-stride test.
- ~~Stuck keys/buttons on release~~ — FIXED: web releases held keys/buttons on blur/mouseleave/unmount; agent releases all held on input-channel close.

## Remaining minor follow-ups
- **Wheel `deltaMode` ignored.** Web sends raw `deltaX/deltaY`; `pixels_to_clicks` (`agent/src/input.rs`) assumes deltaMode 0 (~100px/notch). Firefox physical wheel often reports deltaMode 1 (lines, `deltaY≈±3`) → rounds to ±1 floor, losing magnitude. Fix: normalize by `deltaMode` on the web side or send `deltaMode` on the wire.
- **`rect.width===0` → NaN coords** (`SessionView.tsx`): if the video element is unlaid-out while `videoWidth>0`, `mouseCoords` divides by zero. Pre-existing; guard it.
- **English/Chinese comment mix** in a few Rust files ("被控端") — cosmetic cleanup.
- **No RTL component tests** for SessionView capture/blur wiring — add a React Testing Library harness.
- **Scroll direction unverified against real hardware.** Unit tests pin magnitude/sign of the mapper only; confirm browser `deltaY>0` (down) ↔ `enigo::scroll(+)` physically in the smoke run.
- **`enigo.main_display()` queried on every `MMove`** (`agent/src/input.rs`). Fine at rAF-coalesced ~60/s, but caching display size avoids a syscall per move. Trivial.

## Deferred / carry-over items (fix opportunistically)

### Done in Plan 3
- ~~**A. Pre-offer ICE candidates dropped**~~ — FIXED: `agent/src/webrtc_peer.rs` now buffers remote candidates in an `IceBuffer` until the remote description is set, then flushes them in order (unit + integration tested).

### Server (`packages/server`)
- `/register` `findByEmail`→`create` not atomic → concurrent duplicate email surfaces a raw 500 instead of 409. Wrap `create` in try/catch mapping the SQLite UNIQUE error → 409.
- Email lookup is case-sensitive. Normalize to lowercase on register + login (and store normalized).
- ~~Agent mid-session disconnect doesn't notify the surviving peer~~ — FIXED in Plan 6: `Registry.remove` returns affected peers and the hub sends `peer-left` on any connection close; agent + web both consume it.
- Minor test gaps: repo `findById`/not-found paths, garbage-`Bearer`→401, `agent-online`-with-bad-token WS test.
- CORS is dev-only (`http://localhost|127.0.0.1:\d+`). Add a config-driven allowlist (incl. https) for production.

### Agent (`agent/`)
- ~~No reconnect loop~~ — FIXED in Plan 6: `run_agent` now loops `run_session` with exponential backoff (cap 30s), stopping only on a `bad-token` Fatal.
- Superseded `PeerSession` (new `incoming` while one is active) is dropped without `close()`. Call `close()` on the old one.
- Superseded-session candidate mis-tagging (a queued candidate from the old session can be tagged with the new session id). Benign (browser rejects mismatched-cred candidates); tag candidates with their session id at emit time to be safe.
- `accept_offer` still waits for full ICE gathering before returning the answer (latency; candidates also trickle). Return the answer sooner for lower connect latency.
- Silent-drop paths want `tracing` logs: non-UTF8 data-channel frame, data-channel send failure, malformed ICE `urls`.
- 5 of 8 `SignalingMessage` variants lack serde round-trip tests (connect/session-ready/ice/peer-left/error) — verified by inspection only. Add tests to guard the wire contract.
- `provision` JSON parse errors lack `.context`; `/devices/pair` non-2xx path untested.
- `hostname_or` reads only `COMPUTERNAME`/`HOSTNAME` env, not a real hostname syscall.

### Web (`packages/web`)
- ~~Device online status fetched once on mount~~ — FIXED (Plan 7): DevicesPage polls `/devices` every 5s (silent, no flicker), so online/offline + agent reconnects show without a manual reload. (A WS-driven push could replace polling later.)
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
   npm test           # expect 66 passing
   npm run typecheck  # clean
   cargo test --manifest-path agent/Cargo.toml   # expect 39 passing +2 ignored (first build pulls webrtc-rs + enigo + openh264 source + screencapturekit — slow)
   ```
4. **Try the e2e echo:** follow `docs/superpowers/plan2b-e2e-smoke.md` (server + coturn + agent + web → browser shows `echo:hello`).
5. **Read before coding:** the design spec, then `docs/superpowers/plans/`, then this backlog.

### Project conventions (were in local machine memory — re-apply on the new machine)
- **Test before commit:** every step must pass its tests before it is committed.
- **Keep planning in-repo:** design docs, plans, progress live under `docs/` in this repo.
- **Brainstorm → write spec → write plan → execute** (superpowers skills). Each plan produces independently testable software.
- **Subagent-driven execution:** fresh subagent per task + independent review per task + a final whole-branch review; **parallelize independent tasks** (disjoint dirs → dispatch concurrently; controller commits each separately to avoid git index races).
- Commit messages in English, ending with the Co-Authored-By trailer.
