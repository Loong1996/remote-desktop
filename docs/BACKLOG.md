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
| 4 | **Screen capture + H.264 video** (ScreenCaptureKit capture + openh264 software encode behind traits, VideoPipeline thread → sendonly H264 track, web recvonly transceiver + `<video>`, macOS Screen-Recording check; test-pattern-first de-risking) | ✅ done (branch `plan4-screen-capture-video`), macOS only |

Tests currently green: Node `npm test` → 59; agent `cargo test` → 33 (+2 `#[ignore]` real-hardware: SCK capture, enigo injection); `npm run typecheck` clean; `cargo clippy --all-targets` clean; web `@rd/web build` clean.

Design/plan for Plan 4: `docs/superpowers/specs/2026-07-02-plan4-screen-capture-video-design.md`, `docs/superpowers/plans/2026-07-02-plan4-screen-capture-video.md`. Manual smoke: `docs/superpowers/plan4-video-smoke.md` (two-stage: test pattern, then real screen).
Plan 3 docs: `docs/superpowers/specs/2026-07-01-plan3-input-injection-design.md`, `docs/superpowers/plans/2026-07-01-plan3-input-injection.md`, `docs/superpowers/plan3-input-smoke.md`.
Overall design: `docs/superpowers/specs/2026-07-01-remote-desktop-design.md`. E2E smoke: `docs/superpowers/plan2b-e2e-smoke.md`.

## Next: hardening + cross-platform (no new MVP capability)

MVP is complete on macOS. Highest-value next steps, roughly ordered:
1. **Plan 4 follow-ups** below (SCStream cleanup, SCK capture config, letterbox coords) — make the macOS video path production-worthy.
2. **Cross-platform capture/encode** (Windows/Linux): implement `ScreenCapturer`/`VideoEncoder` for those platforms behind the existing traits (`scrap`/`xcap`, hardware encoders VideoToolbox/NVENC/VAAPI).
3. **Bitrate/resolution/fps adaptation** (design §4.1 deferred): react to congestion; multi-monitor; resolution-change renegotiation.
4. The **Plan 3 input follow-ups** below (stuck keys on release is highest-value).

## Plan 4 follow-ups (from the final whole-branch review)

- **SCStream leaked per session — clean up on shutdown.** `agent/src/video/sck_capturer.rs` uses `std::mem::forget(stream)` to keep capturing; across reconnects on a long-lived agent, leaked `SCStream`s accumulate and ALL keep capturing the display simultaneously (unbounded CPU/memory). Fix: hold the stream in `SckCapturer`/the pipeline and `stop_capture()` on drop.
- **SCK captures at native retina + native refresh.** `sck_capturer.rs` sets no `with_width/with_height` or `minimum_frame_interval`, so SCK delivers full-res frames at up to 60fps that get nearest-neighbour-resized to 720p and encoded — heavy CPU; and each `Sample.duration` is hardcoded `1/30`, a real-time/RTP-timestamp mismatch if SCK ≠ 30fps. Fix: configure capture scale + frame interval to match the pipeline's 720p/30fps.
- **Letterbox coordinate skew.** `SessionView.tsx` `mouseCoords` maps against the `<video>` element rect, but `objectFit: contain` letterboxes a differing-aspect stream — pointer coords in the bars map out of range / with a stretch offset. Fix: map against the actual displayed video content box (account for aspect ratio and bars).
- **`RD_VIDEO_SOURCE` test isn't serialized.** `agent/src/video/mod.rs` `testpattern_env_forces_synthetic_source` mutates the process-global env var without `#[serial]` (serial_test is already a dev-dep) — latent parallel-test flake. Annotate it.
- **`convert.rs` robustness/coverage.** `bgra_to_yuv420(..).expect(..)` panics on a 0×0 target (unreachable at fixed 720p, but harden); the padded-source-stride resize path is correct but untested — add a padded-stride test.

## Plan 3 follow-ups (from the final whole-branch review — do before/with Plan 4)

- **Stuck keys/buttons on release (highest value).** `packages/web/src/pages/SessionView.tsx`: if the operator holds Shift/Ctrl or a mouse button and then presses Esc, drags off the panel, or the tab loses focus, no `kup`/`mup` is sent — the modifier/button stays pressed on the被控端. Fix: track currently-pressed keys/buttons and synthesize `kup`/`mup` for all of them on blur; and/or have the agent auto-release everything when the data channel closes.
- **Wheel `deltaMode` ignored.** Web sends raw `deltaX/deltaY`; `pixels_to_clicks` (`agent/src/input.rs`) assumes deltaMode 0 (~100px/notch). Firefox physical wheel often reports deltaMode 1 (lines, `deltaY≈±3`) → rounds to ±1 floor, losing magnitude. Fix: normalize by `deltaMode` on the web side or send `deltaMode` on the wire.
- **Scroll direction unverified against real hardware.** Unit tests pin magnitude/sign of the mapper only; confirm browser `deltaY>0` (down) ↔ `enigo::scroll(+)` physically in the smoke run.
- **`enigo.main_display()` queried on every `MMove`** (`agent/src/input.rs`). Fine at rAF-coalesced ~60/s, but caching display size avoids a syscall per move. Trivial.

## Deferred / carry-over items (fix opportunistically)

### Done in Plan 3
- ~~**A. Pre-offer ICE candidates dropped**~~ — FIXED: `agent/src/webrtc_peer.rs` now buffers remote candidates in an `IceBuffer` until the remote description is set, then flushes them in order (unit + integration tested).

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
   npm test           # expect 59 passing
   npm run typecheck  # clean
   cargo test --manifest-path agent/Cargo.toml   # expect 33 passing +2 ignored (first build pulls webrtc-rs + enigo + openh264 source + screencapturekit — slow)
   ```
4. **Try the e2e echo:** follow `docs/superpowers/plan2b-e2e-smoke.md` (server + coturn + agent + web → browser shows `echo:hello`).
5. **Read before coding:** the design spec, then `docs/superpowers/plans/`, then this backlog.

### Project conventions (were in local machine memory — re-apply on the new machine)
- **Test before commit:** every step must pass its tests before it is committed.
- **Keep planning in-repo:** design docs, plans, progress live under `docs/` in this repo.
- **Brainstorm → write spec → write plan → execute** (superpowers skills). Each plan produces independently testable software.
- **Subagent-driven execution:** fresh subagent per task + independent review per task + a final whole-branch review; **parallelize independent tasks** (disjoint dirs → dispatch concurrently; controller commits each separately to avoid git index races).
- Commit messages in English, ending with the Co-Authored-By trailer.
