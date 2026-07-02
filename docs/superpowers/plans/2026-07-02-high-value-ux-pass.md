# High-Value UX Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add four independently-shippable UX features to the remote-desktop app — special key combos, a connection-stats overlay, clipboard sync (three modes), and live quality/bitrate switching.

**Architecture:** Combos and the stats HUD are pure web additions over the existing `"input"` data channel and `pc.getStats()`. Clipboard and quality ride a new bidirectional `"control"` data channel carrying a `ControlMessage` tagged union (mirrored TS↔Rust). Quality changes rebuild the openh264 encoder in-place inside the running `VideoPipeline`; clipboard uses macOS `pbcopy`/`pbpaste` subprocesses with per-side echo suppression.

**Tech Stack:** TypeScript (@rd/protocol, @rd/web / React + Vitest + RTL/jsdom), Rust agent (webrtc-rs 0.11, openh264 0.9, serde_json, tokio).

**Spec:** `docs/superpowers/specs/2026-07-02-high-value-ux-pass-design.md`

## Global Constraints

- **macOS-first被控端.** Non-macOS agent builds MUST compile and degrade gracefully (clipboard disabled, no panic). Quality control is platform-agnostic.
- **No new agent runtime dependency for clipboard** — use `pbcopy`/`pbpaste` subprocesses (mirrors the existing `caffeinate` pattern).
- **Privacy default:** clipboard sync defaults to `off` (manual-only). The agent MUST NOT read+broadcast its clipboard unless the web端 selected `both` (via a `clip-mode` message).
- **Wire tags are exact and shared:** control message tags are `clip-set`, `clip-request`, `clip-mode`, `quality`; clip modes are `off`, `oneway`, `both`; `quality` carries camelCase `bitrateBps`. TS `parseControlMessage` and the Rust serde enum MUST agree byte-for-byte.
- **Limits:** clip-set text cap ≈ 256 KB (`262144`); quality bitrate clamp `[250000, 20000000]`; default bitrate `3000000` (均衡).
- **Tests:** every pure helper gets a unit test; agent OS-touching paths get an `#[ignore]`d integration test; new web UI gets an RTL test. Live `connectSession` WebRTC wiring is not unit-tested (jsdom has no WebRTC) — it is verified by typecheck + build + manual e2e, consistent with the existing `rtc.ts` convention.
- **Commit discipline:** English commit messages ending with the trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`; work on a branch; tests green before each commit.

## File Structure

**Create:**
- `packages/web/src/combos.ts` (+ `combos.test.ts`) — chord → InputEvent[] helper + combo list
- `packages/web/src/stats.ts` (+ `stats.test.ts`) — `parseVideoStats`
- `packages/web/src/clipboard.ts` (+ `clipboard.test.ts`) — `clipboardToSend` + re-export `ClipMode`
- `packages/protocol/src/control.ts` (+ `control.test.ts`) — `ControlMessage` + `parseControlMessage`
- `agent/src/control.rs` — Rust `ControlMessage`/`ClipMode` serde enums
- `agent/src/clipboard.rs` — `pbcopy`/`pbpaste` + `clipboard_to_send`

**Modify:**
- `packages/protocol/src/index.ts` — export `./control.js`
- `packages/web/src/rtc.ts` — `"control"` channel, `sendControl`, `getStats`, `onClipboard`, `onControlState`
- `packages/web/src/pages/SessionView.tsx` (+ `SessionView.test.tsx`) — combos bar, stats HUD, quality selector, clipboard controls
- `agent/src/video/mod.rs` — `VideoEncoder::set_bitrate` default method
- `agent/src/video/openh264_encoder.rs` — store fps/bitrate, implement `set_bitrate` (rebuild)
- `agent/src/video/pipeline.rs` — `bitrate_rx` param + drain loop
- `agent/src/webrtc_peer.rs` — dispatch `"control"` channel, bitrate mpsc wiring, `wire_control`
- `agent/src/main.rs` (or `lib.rs`) — `mod control; mod clipboard;`

## Task → Spec-slice map

1. Combos → slice 1. 2. Stats → slice 2. 3–4. Protocol + web control channel → slice 3. 5. Rust control enum → slice 3. 6. Encoder/pipeline bitrate → slice 4. 7. Agent clipboard module → slice 5. 8. Agent control dispatch (quality + clipboard) → slices 4+5. 9. Quality UI → slice 4. 10. Clipboard UI → slice 5.

Tasks are ordered so each task's dependencies already exist. Start on a branch `feat-ux-pass` off `main`.

---

### Task 1: Special key combos (web)

**Files:**
- Create: `packages/web/src/combos.ts`, `packages/web/src/combos.test.ts`
- Modify: `packages/web/src/pages/SessionView.tsx`, `packages/web/src/pages/SessionView.test.tsx`

**Interfaces:**
- Consumes: `InputEvent` from `@rd/protocol`; `emit(ev)` and `connected` already in `SessionView`.
- Produces: `comboEvents(codes: string[]): InputEvent[]`; `COMBOS: { label: string; codes: string[] }[]`.

- [ ] **Step 1: Write the failing unit test**

`packages/web/src/combos.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { comboEvents } from "./combos.js";

describe("comboEvents", () => {
  it("presses in order then releases in reverse", () => {
    expect(comboEvents(["MetaLeft", "KeyC"])).toEqual([
      { t: "kdown", code: "MetaLeft" },
      { t: "kdown", code: "KeyC" },
      { t: "kup", code: "KeyC" },
      { t: "kup", code: "MetaLeft" },
    ]);
  });
  it("handles a single key", () => {
    expect(comboEvents(["Escape"])).toEqual([
      { t: "kdown", code: "Escape" },
      { t: "kup", code: "Escape" },
    ]);
  });
  it("returns empty for empty input", () => {
    expect(comboEvents([])).toEqual([]);
  });
});
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `npm test -w @rd/web -- combos`
Expected: FAIL — cannot find module `./combos.js`.

- [ ] **Step 3: Implement `combos.ts`**

`packages/web/src/combos.ts`:
```ts
import type { InputEvent } from "@rd/protocol";

export interface Combo {
  label: string;
  codes: string[]; // KeyboardEvent.code values, held together
}

/** macOS被控端 chords the browser would otherwise swallow. */
export const COMBOS: Combo[] = [
  { label: "Spotlight", codes: ["MetaLeft", "Space"] },
  { label: "App Switcher", codes: ["MetaLeft", "Tab"] },
  { label: "Mission Control", codes: ["ControlLeft", "ArrowUp"] },
  { label: "Screenshot", codes: ["MetaLeft", "ShiftLeft", "Digit4"] },
  { label: "Copy", codes: ["MetaLeft", "KeyC"] },
  { label: "Paste", codes: ["MetaLeft", "KeyV"] },
  { label: "Close Window", codes: ["MetaLeft", "KeyW"] },
  { label: "Quit App", codes: ["MetaLeft", "KeyQ"] },
  { label: "Esc", codes: ["Escape"] },
];

/** Press all `codes` in order (kdown), then release in reverse order (kup). */
export function comboEvents(codes: string[]): InputEvent[] {
  const out: InputEvent[] = [];
  for (const code of codes) out.push({ t: "kdown", code });
  for (let i = codes.length - 1; i >= 0; i--) out.push({ t: "kup", code: codes[i] });
  return out;
}
```

- [ ] **Step 4: Run the unit test — expect PASS**

Run: `npm test -w @rd/web -- combos`
Expected: PASS (3 tests).

- [ ] **Step 5: Add the combo bar to `SessionView.tsx`**

