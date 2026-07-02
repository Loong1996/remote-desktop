# Plan 4 — Screen capture + video e2e smoke

Prereq: complete the Plan 3 bring-up (server + coturn + agent + web, session
connects, input works) from `plan3-input-smoke.md`. macOS: grant the agent
**Screen Recording** permission (System Settings → Privacy & Security → Screen
Recording) and restart it — otherwise the remote video is blank (the agent logs
a warning at startup).

## Stage 1 — test pattern (proves encode + transport + browser decode)
1. Start the agent with `RD_VIDEO_SOURCE=testpattern` (prefix the run command).
2. Connect from the browser and open the session.
3. **Expected:** the `<video>` shows an animated gradient with a marching white
   square. This confirms H.264 negotiation, Annex-B packetization, the video
   m-line, and browser decode — independent of screen capture.
4. Confirm Plan 3 still works: focus the video, move the mouse / type → the
   controlled machine responds; "Sent events" log updates.

## Stage 2 — real screen
1. Restart the agent WITHOUT `RD_VIDEO_SOURCE` (defaults to `screen`).
2. Reconnect.
3. **Expected:** the `<video>` shows the被控端's live main display. Moving the
   controlled machine's windows is visible in near real time.

## Expected
Live remote screen in the browser plus working mouse/keyboard injection over the
same PeerConnection. Fixed 720p/30fps/3Mbps (no adaptation yet).

## Hardening (Plan 5) — what to verify
- **720p/30fps at the source:** the agent captures at 1280×720/30fps (not native retina/60fps); CPU should be markedly lower than the first Plan 4 cut.
- **Capture stops on disconnect:** close the browser tab / disconnect → the agent stops capturing (no lingering CPU), and reconnecting does not accumulate capture load.
- **No stuck keys:** hold Shift (or a mouse button), then press Esc / move the pointer off the video / close the tab → the被控端 releases it (no stuck modifier/button).
- **Letterbox coords:** if the remote aspect ratio differs from the `<video>` box, the cursor tracks the visible image; positions over the black bars clamp to the edge.

## Reliability (Plan 6) — what to verify
- **Agent auto-reconnect:** with a session idle, restart the signaling server (or briefly drop the network). The agent logs `signaling disconnected; reconnecting in …` and comes back `agent online` within the backoff window (≤ 30s); the device returns to online in the web device list and a new session can be started. No manual agent restart needed.
- **Peer-left both ways:** close the browser tab mid-session → the agent logs `peer left session …; released` (and the被控端 releases any held keys). Kill/disconnect the agent mid-session → the web session flips from Connected to Disconnected instead of hanging.
- **Fatal stop:** an invalid device token logs `agent stopped: device token rejected …` and the agent exits (re-pair needed) rather than looping forever.

## Automated e2e run — result (2026-07-02)

Ran the whole loop against real processes (server + agent + real Chrome over
WebRTC) and confirmed the MVP works end to end on macOS:

- **Video, real screen:** with the agent on the default `screen` source, the
  browser rendered the live macOS desktop (ScreenCaptureKit → openh264 →
  WebRTC → `<video>`). First frame takes a few seconds (SCK start + first
  H.264 keyframe) — a brief black panel at connect is normal.
- **Video, test pattern:** `RD_VIDEO_SOURCE=testpattern` shows the animated
  gradient + marching square, proving encode/transport/decode independent of
  capture.
- **Input:** typed text landed in a terminal on the被控端; `mmove` cursor
  events flow. "Sent events" mirrors what the web sent.

### Bug found + fixed by this run: macOS keyboard injection

The first real keystroke test spammed `enigo::platform::macos_impl:
UCKeyTranslate failed with status: -25340` and injected nothing: enigo's
`Key::Unicode` path reverse-maps a char → keycode via `UCKeyTranslate`, which
fails in the agent's background process context. Fixed by mapping
`KeyboardEvent.code` (a physical key position) directly to macOS virtual
keycodes and injecting via `enigo.raw()` (`agent/src/input.rs`,
`code_to_macos_keycode`), which posts a CGEvent with no layout lookup.
Re-validated: zero `UCKeyTranslate` errors, text lands.

### Non-interactive bring-up (for a scripted smoke)

The agent's first-run login is an interactive stdin/`rpassword` prompt. To
script the smoke, provision the device via REST and write the agent config
directly instead:

```bash
JWT=$(curl -s -X POST http://127.0.0.1:8080/login \
  -H 'Content-Type: application/json' \
  -d '{"email":"a@b.com","password":"pw123456"}' | jq -r .token)
PAIR=$(curl -s -X POST http://127.0.0.1:8080/devices/pair \
  -H "Authorization: Bearer $JWT" -H 'Content-Type: application/json' \
  -d '{"name":"mac-smoke"}')
# Write {server_url, device_id, device_token} to a JSON file, then point the
# agent at it — RD_AGENT_CONFIG overrides the config path (agent/src/config.rs):
RD_AGENT_CONFIG=/path/to/config.json cargo run --manifest-path agent/Cargo.toml
```

Same-host ICE needs only host/STUN candidates, so coturn is optional for a
localhost smoke (give the server a STUN-only `ICE_SERVERS`).
