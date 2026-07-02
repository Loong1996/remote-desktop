# High-Value UX Pass — Design

**Status:** Approved (brainstorm) — ready for implementation plan
**Date:** 2026-07-02
**Scope:** Four independent UX features for the remote-desktop app, macOS-first被控端.

## Goal

Add four high-value features to make real remote control materially nicer to
use: **special key combos**, a **connection-stats overlay**, **clipboard sync**,
and **live quality/bitrate switching**. Each is independently shippable and
testable.

## Background (current code)

- The web (控制端) is the WebRTC **offerer**; it opens one data channel
  `"input"` carrying only `InputEvent` (`packages/web/src/rtc.ts`,
  `packages/protocol/src/input.ts`). The agent (被控端) is the answerer and
  picks the channel up in `on_data_channel`, forwarding parsed events to the
  injector (`agent/src/webrtc_peer.rs::wire_input`).
- Video: `Openh264Encoder::new(w, h, 3_000_000, fps)` — bitrate is **hardcoded
  at 3 Mbps**. The encode loop lives in `VideoPipeline`
  (`agent/src/video/pipeline.rs`); the encoder is passed in pre-built as
  `Box<dyn VideoEncoder>`.
- The browser already exposes `RTCPeerConnection.getStats()`.
- Precedent for shelling out to a macOS binary already exists
  (`KeepDisplayAwake` spawns `/usr/bin/caffeinate`).

## Global Constraints

- **macOS-first被控端.** Non-macOS agent builds must compile and degrade
  gracefully (clipboard disabled, no panic). Video quality control is
  platform-agnostic (encoder-level).
- **No new agent runtime dependency for clipboard.** Use `pbcopy`/`pbpaste`
  subprocesses on macOS, mirroring the existing `caffeinate` pattern. (arboard
  or similar is deferred to a future cross-platform pass.)
- **Privacy default:** clipboard sync defaults to **off / manual-only**. The
  agent MUST NOT read and broadcast its clipboard unless the web端 has
  explicitly selected a bidirectional-auto mode (via a `clip-mode` message).
- **Tests:** every pure helper gets a unit test; agent OS-touching paths get an
  `#[ignore]`d integration test; new web UI gets an RTL test. Follow existing
  test style (`parseInputEvent`-style parsers, `serial_test` for env-touching
  Rust tests, jsdom docblock + `cleanup()` for RTL).
- **Commit/branch discipline:** English commit messages ending with the
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
  trailer; work on a branch, tests green before commit, merge to main + push.

## Architecture

### The `"control"` data channel

Add a **second** data channel `"control"`, created by the web offerer alongside
`"input"` (same SDP negotiation, no extra round trip). The agent dispatches in
`on_data_channel` by `dc.label`:

- `"input"` → existing `wire_input` (unchanged; `InputEvent` only).
- `"control"` → new `wire_control`, handling `ControlMessage` **bidirectionally**
  (clipboard + quality).

Rationale: keeps high-frequency input isolated from low-frequency/large control
payloads, and keeps the `"input"` path exactly as-is.

### Protocol: `ControlMessage`

New file `packages/protocol/src/control.ts`, exported from `index.ts`, with a
`parseControlMessage(raw): ControlMessage` validator mirroring
`parseInputEvent` (throws on malformed). Tagged union:

```ts
export type ClipMode = "off" | "oneway" | "both";

export interface ClipSet     { t: "clip-set"; text: string; }      // both directions
export interface ClipRequest { t: "clip-request"; }                // web → agent (manual pull)
export interface ClipMode_   { t: "clip-mode"; mode: ClipMode; }   // web → agent (subscription)
export interface Quality     { t: "quality"; bitrateBps: number; } // web → agent

export type ControlMessage = ClipSet | ClipRequest | ClipMode_ | Quality;
```

Validation rules:
- `clip-set.text`: string, length ≤ `262144` (256 KB). Longer → reject (throw);
  senders truncate/skip before sending (see clipboard section).