Add the import near the other imports:
```ts
import { COMBOS, comboEvents } from "../combos.js";
```
Add a callback next to `emit` (after the `emit` definition, ~line 106):
```ts
const sendCombo = useCallback(
  (codes: string[]) => {
    for (const ev of comboEvents(codes)) emit(ev);
  },
  [emit],
);
```
Insert the bar just below the `<p>Click the panel…</p>` line (before the `{error && …}`):
```tsx
<div
  data-testid="combo-bar"
  style={{ display: connected ? "flex" : "none", flexWrap: "wrap", gap: 6, margin: "8px 0" }}
>
  {COMBOS.map((c) => (
    <button
      key={c.label}
      data-testid={`combo-${c.label}`}
      disabled={!connected}
      onClick={() => sendCombo(c.codes)}
      style={{ fontSize: 12 }}
    >
      {c.label}
    </button>
  ))}
</div>
```

- [ ] **Step 6: Add the RTL test**

Append to `packages/web/src/pages/SessionView.test.tsx` inside the existing `describe` (the mock session already exposes `sendInput`):
```ts
it("sends a chord as kdown…kup in reverse on combo click", () => {
  render(<SessionView token="t" device={device} onExit={() => {}} />);
  act(() => h.opts!.onState("connected"));
  fireEvent.click(screen.getByTestId("combo-Copy"));
  const sent = h.session.sendInput.mock.calls.map((c) => c[0]);
  expect(sent).toEqual([
    { t: "kdown", code: "MetaLeft" },
    { t: "kdown", code: "KeyC" },
    { t: "kup", code: "KeyC" },
    { t: "kup", code: "MetaLeft" },
  ]);
});
```

- [ ] **Step 7: Run web tests + typecheck + build**

Run: `npm test -w @rd/web && npm run -w @rd/web typecheck && npm run -w @rd/web build`
Expected: all PASS.

- [ ] **Step 8: Commit**

```bash
git add packages/web/src/combos.ts packages/web/src/combos.test.ts packages/web/src/pages/SessionView.tsx packages/web/src/pages/SessionView.test.tsx
git commit -m "feat(web): special key combo bar for browser-swallowed chords

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Connection stats overlay (web)

**Files:**
- Create: `packages/web/src/stats.ts`, `packages/web/src/stats.test.ts`
- Modify: `packages/web/src/rtc.ts` (add `getStats` to `Session`), `packages/web/src/pages/SessionView.tsx`, `packages/web/src/pages/SessionView.test.tsx`

**Interfaces:**
- Consumes: `RTCPeerConnection.getStats()`.
- Produces: `parseVideoStats(report, prev): { stats: VideoStats; sample: StatsSample }`; `Session.getStats(): Promise<RTCStatsReport | null>`.

- [ ] **Step 1: Write the failing unit test**

`packages/web/src/stats.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { parseVideoStats } from "./stats.js";

function fakeReport(entries: Record<string, unknown>[]): RTCStatsReport {
  const m = new Map<string, unknown>();
  entries.forEach((e, i) => m.set(String(i), e));
  return m as unknown as RTCStatsReport;
}

describe("parseVideoStats", () => {
  it("reads fps, resolution, and rtt from a single sample", () => {
    const report = fakeReport([
      { type: "inbound-rtp", kind: "video", framesPerSecond: 30, frameWidth: 1280, frameHeight: 720, bytesReceived: 1000, timestamp: 1000 },
      { type: "candidate-pair", state: "succeeded", nominated: true, currentRoundTripTime: 0.042 },
    ]);
    const { stats, sample } = parseVideoStats(report, null);
    expect(stats.fps).toBe(30);
    expect(stats.width).toBe(1280);
    expect(stats.height).toBe(720);
    expect(stats.rttMs).toBe(42);
    expect(stats.kbps).toBe(0); // no prev sample → no bitrate yet
    expect(sample).toEqual({ bytesReceived: 1000, timestamp: 1000 });
  });

  it("computes kbps from the delta vs the previous sample", () => {
    const report = fakeReport([
      { type: "inbound-rtp", kind: "video", framesPerSecond: 30, frameWidth: 1280, frameHeight: 720, bytesReceived: 126000, timestamp: 2000 },
    ]);
    // 125000 bytes over 1.0s = 1,000,000 bits/s = 1000 kbps
    const { stats } = parseVideoStats(report, { bytesReceived: 1000, timestamp: 1000 });
    expect(stats.kbps).toBe(1000);
  });

  it("returns null rtt when there is no nominated pair", () => {
    const report = fakeReport([
      { type: "inbound-rtp", kind: "video", framesPerSecond: 24, bytesReceived: 0, timestamp: 0 },
    ]);
    expect(parseVideoStats(report, null).stats.rttMs).toBeNull();
  });
});
```

- [ ] **Step 2: Run it — expect FAIL**

Run: `npm test -w @rd/web -- stats`
Expected: FAIL — cannot find `./stats.js`.

- [ ] **Step 3: Implement `stats.ts`**

`packages/web/src/stats.ts`:
```ts
export interface VideoStats {
  fps: number;
  kbps: number;
  rttMs: number | null;
  width: number;
  height: number;
}

/** The raw fields needed to compute a bitrate delta between two polls. */
export interface StatsSample {
  bytesReceived: number;
  timestamp: number; // ms
}

/** Extract a display-ready VideoStats from a getStats() report. `prev` is the
 *  sample from the previous poll, used for the byte/time delta → bitrate. */
export function parseVideoStats(
  report: RTCStatsReport,
  prev: StatsSample | null,
): { stats: VideoStats; sample: StatsSample } {
  let inbound: Record<string, unknown> | null = null;
  let pair: Record<string, unknown> | null = null;
  report.forEach((s: unknown) => {
    const r = s as Record<string, unknown>;
    if (r.type === "inbound-rtp" && r.kind === "video") inbound = r;
    if (r.type === "candidate-pair" && r.state === "succeeded" && r.nominated === true) pair = r;
  });

  const num = (o: Record<string, unknown> | null, k: string): number =>
    typeof o?.[k] === "number" ? (o[k] as number) : 0;

  const bytesReceived = num(inbound, "bytesReceived");
  const timestamp = num(inbound, "timestamp");
  const sample: StatsSample = { bytesReceived, timestamp };

  let kbps = 0;
  if (prev && timestamp > prev.timestamp) {
    const bits = (bytesReceived - prev.bytesReceived) * 8;
    const seconds = (timestamp - prev.timestamp) / 1000;
    if (seconds > 0) kbps = Math.max(0, Math.round(bits / seconds / 1000));
  }

  const rttRaw = pair && typeof (pair as Record<string, unknown>).currentRoundTripTime === "number"
    ? ((pair as Record<string, unknown>).currentRoundTripTime as number)
    : null;

  return {
    stats: {
      fps: Math.round(num(inbound, "framesPerSecond")),
      kbps,
      rttMs: rttRaw === null ? null : Math.round(rttRaw * 1000),
      width: num(inbound, "frameWidth"),
      height: num(inbound, "frameHeight"),
    },
    sample,
  };
}
```

- [ ] **Step 4: Run the unit test — expect PASS**

Run: `npm test -w @rd/web -- stats`
Expected: PASS (3 tests).

- [ ] **Step 5: Add `getStats` to the `Session` interface + implementation in `rtc.ts`**

In `packages/web/src/rtc.ts`, extend the `Session` interface (after `sendInput`):
```ts
export interface Session {
  /** Send an InputEvent over the "input" data channel (no-op until open). */
  sendInput: (ev: InputEvent) => void;
  /** Snapshot the peer connection's WebRTC stats (null before the pc exists). */
  getStats: () => Promise<RTCStatsReport | null>;
  /** Tear down the data channel, peer connection, and WebSocket. */
  close: () => void;
}
```
Extend the returned object at the bottom of `connectSession` (the `return { sendInput, close }` block):
```ts
  return {
    sendInput(ev: InputEvent) {
      if (channel && channel.readyState === "open") {
        channel.send(JSON.stringify(ev));
      }
    },
    async getStats() {
      return pc ? pc.getStats() : null;
    },
    close,
  };
```

- [ ] **Step 6: Add the HUD + toggle to `SessionView.tsx`**

Add import:
```ts
import { parseVideoStats, type StatsSample, type VideoStats } from "../stats.js";
```
Add state near the other `useState` hooks:
```ts
const [showStats, setShowStats] = useState(false);
const [stats, setStats] = useState<VideoStats | null>(null);
const statsSample = useRef<StatsSample | null>(null);
```
Add a polling effect (after the fullscreen effects):
```ts
// Poll WebRTC stats ~1/s while the HUD is on and connected.
useEffect(() => {
  if (!connected || !showStats) {
    setStats(null);
    statsSample.current = null;
    return;
  }
  const id = setInterval(() => {
    void sessionRef.current?.getStats().then((report) => {
      if (!report) return;
      const { stats: s, sample } = parseVideoStats(report, statsSample.current);
      statsSample.current = sample;
      setStats(s);
    });
  }, 1000);
  return () => clearInterval(id);
}, [connected, showStats]);
```
Add a toggle button in the header `<span>` (next to `maximize-btn`):
```tsx
<button onClick={() => setShowStats((v) => !v)} disabled={!connected} data-testid="stats-btn">
  {showStats ? "Hide stats" : "📊 Stats"}
</button>
```
Render the HUD (place it right after the `<video>` element, before the `{isMaximized && …}` block):
```tsx
{showStats && (
  <div
    data-testid="stats-hud"
    style={{
      position: "fixed", top: 12, left: 12, zIndex: 1001,
      padding: "6px 10px", borderRadius: 6, background: "rgba(0,0,0,0.6)",
      color: "#fff", fontFamily: "ui-monospace, monospace", fontSize: 12,
    }}
  >
    {stats
      ? `${stats.fps} fps · ${stats.kbps} kbps · ${stats.rttMs ?? "?"} ms · ${stats.width}×${stats.height}`
      : "sampling…"}
  </div>
)}
```

- [ ] **Step 7: Extend the RTL mock + add a toggle test**

In `SessionView.test.tsx`, extend the hoisted mock session to include `getStats`:
```ts
const h = vi.hoisted(() => ({
  opts: null as null | { onState: (s: string) => void },
  session: { close: vi.fn(), sendInput: vi.fn(), getStats: vi.fn().mockResolvedValue(null) },
}));
```
Add the test:
```ts
it("toggles the stats HUD", () => {
  render(<SessionView token="t" device={device} onExit={() => {}} />);
  act(() => h.opts!.onState("connected"));
  expect(screen.queryByTestId("stats-hud")).toBeNull();
  fireEvent.click(screen.getByTestId("stats-btn"));
  expect(screen.getByTestId("stats-hud")).toBeTruthy();
  fireEvent.click(screen.getByTestId("stats-btn"));
  expect(screen.queryByTestId("stats-hud")).toBeNull();
});
```

- [ ] **Step 8: Run web tests + typecheck + build**

Run: `npm test -w @rd/web && npm run -w @rd/web typecheck && npm run -w @rd/web build`
Expected: all PASS.

- [ ] **Step 9: Commit**

```bash
git add packages/web/src/stats.ts packages/web/src/stats.test.ts packages/web/src/rtc.ts packages/web/src/pages/SessionView.tsx packages/web/src/pages/SessionView.test.tsx
git commit -m "feat(web): connection stats HUD (fps/bitrate/rtt/resolution)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Protocol `ControlMessage` (TypeScript)

**Files:**
- Create: `packages/protocol/src/control.ts`, `packages/protocol/src/control.test.ts`
- Modify: `packages/protocol/src/index.ts`

**Interfaces:**
- Produces: `ControlMessage`, `ClipMode`, `parseControlMessage(raw): ControlMessage`, and constants `CLIP_MAX_BYTES`, `QUALITY_MIN_BPS`, `QUALITY_MAX_BPS`.

- [ ] **Step 1: Write the failing test**

`packages/protocol/src/control.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { parseControlMessage, CLIP_MAX_BYTES } from "./control.js";

describe("parseControlMessage", () => {
  it("parses each variant", () => {
    expect(parseControlMessage({ t: "clip-set", text: "hi" })).toEqual({ t: "clip-set", text: "hi" });
    expect(parseControlMessage({ t: "clip-request" })).toEqual({ t: "clip-request" });
    expect(parseControlMessage({ t: "clip-mode", mode: "both" })).toEqual({ t: "clip-mode", mode: "both" });
    expect(parseControlMessage({ t: "quality", bitrateBps: 3_000_000 })).toEqual({ t: "quality", bitrateBps: 3_000_000 });
  });

  it("rejects malformed messages", () => {
    expect(() => parseControlMessage(null)).toThrow();
    expect(() => parseControlMessage({ t: "nope" })).toThrow();
    expect(() => parseControlMessage({ t: "clip-set", text: 5 })).toThrow();
    expect(() => parseControlMessage({ t: "clip-set", text: "x".repeat(CLIP_MAX_BYTES + 1) })).toThrow();
    expect(() => parseControlMessage({ t: "clip-mode", mode: "sideways" })).toThrow();
    expect(() => parseControlMessage({ t: "quality", bitrateBps: 10 })).toThrow();
    expect(() => parseControlMessage({ t: "quality", bitrateBps: 99_000_000 })).toThrow();
  });
});
```

- [ ] **Step 2: Run it — expect FAIL**

Run: `npm test -w @rd/protocol -- control`
Expected: FAIL — cannot find `./control.js`.

- [ ] **Step 3: Implement `control.ts`**

`packages/protocol/src/control.ts`:
```ts
export type ClipMode = "off" | "oneway" | "both";

export interface ClipSet { t: "clip-set"; text: string; }
export interface ClipRequest { t: "clip-request"; }
export interface ClipModeMsg { t: "clip-mode"; mode: ClipMode; }
export interface Quality { t: "quality"; bitrateBps: number; }

export type ControlMessage = ClipSet | ClipRequest | ClipModeMsg | Quality;

// Approximate 256 KB cap on clip-set text (measured in UTF-16 code units on
// the web side; the agent applies the same numeric cap in bytes — both are
// guards against pathological payloads, not exact-byte contracts).
export const CLIP_MAX_BYTES = 262144;
export const QUALITY_MIN_BPS = 250_000;
export const QUALITY_MAX_BPS = 20_000_000;

const CLIP_MODES = new Set<ClipMode>(["off", "oneway", "both"]);

function isObj(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null;
}

export function parseControlMessage(raw: unknown): ControlMessage {
  if (!isObj(raw)) throw new Error("control message must be an object");
  switch (raw.t) {
    case "clip-set": {
      if (typeof raw.text !== "string") throw new Error("clip-set.text must be a string");
      if (raw.text.length > CLIP_MAX_BYTES) throw new Error("clip-set.text too large");
      return { t: "clip-set", text: raw.text };
    }
    case "clip-request":
      return { t: "clip-request" };
    case "clip-mode": {
      if (typeof raw.mode !== "string" || !CLIP_MODES.has(raw.mode as ClipMode))
        throw new Error("invalid clip-mode.mode");
      return { t: "clip-mode", mode: raw.mode as ClipMode };
    }
    case "quality": {
      const n = raw.bitrateBps;
      if (typeof n !== "number" || Number.isNaN(n) || n < QUALITY_MIN_BPS || n > QUALITY_MAX_BPS)
        throw new Error("invalid quality.bitrateBps");
      return { t: "quality", bitrateBps: n };
    }
    default:
      throw new Error(`unknown control type: ${String(raw.t)}`);
  }
}
```

- [ ] **Step 4: Export from `index.ts`**