- `clip-mode.mode`: one of `off | oneway | both`.
- `quality.bitrateBps`: finite number, `250_000 ≤ n ≤ 20_000_000`.

The agent mirrors this as a serde-tagged Rust enum
(`agent/src/protocol.rs` or a new `agent/src/control.rs`) with
`#[serde(tag = "t", rename_all = "kebab-case")]` matching the wire tags
(`clip-set`, `clip-request`, `clip-mode`, `quality`).

### Web `Session` API additions (`rtc.ts`)

```ts
export interface Session {
  sendInput: (ev: InputEvent) => void;
  sendControl: (msg: ControlMessage) => void;      // no-op until control channel open
  getStats: () => Promise<RTCStatsReport | null>;  // null before pc exists
  close: () => void;
}

export interface SessionCallbacks {
  // ...existing...
  onClipboard?: (text: string) => void;   // agent pushed a clip-set to us
  onControlState?: (open: boolean) => void; // control channel open/closed
}
```

`connectSession` creates `pc.createDataChannel("control")`, wires its
`onopen/onclose/onmessage`, routes inbound `clip-set` → `onClipboard`, and
`sendControl` serializes to JSON over that channel.

---

## Feature 1 — Special key combos (web only)

**What:** A toolbar control (button group or small menu) in `SessionView` that
sends predefined chord sequences over the existing `"input"` channel. Needed
because the browser swallows many system chords (Cmd+W, Cmd+Q, Cmd+Space…).

**Pure helper** (`packages/web/src/combos.ts`, unit-tested):

```ts
// Press all codes in order (kdown), then release in reverse order (kup).
export function comboEvents(codes: string[]): InputEvent[]
```

**Combo set** (macOS被控端; each is `{ label, codes }`):
- Spotlight — `["MetaLeft","Space"]`
- App Switcher — `["MetaLeft","Tab"]`
- Mission Control — `["ControlLeft","ArrowUp"]`
- Screenshot region — `["MetaLeft","ShiftLeft","Digit4"]`
- Copy — `["MetaLeft","KeyC"]`
- Paste — `["MetaLeft","KeyV"]`
- Close window — `["MetaLeft","KeyW"]`
- Quit app — `["MetaLeft","KeyQ"]`
- Esc — `["Escape"]`

Buttons `disabled` until `connected`. Clicking emits the sequence via
`session.sendInput`. Each emitted event also appends to the existing "Sent
events" log for visibility.

**Tests:** `comboEvents` order (kdown A,B then kup B,A); RTL test that clicking a
combo button sends the right sequence through the mocked session.

**No protocol/agent change.**

---

## Feature 2 — Connection stats overlay (web only)

**What:** A toggle-able corner HUD showing FPS, bitrate, round-trip latency, and
remote resolution, sampled every ~1s from `pc.getStats()`.

**Pure helper** (`packages/web/src/stats.ts`, unit-tested):

```ts
export interface VideoStats { fps: number; kbps: number; rttMs: number | null; width: number; height: number; }
// `prev` is the previous raw sample used for byte/time deltas → bitrate.
export function parseVideoStats(report: RTCStatsReport, prev: StatsSample | null): { stats: VideoStats; sample: StatsSample }
```

Extraction:
- `inbound-rtp` (kind `"video"`): `framesPerSecond` (fallback to frame-count
  delta / time delta), `frameWidth`/`frameHeight`, `bytesReceived` (delta with
  `timestamp` delta → kbps).
- nominated `candidate-pair` (`state === "succeeded"`, `nominated`):
  `currentRoundTripTime` (seconds → ms); `null` if unavailable (e.g. relayed).

**UI:** a "📊 Stats" toggle button in the header; when on, a small fixed HUD
(top-left, semi-transparent) rendered over the video. A `setInterval(1000)` in
an effect calls `session.getStats()`, runs `parseVideoStats`, stores the sample
for the next delta. Cleared on disconnect / unmount.