Edit `packages/protocol/src/index.ts` to add the line:
```ts
export * from "./control.js";
```

- [ ] **Step 5: Run tests + typecheck — expect PASS**

Run: `npm test -w @rd/protocol -- control && npm run -w @rd/protocol typecheck`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add packages/protocol/src/control.ts packages/protocol/src/control.test.ts packages/protocol/src/index.ts
git commit -m "feat(protocol): ControlMessage (clipboard + quality) with parser

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Web `"control"` data channel wiring (`rtc.ts`)

**Files:**
- Modify: `packages/web/src/rtc.ts`

**Interfaces:**
- Consumes: `ControlMessage`, `parseControlMessage` from `@rd/protocol`.
- Produces: `Session.sendControl(msg: ControlMessage): void`; `SessionCallbacks.onClipboard?(text: string)`, `SessionCallbacks.onControlState?(open: boolean)`.

> **Verification note:** the live channel is exercised by manual e2e (jsdom has no WebRTC). This task is verified by typecheck + web build; downstream Tasks 9–10 add the RTL coverage that drives `sendControl`/`onClipboard` through the mocked session.

- [ ] **Step 1: Import the control types**

At the top of `packages/web/src/rtc.ts`, extend the `@rd/protocol` import to add `type ControlMessage` and the runtime `parseControlMessage`:
```ts
import {
  parseSignalingMessage,
  parseControlMessage,
  type Connect,
  type Sdp,
  type Ice,
  type IceServer,
  type InputEvent,
  type ControlMessage,
  type MouseButton,
} from "@rd/protocol";
```

- [ ] **Step 2: Extend `SessionCallbacks` and `Session`**

Add to `SessionCallbacks`:
```ts
  /** The agent pushed a clip-set to us (manual pull reply, or both-mode auto). */
  onClipboard?: (text: string) => void;
  /** The "control" data channel opened (true) or closed (false). */
  onControlState?: (open: boolean) => void;
```
Add to `Session`:
```ts
  /** Send a ControlMessage over the "control" data channel (no-op until open). */
  sendControl: (msg: ControlMessage) => void;
```

- [ ] **Step 3: Create + wire the control channel**

In `connectSession`, destructure the new callbacks:
```ts
const { onState, onError, onRemoteStream, onClipboard, onControlState } = callbacks;
```
Add a `control` variable next to `channel`:
```ts
let control: RTCDataChannel | null = null;
```
Inside `startPeer`, after the existing `channel = pc.createDataChannel("input");` block, add:
```ts
control = pc.createDataChannel("control");
control.onopen = () => onControlState?.(true);
control.onclose = () => onControlState?.(false);
control.onmessage = (ev) => {
  let raw: unknown;
  try {
    raw = JSON.parse(typeof ev.data === "string" ? ev.data : String(ev.data));
  } catch {
    return;
  }
  let msg: ControlMessage;
  try {
    msg = parseControlMessage(raw);
  } catch {
    return;
  }
  if (msg.t === "clip-set") onClipboard?.(msg.text);
};
```
In `close()`, close and null the control channel alongside `channel`:
```ts
try {
  control?.close();
} catch {
  /* ignore */
}
```
and after `channel = null;` add `control = null;`.

- [ ] **Step 4: Add `sendControl` to the returned object**

```ts
    sendControl(msg: ControlMessage) {
      if (control && control.readyState === "open") {
        control.send(JSON.stringify(msg));
      }
    },
```

- [ ] **Step 5: Typecheck + build**

Run: `npm run -w @rd/web typecheck && npm run -w @rd/web build`
Expected: PASS. (No new unit test — see verification note.)

- [ ] **Step 6: Commit**

```bash
git add packages/web/src/rtc.ts
git commit -m "feat(web): bidirectional control data channel (sendControl/onClipboard)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Rust `ControlMessage` enum (agent)

**Files:**
- Create: `agent/src/control.rs`
- Modify: `agent/src/main.rs` (add `mod control;`)

**Interfaces:**
- Produces: `crate::control::ControlMessage` (`ClipSet{text}`, `ClipRequest`, `ClipMode{mode}`, `Quality{bitrate_bps}`) and `crate::control::ClipMode` (`Off|Oneway|Both`). Wire tags MUST match Task 3.

- [ ] **Step 1: Write the failing test (inside the new module)**

`agent/src/control.rs`:
```rust
use serde::{Deserialize, Serialize};

/// Control-channel messages (clipboard + quality), mirrored from
/// packages/protocol/src/control.ts. The serde tag + kebab-case renaming must
/// match the TypeScript wire tags byte-for-byte.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "kebab-case")]
pub enum ControlMessage {
    ClipSet { text: String },
    ClipRequest,
    ClipMode { mode: ClipMode },
    Quality {
        #[serde(rename = "bitrateBps")]
        bitrate_bps: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClipMode {
    Off,
    Oneway,
    Both,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_variant_from_web_tags() {
        let cases = [
            (r#"{"t":"clip-set","text":"hi"}"#, ControlMessage::ClipSet { text: "hi".into() }),
            (r#"{"t":"clip-request"}"#, ControlMessage::ClipRequest),
            (r#"{"t":"clip-mode","mode":"both"}"#, ControlMessage::ClipMode { mode: ClipMode::Both }),
            (r#"{"t":"quality","bitrateBps":3000000}"#, ControlMessage::Quality { bitrate_bps: 3_000_000 }),
        ];
        for (json, want) in cases {
            let got: ControlMessage = serde_json::from_str(json).unwrap();
            assert_eq!(got, want);
        }
    }

    #[test]
    fn serializes_clip_mode_with_web_tags() {
        let s = serde_json::to_string(&ControlMessage::ClipMode { mode: ClipMode::Oneway }).unwrap();
        assert_eq!(s, r#"{"t":"clip-mode","mode":"oneway"}"#);
    }
}
```

- [ ] **Step 2: Register the module**

In `agent/src/main.rs`, add alongside the other `mod` declarations:
```rust
mod control;
```

- [ ] **Step 3: Run it — expect PASS**

Run: `cargo test --manifest-path agent/Cargo.toml control::`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add agent/src/control.rs agent/src/main.rs
git commit -m "feat(agent): Rust ControlMessage enum mirroring the web protocol

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Live bitrate — encoder `set_bitrate` + pipeline drain (agent)

**Files:**
- Modify: `agent/src/video/mod.rs`, `agent/src/video/openh264_encoder.rs`, `agent/src/video/pipeline.rs`, `agent/src/webrtc_peer.rs` (callsite only)
- Also update any existing `VideoPipeline::start` callers in `agent/tests/` (add a bitrate receiver arg).

**Interfaces:**
- Produces: `VideoEncoder::set_bitrate(&mut self, bitrate_bps: u32)` (default no-op); `VideoPipeline::start(..., bitrate_rx: std::sync::mpsc::Receiver<u32>)` (new last param).
- Consumes: nothing new.

- [ ] **Step 1: Add the default trait method (`mod.rs`)**

In `agent/src/video/mod.rs`, extend the `VideoEncoder` trait:
```rust
/// Encodes I420 frames to H.264 Annex-B. `force_idr` requests a keyframe.
pub trait VideoEncoder: Send {
    fn encode(&mut self, frame: &I420, force_idr: bool) -> anyhow::Result<EncodedSample>;
    /// Change the target bitrate for subsequent frames. Default: no-op.
    fn set_bitrate(&mut self, _bitrate_bps: u32) {}
}
```

- [ ] **Step 2: Write the failing encoder test**

Add to the `tests` module in `agent/src/video/openh264_encoder.rs`:
```rust
#[test]
fn set_bitrate_rebuild_still_emits_keyframe() {
    let mut enc = Openh264Encoder::new(64, 64, 1_000_000, 30.0).unwrap();
    let _ = enc.encode(&gray_i420(64, 64), true).unwrap();
    enc.set_bitrate(4_000_000);
    // force_idr=false, but the rebuilt encoder must still emit SPS+PPS+IDR.
    let s = enc.encode(&gray_i420(64, 64), false).unwrap();
    assert!(s.keyframe);
    let types = nal_types(&s.data);
    assert!(types.contains(&7) && types.contains(&8) && types.contains(&5), "got {types:?}");
}
```

- [ ] **Step 3: Run it — expect FAIL**

Run: `cargo test --manifest-path agent/Cargo.toml set_bitrate_rebuild`
Expected: FAIL — `set_bitrate` currently is the no-op default, so no keyframe is forced and the assertion on NAL 5/7/8 fails (or `keyframe` is false).

- [ ] **Step 4: Implement `set_bitrate` (rebuild) in `openh264_encoder.rs`**

Replace the struct + impls with:
```rust
/// Software H.264 encoder (openh264). Owns the encoder, per-frame duration, and
/// the parameters needed to rebuild the encoder when the bitrate changes.
pub struct Openh264Encoder {
    encoder: Encoder,
    frame_dur: Duration,
    fps: f32,
    bitrate_bps: u32,
    force_idr_next: bool,
}

impl Openh264Encoder {
    pub fn new(width: u32, height: u32, bitrate_bps: u32, fps: f32) -> anyhow::Result<Self> {
        let _ = (width, height); // resolution is taken from the YUVSource at encode time
        let encoder = Self::build_encoder(bitrate_bps, fps)?;
        Ok(Self {
            encoder,
            frame_dur: Duration::from_secs_f32(1.0 / fps),
            fps,
            bitrate_bps,
            force_idr_next: false,
        })
    }

    fn build_encoder(bitrate_bps: u32, fps: f32) -> anyhow::Result<Encoder> {
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bitrate_bps))
            .max_frame_rate(FrameRate::from_hz(fps));
        Ok(Encoder::with_api_config(openh264::OpenH264API::from_source(), config)?)
    }
}

impl VideoEncoder for Openh264Encoder {
    fn encode(&mut self, frame: &I420, force_idr: bool) -> anyhow::Result<EncodedSample> {
        let idr = force_idr || self.force_idr_next;
        if idr {
            self.encoder.force_intra_frame();
        }
        self.force_idr_next = false;
        let yuv = YUVSlices::new(
            (&frame.y, &frame.u, &frame.v),
            (frame.width, frame.height),
            (frame.y_stride, frame.uv_stride, frame.uv_stride),
        );
        let bitstream = self.encoder.encode(&yuv)?;
        let data = bitstream.to_vec();
        // openh264 emits SPS+PPS with each IDR; treat a forced-IDR frame as keyframe.
        Ok(EncodedSample { data, duration: self.frame_dur, keyframe: idr })
    }

    fn set_bitrate(&mut self, bitrate_bps: u32) {
        if bitrate_bps == self.bitrate_bps {
            return;
        }
        match Self::build_encoder(bitrate_bps, self.fps) {
            Ok(enc) => {
                self.encoder = enc;
                self.bitrate_bps = bitrate_bps;
                // A fresh encoder must open with a keyframe so the decoder re-syncs.
                self.force_idr_next = true;
            }
            Err(e) => tracing::warn!("set_bitrate rebuild failed, keeping current bitrate: {e}"),
        }
    }
}
```

- [ ] **Step 5: Run the encoder test — expect PASS**

Run: `cargo test --manifest-path agent/Cargo.toml -p rd-agent openh264`
Expected: PASS (existing tests + `set_bitrate_rebuild_still_emits_keyframe`).

- [ ] **Step 6: Write the failing pipeline drain test**

Add a new test module at the bottom of `agent/src/video/pipeline.rs`:
```rust
#[cfg(test)]
mod bitrate_tests {
    use super::*;
    use crate::video::{EncodedSample, Frame, I420, SampleSink, ScreenCapturer, VideoEncoder};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    struct LoopingSource;
    impl ScreenCapturer for LoopingSource {
        fn start(&mut self, sink: std::sync::mpsc::Sender<Frame>) -> anyhow::Result<()> {
            std::thread::spawn(move || loop {
                let f = Frame { width: 2, height: 2, stride: 8, data: vec![0u8; 16], ts_micros: 0 };
                if sink.send(f).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            });
            Ok(())
        }
    }

    struct RecordingEncoder {
        bitrates: Arc<Mutex<Vec<u32>>>,
    }
    impl VideoEncoder for RecordingEncoder {
        fn encode(&mut self, _f: &I420, _idr: bool) -> anyhow::Result<EncodedSample> {
            Ok(EncodedSample { data: vec![0], duration: Duration::from_millis(33), keyframe: true })
        }
        fn set_bitrate(&mut self, bps: u32) {
            self.bitrates.lock().unwrap().push(bps);
        }
    }

    struct NullSink;
    impl SampleSink for NullSink {
        fn write(&self, _s: EncodedSample) {}
    }

    #[test]
    fn pipeline_applies_queued_bitrate_requests_in_order() {
        let bitrates = Arc::new(Mutex::new(Vec::new()));
        let enc = Box::new(RecordingEncoder { bitrates: bitrates.clone() });
        let (tx, rx) = std::sync::mpsc::channel::<u32>();
        tx.send(1_500_000).unwrap();
        tx.send(6_000_000).unwrap();
        let pipeline = VideoPipeline::start(Box::new(LoopingSource), enc, Arc::new(NullSink), 2, 2, 60, rx);
        std::thread::sleep(Duration::from_millis(150)); // let the loop process several frames
        drop(pipeline);
        let seen = bitrates.lock().unwrap().clone();
        assert_eq!(seen, vec![1_500_000, 6_000_000], "got {seen:?}");
    }
}
```

- [ ] **Step 7: Run it — expect FAIL**

Run: `cargo test --manifest-path agent/Cargo.toml pipeline_applies_queued_bitrate`
Expected: FAIL to compile — `VideoPipeline::start` does not yet take a 7th `bitrate_rx` argument.

- [ ] **Step 8: Add `bitrate_rx` to `VideoPipeline::start` + drain loop**

In `agent/src/video/pipeline.rs`, change the signature and add the drain inside the loop:
```rust
    pub fn start(
        mut capturer: Box<dyn ScreenCapturer>,
        mut encoder: Box<dyn VideoEncoder>,
        sink: Arc<dyn SampleSink>,
        dst_w: usize,
        dst_h: usize,
        keyframe_interval: u64,
        bitrate_rx: mpsc::Receiver<u32>,
    ) -> VideoPipeline {
```
Inside the `while let Err(...Empty) = stop_rx.try_recv()` loop, right after obtaining `frame` (before `bgra_to_i420`), drain any pending bitrate requests:
```rust
                // Apply any live bitrate changes requested via the control channel.
                while let Ok(bps) = bitrate_rx.try_recv() {
                    encoder.set_bitrate(bps);
                }
```

- [ ] **Step 9: Update the production callsite (`webrtc_peer.rs`)**

In `agent/src/webrtc_peer.rs::build`, just before the `let capturer = make_source(...)` / `VideoPipeline::start(...)` block, create the channel and pass the receiver. (The sender is wired to the control channel in Task 8; for now, bind it so the code compiles — Task 8 moves it into the dispatcher.)
```rust
        let (bitrate_tx, bitrate_rx) = std::sync::mpsc::channel::<u32>();
        let _ = &bitrate_tx; // wired to the control channel in Task 8
```
Change the pipeline start line to pass `bitrate_rx`:
```rust
        let video = VideoPipeline::start(capturer, encoder, sink, dst_w as usize, dst_h as usize, 60, bitrate_rx);
```
> Note: the encoder-init-failure early-return path (`return Self::finish(...)`) drops `bitrate_rx` unused — that is fine; a control `quality` message then no-ops.

- [ ] **Step 10: Update existing test callers**

Search for other `VideoPipeline::start(` callers: `grep -rn "VideoPipeline::start" agent/`. For each (e.g. in `agent/tests/video_sdp.rs`), append a never-fed receiver argument:
```rust
let (_bitrate_tx, bitrate_rx) = std::sync::mpsc::channel::<u32>();
// …existing args…, bitrate_rx)
```
(Keep `_bitrate_tx` bound in scope so the channel isn't immediately disconnected — though a disconnected receiver is also harmless here, since the drain loop simply stops draining.)

- [ ] **Step 11: Run agent tests + clippy — expect PASS**

Run: `cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings`
Expected: PASS, no clippy warnings.

- [ ] **Step 12: Commit**

```bash
git add agent/src/video/mod.rs agent/src/video/openh264_encoder.rs agent/src/video/pipeline.rs agent/src/webrtc_peer.rs agent/tests
git commit -m "feat(agent): live bitrate switching via encoder rebuild + pipeline drain

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Agent clipboard module (`clipboard.rs`)

**Files:**
- Create: `agent/src/clipboard.rs`
- Modify: `agent/src/main.rs` (add `mod clipboard;`)

**Interfaces:**
- Produces: `crate::clipboard::read_clipboard() -> anyhow::Result<String>`, `write_clipboard(text: &str) -> anyhow::Result<()>`, `clipboard_to_send(current: &str, last_known: &str, cap_bytes: usize) -> Option<String>`, `CLIP_MAX_BYTES: usize`.

- [ ] **Step 1: Write the module with the failing test**

`agent/src/clipboard.rs`:
```rust
use std::io::Write;
use std::process::{Command, Stdio};

/// Approximate 256 KB cap on clipboard payloads (bytes), matching the web
/// CLIP_MAX_BYTES guard.
pub const CLIP_MAX_BYTES: usize = 262_144;

/// Read the macOS clipboard via `pbpaste`. Non-macOS: unsupported (Err).
pub fn read_clipboard() -> anyhow::Result<String> {
    #[cfg(target_os = "macos")]
    {
        let out = Command::new("/usr/bin/pbpaste").output()?;
        if !out.status.success() {
            anyhow::bail!("pbpaste exited with {}", out.status);
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        anyhow::bail!("clipboard unsupported on this platform")
    }
}

/// Write `text` to the macOS clipboard via `pbcopy`. Non-macOS: unsupported (Err).
pub fn write_clipboard(text: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let mut child = Command::new("/usr/bin/pbcopy").stdin(Stdio::piped()).spawn()?;
        child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("no pbcopy stdin"))?
            .write_all(text.as_bytes())?;
        let status = child.wait()?;
        if !status.success() {
            anyhow::bail!("pbcopy exited with {status}");
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
        anyhow::bail!("clipboard unsupported on this platform")
    }
}

/// Decide the text to broadcast given the current clipboard and last-known
/// value. Returns None when unchanged, empty, or over the size cap (skip —
/// never truncate).
pub fn clipboard_to_send(current: &str, last_known: &str, cap_bytes: usize) -> Option<String> {
    if current.is_empty() || current == last_known || current.len() > cap_bytes {
        return None;
    }
    Some(current.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_to_send_skips_unchanged_empty_and_oversized() {
        assert_eq!(clipboard_to_send("a", "a", 100), None); // unchanged
        assert_eq!(clipboard_to_send("", "a", 100), None); // empty
        assert_eq!(clipboard_to_send("b", "a", 100), Some("b".to_string())); // changed
        let big = "x".repeat(101);
        assert_eq!(clipboard_to_send(&big, "a", 100), None); // over cap
    }

    // Requires macOS + a session pasteboard. Run explicitly:
    // cargo test --manifest-path agent/Cargo.toml -- --ignored clipboard_roundtrip
    #[test]
    #[ignore]
    fn clipboard_roundtrip() {
        write_clipboard("rd-clip-test-123").unwrap();
        assert_eq!(read_clipboard().unwrap(), "rd-clip-test-123");
    }
}
```

- [ ] **Step 2: Register the module**

In `agent/src/main.rs`, add:
```rust
mod clipboard;
```

- [ ] **Step 3: Run tests — expect PASS**

Run: `cargo test --manifest-path agent/Cargo.toml clipboard_to_send`
Expected: PASS (the `#[ignore]`d roundtrip is skipped).

- [ ] **Step 4: (macOS, optional) verify the ignored roundtrip**

Run: `cargo test --manifest-path agent/Cargo.toml -- --ignored clipboard_roundtrip`
Expected: PASS on macOS.

- [ ] **Step 5: Commit**

```bash
git add agent/src/clipboard.rs agent/src/main.rs
git commit -m "feat(agent): clipboard read/write via pbcopy/pbpaste + echo guard

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Agent control-channel dispatch — quality + clipboard wiring

**Files:**
- Modify: `agent/src/webrtc_peer.rs`

**Interfaces:**
- Consumes: `crate::control::{ControlMessage, ClipMode}`, `crate::clipboard`, the `bitrate_tx` from Task 6.
- Produces: agent-side handling of `quality` (→ bitrate), `clip-set` (→ write clipboard), `clip-request` (→ reply clip-set), `clip-mode` (→ start/stop the clipboard poller in `both`).

> **Verification note:** the `on_data_channel` control handler needs a live data channel, so it is verified by `cargo build` + the existing ICE/SDP integration tests still passing + manual e2e. The pure decision logic it relies on (`clipboard_to_send`, `ControlMessage` parsing, `set_bitrate`) is unit-tested in Tasks 5–7.

- [ ] **Step 1: Add imports**

At the top of `agent/src/webrtc_peer.rs`:
```rust
use crate::control::{ClipMode, ControlMessage};
use crate::clipboard;
use std::sync::atomic::{AtomicBool, Ordering};
use webrtc::data_channel::data_channel_message::DataChannelMessage;
```
(`DataChannelMessage` is already imported — do not duplicate; add only the missing ones.) Also ensure `std::sync::Mutex` and `std::sync::Arc` are imported (they are).

- [ ] **Step 2: Dispatch `on_data_channel` by label**

Replace the existing `pc.on_data_channel(...)` block in `build` with a label-dispatching version. Create the shared clipboard state and clone the bitrate sender first (place this right after `let (bitrate_tx, bitrate_rx) = ...` from Task 6, and remove the temporary `let _ = &bitrate_tx;` line):
```rust
        // Shared "last clipboard value we set or saw", so the agent's poller does
        // not echo back a value the web端 just pushed (and vice-versa).
        let last_clipboard: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

        let dc_input_tx = input_tx.clone();
        let ctl_bitrate_tx = bitrate_tx.clone();
        let ctl_last_clip = last_clipboard.clone();
        pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
            match dc.label() {
                "input" => wire_input(dc, dc_input_tx.clone()),
                "control" => wire_control(dc, ctl_bitrate_tx.clone(), ctl_last_clip.clone()),
                other => tracing::warn!("ignoring unknown data channel: {other}"),
            }
            Box::pin(async {})
        }));
```

- [ ] **Step 3: Implement `wire_control`**

Add this free function next to `wire_input`:
```rust
/// Wire the bidirectional "control" data channel: clipboard sync + live quality.
/// `bitrate_tx` forwards quality requests to the video pipeline; `last_clipboard`
/// is the shared echo-suppression state. The agent only reads+broadcasts its own
/// clipboard while a `both` subscription is active (privacy: no unsolicited reads).
fn wire_control(
    dc: Arc<RTCDataChannel>,
    bitrate_tx: Sender<u32>,
    last_clipboard: Arc<Mutex<String>>,
) {
    // Poller lifecycle: `poller_on` is flipped false to stop the current poller.
    let poller_on = Arc::new(AtomicBool::new(false));
    let dc_for_msg = dc.clone();
    dc.on_message(Box::new(move |msg: DataChannelMessage| {
        let bitrate_tx = bitrate_tx.clone();
        let last_clipboard = last_clipboard.clone();
        let poller_on = poller_on.clone();
        let dc = dc_for_msg.clone();
        Box::pin(async move {
            let text = match String::from_utf8(msg.data.to_vec()) {
                Ok(t) => t,
                Err(_) => {
                    tracing::warn!("dropping non-utf8 control frame");
                    return;
                }
            };
            let ctl: ControlMessage = match serde_json::from_str(&text) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("dropping malformed control message: {e}");
                    return;
                }
            };
            match ctl {
                ControlMessage::Quality { bitrate_bps } => {
                    let clamped = bitrate_bps.clamp(250_000, 20_000_000);
                    let _ = bitrate_tx.send(clamped);
                }
                ControlMessage::ClipSet { text } => {
                    if text.len() <= clipboard::CLIP_MAX_BYTES {
                        if let Err(e) = clipboard::write_clipboard(&text) {
                            tracing::warn!("write_clipboard failed: {e}");
                        } else {
                            *last_clipboard.lock().unwrap() = text;
                        }
                    }
                }
                ControlMessage::ClipRequest => {
                    match clipboard::read_clipboard() {
                        Ok(current) => {
                            *last_clipboard.lock().unwrap() = current.clone();
                            send_clip_set(&dc, current);
                        }
                        Err(e) => tracing::warn!("read_clipboard failed: {e}"),
                    }
                }
                ControlMessage::ClipMode { mode } => {
                    if mode == ClipMode::Both {
                        start_clipboard_poller(dc.clone(), last_clipboard.clone(), poller_on.clone());
                    } else {
                        poller_on.store(false, Ordering::SeqCst);
                    }
                }
            }
        })
    }));
}

/// Serialize + send a clip-set on the control channel (fire-and-forget).
fn send_clip_set(dc: &Arc<RTCDataChannel>, text: String) {
    let msg = ControlMessage::ClipSet { text };
    let json = match serde_json::to_string(&msg) {
        Ok(j) => j,
        Err(e) => {
            tracing::warn!("failed to serialize clip-set: {e}");
            return;
        }
    };
    let dc = dc.clone();
    tokio::spawn(async move {
        if let Err(e) = dc.send_text(json).await {
            tracing::warn!("failed to send clip-set: {e}");
        }
    });
}

/// Start (or restart) the agent-side clipboard poller for `both` mode. Reads the
/// clipboard ~every 800ms; on a change vs the shared last-known value, pushes a
/// clip-set to the web端. Stops when `poller_on` is set false (mode left `both`)
/// or the channel closes.
fn start_clipboard_poller(
    dc: Arc<RTCDataChannel>,
    last_clipboard: Arc<Mutex<String>>,
    poller_on: Arc<AtomicBool>,
) {
    // Idempotent: if already running, leave it.
    if poller_on.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        while poller_on.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            if !poller_on.load(Ordering::SeqCst) {
                break;
            }
            let current = match clipboard::read_clipboard() {
                Ok(c) => c,
                Err(_) => continue,
            };
            let to_send = {
                let last = last_clipboard.lock().unwrap();
                clipboard::clipboard_to_send(&current, &last, clipboard::CLIP_MAX_BYTES)
            };
            if let Some(text) = to_send {
                *last_clipboard.lock().unwrap() = text.clone();
                send_clip_set(&dc, text);
            }
        }
    });
}
```

- [ ] **Step 3b: Confirm `Sender` is imported**

`use std::sync::mpsc::Sender;` already exists at the top of `webrtc_peer.rs` (used by `wire_input`). No change needed.

- [ ] **Step 4: Build + run the full agent suite + clippy**

Run: `cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings`
Expected: PASS, no warnings. (Existing ICE/SDP integration tests must still pass, proving the dispatch refactor didn't break input wiring.)

- [ ] **Step 5: Commit**

```bash
git add agent/src/webrtc_peer.rs
git commit -m "feat(agent): control-channel dispatch — quality + clipboard sync

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: Quality preset UI (web)