**Tests:** `parseVideoStats` fed a hand-built `Map`-based fake `RTCStatsReport`
(two samples) asserts fps/kbps/rtt/resolution and the delta math; RTL test that
toggling shows/hides the HUD.

**No protocol/agent change.**

---

## Feature 3 — Clipboard sync (bidirectional)

**Modes** (web selector, default **off**):

| Mode | Manual buttons | Local→remote auto | Remote→local auto |
|------|:---:|:---:|:---:|
| `off` (manual-only) | ✓ | — | — |
| `oneway` | ✓ | ✓ | — |
| `both` | ✓ | ✓ | ✓ |

Manual buttons are **always** present regardless of mode (they are explicit user
gestures):
- **"Send my clipboard → remote":** `navigator.clipboard.readText()` (button
  click provides the required user gesture) → `sendControl({t:"clip-set",text})`.
- **"Pull remote clipboard":** `sendControl({t:"clip-request"})`; on the reply
  `clip-set`, `navigator.clipboard.writeText(text)`.

Changing the selector sends `sendControl({t:"clip-mode",mode})` so the agent
knows whether to run its own clipboard poller.

**Web auto-sync** (`oneway`/`both`): while the tab has focus, poll
`navigator.clipboard.readText()` every ~800ms; on change, `clip-set` to agent.
In `both`, inbound `clip-set` from the agent is written to the local clipboard.

**Agent side** (`agent/src/clipboard.rs`, macOS via `pbpaste`/`pbcopy`):
- `read_clipboard() -> Result<String>` (`pbpaste`), `write_clipboard(&str)`
  (`pbcopy` via stdin). Non-macOS: return `Err`/no-op so the feature disables
  cleanly.
- On `clip-set`: write to clipboard, record `last_known`.
- On `clip-request`: read clipboard, reply `clip-set` on the control channel.
- On `clip-mode`: store mode. Start the agent clipboard poller **only** when
  `mode == "both"`; stop it otherwise. Poller (~800ms) reads the clipboard;
  on change vs `last_known`, sends `clip-set` on the control channel.

**Echo/loop prevention:** each side keeps `last_known`. When a side *applies* a
received `clip-set`, it sets `last_known` to that text; a poller only sends when
`current != last_known`, updating `last_known` when it sends. Pure decision
helper on both sides, e.g.:

```ts
// returns the text to send, or null if it would be a no-op / echo
export function clipboardToSend(current: string, lastKnown: string, capBytes = 262144): string | null
```

**Size cap:** 256 KB; longer clipboard content is skipped (with a warn), not
truncated.

**Tests:**
- `clipboardToSend` (web) + the Rust equivalent: unchanged → null; changed →
  text; over-cap → null.
- `parseControlMessage` round-trips each variant + rejects over-cap `clip-set`,
  bad `mode`, out-of-range `bitrateBps`.
- Agent `#[ignore]` integration: `write_clipboard("x")` then `read_clipboard()`
  == `"x"` (macOS, needs a session).
- RTL: mode selector sends `clip-mode`; "Send my clipboard" reads
  `navigator.clipboard` (mocked) and sends `clip-set`; inbound `clip-set`
  (`both` mode) writes local clipboard (mocked).

---

## Feature 4 — Live quality / bitrate switching

**What:** UI presets — **流畅 1.5 Mbps / 均衡 3 Mbps (default) / 高清 6 Mbps** —
that change the encoder bitrate **without dropping the connection**.

**Encoder** (`agent/src/video/`): add to the `VideoEncoder` trait

```rust
/// Change the target bitrate live. Default: no-op.
fn set_bitrate(&mut self, _bitrate_bps: u32) {}
```

`Openh264Encoder` stores `fps` (and current bitrate) and implements
`set_bitrate` by **rebuilding its inner `openh264::Encoder`** with the new
`BitRate`. A rebuilt encoder emits a fresh SPS/PPS+IDR on its next frame, so the
browser decoder re-syncs seamlessly (resolution unchanged → cheap). Set an
internal `force_idr_next` flag if needed to guarantee the keyframe.