**Files:**
- Modify: `packages/web/src/pages/SessionView.tsx`, `packages/web/src/pages/SessionView.test.tsx`

**Interfaces:**
- Consumes: `Session.sendControl`, `ControlMessage` from `@rd/protocol`.

- [ ] **Step 1: Extend the RTL mock session with `sendControl`**

In `SessionView.test.tsx`, update the hoisted mock:
```ts
const h = vi.hoisted(() => ({
  opts: null as null | { onState: (s: string) => void; onClipboard?: (t: string) => void },
  session: {
    close: vi.fn(),
    sendInput: vi.fn(),
    getStats: vi.fn().mockResolvedValue(null),
    sendControl: vi.fn(),
  },
}));
```

- [ ] **Step 2: Write the failing RTL test**

```ts
it("sends a quality control message when a preset is chosen", () => {
  render(<SessionView token="t" device={device} onExit={() => {}} />);
  act(() => h.opts!.onState("connected"));
  fireEvent.change(screen.getByTestId("quality-select"), { target: { value: "6000000" } });
  expect(h.session.sendControl).toHaveBeenCalledWith({ t: "quality", bitrateBps: 6000000 });
});
```

- [ ] **Step 3: Run it — expect FAIL**

Run: `npm test -w @rd/web -- SessionView`
Expected: FAIL — no `quality-select` element.