**Pipeline** (`agent/src/video/pipeline.rs`): `VideoPipeline::start` takes an
extra `bitrate_rx: std::sync::mpsc::Receiver<u32>`. Each loop iteration:
`while let Ok(bps) = bitrate_rx.try_recv() { encoder.set_bitrate(bps); }` before
encoding the next frame.

**Wiring** (`webrtc_peer.rs::build`): create `let (bitrate_tx, bitrate_rx) =
mpsc::channel::<u32>();`. Pass `bitrate_rx` into `VideoPipeline::start`; move
`bitrate_tx` into the control-channel dispatcher. On `quality{bitrateBps}`, the
dispatcher clamps to `[250_000, 20_000_000]` and `bitrate_tx.send(bps)`. The
channel buffers if the control message arrives before the pipeline exists.

**Web:** a preset selector in the header; selecting sends
`sendControl({t:"quality",bitrateBps})`. Default label reflects 均衡/3 Mbps.

**Tests:**
- `Openh264Encoder::set_bitrate` then `encode(..., false)` still yields a
  keyframe (SPS+PPS+IDR present) — the rebuild path.
- Pipeline drains bitrate requests: a fake `VideoEncoder` records `set_bitrate`
  calls; push two values on the channel, run enough frames, assert the encoder
  saw both in order.
- `parseControlMessage` quality range validation (covered above).
- RTL: selecting a preset sends the right `quality` message.

---

## File map

**Create:**
- `packages/protocol/src/control.ts` — `ControlMessage` + `parseControlMessage`
- `packages/protocol/src/control.test.ts`
- `packages/web/src/combos.ts` + `combos.test.ts`
- `packages/web/src/stats.ts` + `stats.test.ts`
- `packages/web/src/clipboard.ts` (`clipboardToSend` + mode types) + test
- `agent/src/clipboard.rs` (pbcopy/pbpaste, `last_known`, `clipboard_to_send`)
- `agent/src/control.rs` (Rust `ControlMessage` enum + dispatcher) *(or fold
  into `webrtc_peer.rs` if small)*

**Modify:**
- `packages/protocol/src/index.ts` — export control
- `packages/web/src/rtc.ts` — `"control"` channel, `sendControl`, `getStats`,
  `onClipboard`, `onControlState`
- `packages/web/src/pages/SessionView.tsx` — combos bar, stats HUD + toggle,
  clipboard controls + mode selector, quality selector
- `packages/web/src/pages/SessionView.test.tsx` — new UI tests
- `agent/src/webrtc_peer.rs` — dispatch `"control"` channel, bitrate mpsc wiring
- `agent/src/video/mod.rs` — `VideoEncoder::set_bitrate` default method
- `agent/src/video/openh264_encoder.rs` — store fps, implement `set_bitrate`
- `agent/src/video/pipeline.rs` — `bitrate_rx` param + drain loop
- `agent/src/protocol.rs` — Rust control message types (if not new file)

## Sequencing (independence)

Order by risk/independence; each is a shippable slice:
1. **Special key combos** (pure web, zero risk) — also validates the toolbar
   layout that later features hang buttons on.
2. **Stats overlay** (pure web) — helps diagnose the bitrate feature later.
3. **`control` channel + protocol** (plumbing both #4 and clipboard depend on).
4. **Quality/bitrate** (encoder `set_bitrate` + pipeline drain + control wiring).
5. **Clipboard sync** (agent pbcopy/pbpaste + modes + echo prevention).

## Out of scope (YAGNI)

- Clipboard images/files (text only).
- Cross-platform被控端 clipboard (macOS `pbcopy`/`pbpaste` only; arboard later).
- Live **resolution** switching (bitrate only).
- Adaptive/automatic bitrate (manual presets only).
- Persisting the user's mode/quality/stats preferences across sessions.