- [ ] **Step 4: Implement the selector in `SessionView.tsx`**

Add a constant near the top of the file (module scope, after imports):
```ts
const QUALITY_PRESETS = [
  { label: "流畅", bps: 1_500_000 },
  { label: "均衡", bps: 3_000_000 },
  { label: "高清", bps: 6_000_000 },
];
```
Add state near the other `useState` hooks:
```ts
const [bitrate, setBitrate] = useState(3_000_000);
```
Add a handler near `sendCombo`:
```ts
const onQuality = useCallback((bps: number) => {
  setBitrate(bps);
  sessionRef.current?.sendControl({ t: "quality", bitrateBps: bps });
}, []);
```
Add the control to the header `<span>` (next to the stats button):
```tsx
<select
  data-testid="quality-select"
  value={bitrate}
  disabled={!connected}
  onChange={(e) => onQuality(Number(e.target.value))}
  style={{ fontSize: 12 }}
>
  {QUALITY_PRESETS.map((q) => (
    <option key={q.bps} value={q.bps}>{q.label}</option>
  ))}
</select>
```

- [ ] **Step 5: Run web tests + typecheck + build — expect PASS**

Run: `npm test -w @rd/web && npm run -w @rd/web typecheck && npm run -w @rd/web build`
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add packages/web/src/pages/SessionView.tsx packages/web/src/pages/SessionView.test.tsx
git commit -m "feat(web): quality preset selector (流畅/均衡/高清)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 10: Clipboard UI + auto-sync (web)

**Files:**
- Create: `packages/web/src/clipboard.ts`, `packages/web/src/clipboard.test.ts`
- Modify: `packages/web/src/pages/SessionView.tsx`, `packages/web/src/pages/SessionView.test.tsx`

**Interfaces:**
- Consumes: `Session.sendControl`, `SessionCallbacks.onClipboard`, `ClipMode` from `@rd/protocol`.
- Produces: `clipboardToSend(current, lastKnown, capBytes?): string | null`.

- [ ] **Step 1: Write the failing unit test**

`packages/web/src/clipboard.test.ts`:
```ts
import { describe, it, expect } from "vitest";
import { clipboardToSend } from "./clipboard.js";

describe("clipboardToSend", () => {
  it("skips unchanged, empty, and oversized; returns changed text", () => {
    expect(clipboardToSend("a", "a")).toBeNull(); // unchanged
    expect(clipboardToSend("", "a")).toBeNull(); // empty
    expect(clipboardToSend("b", "a")).toBe("b"); // changed
    expect(clipboardToSend("x".repeat(11), "a", 10)).toBeNull(); // over cap
  });
});
```

- [ ] **Step 2: Run it — expect FAIL**

Run: `npm test -w @rd/web -- clipboard`
Expected: FAIL — cannot find `./clipboard.js`.

- [ ] **Step 3: Implement `clipboard.ts`**

`packages/web/src/clipboard.ts`:
```ts
import type { ClipMode } from "@rd/protocol";
export type { ClipMode };

export const CLIP_MAX_BYTES = 262144;

/** The text to send given the current clipboard + last-known value, or null to
 *  skip (unchanged, empty, or over the size cap — never truncate). */
export function clipboardToSend(
  current: string,
  lastKnown: string,
  capBytes = CLIP_MAX_BYTES,
): string | null {
  if (current.length === 0 || current === lastKnown || current.length > capBytes) {
    return null;
  }
  return current;
}
```

- [ ] **Step 4: Run the unit test — expect PASS**

Run: `npm test -w @rd/web -- clipboard`
Expected: PASS.

- [ ] **Step 5: Wire clipboard into `SessionView.tsx`**

Add imports:
```ts
import { clipboardToSend, type ClipMode } from "../clipboard.js";
```
Add state + a `lastClip` ref near the other hooks:
```ts
const [clipMode, setClipMode] = useState<ClipMode>("off");
const lastClip = useRef<string>("");
```
Add `onClipboard` to the `connectSession` callbacks object (inside the mount effect, alongside `onState`/`onError`/`onRemoteStream`):
```ts
onClipboard: (text) => {
  // Received a clip-set (manual pull reply, or both-mode auto push): mirror it
  // locally and record it so our own poller won't echo it back.
  lastClip.current = text;
  void navigator.clipboard.writeText(text).catch(() => {});
},
```
Add handlers near `onQuality`:
```ts
const onClipModeChange = useCallback((mode: ClipMode) => {
  setClipMode(mode);
  sessionRef.current?.sendControl({ t: "clip-mode", mode });
}, []);

const sendMyClipboard = useCallback(async () => {
  try {
    const text = await navigator.clipboard.readText();
    const toSend = clipboardToSend(text, lastClip.current);
    if (toSend !== null) {
      lastClip.current = toSend;
      sessionRef.current?.sendControl({ t: "clip-set", text: toSend });
    }
  } catch {
    /* clipboard read denied / no focus */
  }
}, []);

const pullRemoteClipboard = useCallback(() => {
  sessionRef.current?.sendControl({ t: "clip-request" });
}, []);
```
Add an auto-poll effect (after the stats effect):
```ts
// Auto-sync local → remote in oneway/both while the tab has focus.
useEffect(() => {
  if (!connected || clipMode === "off") return;
  const id = setInterval(() => {
    if (!document.hasFocus()) return;
    void navigator.clipboard.readText().then((text) => {
      const toSend = clipboardToSend(text, lastClip.current);
      if (toSend !== null) {
        lastClip.current = toSend;
        sessionRef.current?.sendControl({ t: "clip-set", text: toSend });
      }
    }).catch(() => {});
  }, 800);
  return () => clearInterval(id);
}, [connected, clipMode]);
```
Add the UI to the header `<span>` (after the quality select):
```tsx
<select
  data-testid="clip-mode"
  value={clipMode}
  disabled={!connected}
  onChange={(e) => onClipModeChange(e.target.value as ClipMode)}
  style={{ fontSize: 12 }}
>
  <option value="off">剪贴板:手动</option>
  <option value="oneway">剪贴板:单向</option>
  <option value="both">剪贴板:双向</option>
</select>
<button data-testid="clip-send" disabled={!connected} onClick={() => void sendMyClipboard()} style={{ fontSize: 12 }}>
  发送剪贴板
</button>
<button data-testid="clip-pull" disabled={!connected} onClick={pullRemoteClipboard} style={{ fontSize: 12 }}>
  拉取远程
</button>
```

- [ ] **Step 6: Add RTL tests (mock `navigator.clipboard`)**

In `SessionView.test.tsx` `beforeEach`, add a clipboard mock:
```ts
Object.defineProperty(navigator, "clipboard", {
  configurable: true,
  value: {
    readText: vi.fn().mockResolvedValue("hello"),
    writeText: vi.fn().mockResolvedValue(undefined),
  },
});
```
Ensure the hoisted `h.opts` type includes `onClipboard?: (t: string) => void` (done in Task 9 mock update). Add tests:
```ts
it("sends clip-mode when the mode selector changes", () => {
  render(<SessionView token="t" device={device} onExit={() => {}} />);
  act(() => h.opts!.onState("connected"));
  fireEvent.change(screen.getByTestId("clip-mode"), { target: { value: "both" } });
  expect(h.session.sendControl).toHaveBeenCalledWith({ t: "clip-mode", mode: "both" });
});

it("reads the local clipboard and sends clip-set on 'send'", async () => {
  render(<SessionView token="t" device={device} onExit={() => {}} />);
  act(() => h.opts!.onState("connected"));
  await act(async () => {
    fireEvent.click(screen.getByTestId("clip-send"));
  });
  expect(navigator.clipboard.readText).toHaveBeenCalled();
  expect(h.session.sendControl).toHaveBeenCalledWith({ t: "clip-set", text: "hello" });
});

it("writes the local clipboard when a clip-set arrives", () => {
  render(<SessionView token="t" device={device} onExit={() => {}} />);
  act(() => h.opts!.onState("connected"));
  act(() => h.opts!.onClipboard!("world"));
  expect(navigator.clipboard.writeText).toHaveBeenCalledWith("world");
});
```

- [ ] **Step 7: Run web tests + typecheck + build — expect PASS**

Run: `npm test -w @rd/web && npm run -w @rd/web typecheck && npm run -w @rd/web build`
Expected: all PASS.

- [ ] **Step 8: Commit**

```bash
git add packages/web/src/clipboard.ts packages/web/src/clipboard.test.ts packages/web/src/pages/SessionView.tsx packages/web/src/pages/SessionView.test.tsx
git commit -m "feat(web): clipboard sync UI (manual + oneway/both auto)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification (before finishing the branch)

- [ ] Full workspace tests: `npm test` (all packages green).
- [ ] Full typecheck + build: `npm run -w @rd/protocol typecheck && npm run -w @rd/web typecheck && npm run -w @rd/web build`.
- [ ] Agent: `cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings`.
- [ ] Manual e2e (macOS被控端, live browser): connect, then verify each — a combo (e.g. Spotlight) fires on the remote; the stats HUD shows non-zero fps/kbps; switching 高清/流畅 changes the HUD bitrate within ~1–2s without a reconnect; "发送剪贴板" then paste on the remote; "拉取远程" then paste locally; set mode 双向, copy on one side, confirm it appears on the other with no echo storm.
- [ ] Then use **superpowers:finishing-a-development-branch** to merge to `main` + push.

## Notes for the implementer

- The web live `connectSession` control-channel code (Task 4) has no unit test by design — jsdom has no WebRTC. Do not invent brittle fake-RTCPeerConnection tests; rely on typecheck/build + the RTL tests in Tasks 9–10 (which drive the mocked `Session`) + manual e2e.
- Keep `SessionView.tsx` edits additive and co-located with the existing header controls; the file is already large but the spec keeps these controls there rather than extracting a toolbar (YAGNI for this pass).
- Echo suppression correctness depends on **both** sides updating their `last_known`/`lastClip` whenever they *apply* a received value. Do not skip those updates.
