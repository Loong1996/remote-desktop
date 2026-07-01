# Plan 5 — macOS Video Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the Plan 4 review findings so the macOS remote-access path is performant and reliably usable: stop capture on session end (no cross-session leak), capture at 720p/30fps at the source, map pointer coords correctly on the letterboxed `<video>`, and never leave keys/buttons stuck.

**Architecture:** Agent-side changes to `SckCapturer` (owner-thread stream lifecycle + capture config), `InputInjector` (release held keys/buttons when the input channel closes), and `convert` (robustness). Web-side changes to coordinate mapping (`objectFit: contain` content box) and stuck-key release on blur. All behind the existing Plan 3/4 structure; no new capability, no new deps.

**Tech Stack:** Rust (`screencapturekit` 8, `enigo` 0.6, existing); TypeScript/React (native DOM, Vitest).

## Global Constraints

- Toolchains: Node ≥ 20; `cargo` in `~/.cargo/bin` — prefix cargo commands with `export PATH="$HOME/.cargo/bin:$PATH"`.
- No new dependencies. `serial_test` is already a dev-dependency.
- macOS-only capture code stays `#[cfg(target_os = "macos")]`.
- Fixed capture params: 1280×720, 30 fps (must match the pipeline's `dst_w/dst_h` and the encoder's `1/30` sample duration).
- Idempotent stuck-key release: web (primary) and agent (backstop) may both release the same key; that is fine (releasing an already-released key is harmless).
- API-verification rule: leaf signatures below (`screencapturekit` `with_width/with_height/with_fps/stop_capture`, enigo release calls) are transcribed from docs; if one does not compile, verify against docs.rs (`screencapturekit` 8.0.0, `enigo` 0.6.1) and report the correction — do not guess.
- TDD; frequent commits; commit messages English ending with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- Do not break green baselines: agent `cargo test` (33 + 2 ignored) grows and stays green; `cargo clippy --all-targets` clean; Node `npm test` (59) grows and stays green; `npm run typecheck` + `npm run -w @rd/web build` clean.

## File Structure

```
agent/src/video/sck_capturer.rs   # Modify: owner-thread stream lifecycle + 720p/30fps config
agent/src/input.rs                # Modify: PressedState + release-on-close in injector_loop; MouseButton Eq/Hash
agent/src/video/convert.rs        # Modify: 0x0 guard + padded-stride test
agent/src/video/mod.rs            # Modify: #[serial] on the env test
packages/web/src/rtc.ts           # Modify: contentRect() pure helper (+ test)
packages/web/src/rtc.test.ts      # Modify: contentRect + releaseEvents tests
packages/web/src/pages/SessionView.tsx  # Modify: letterbox-aware coords + stuck-key release on blur
docs/superpowers/plan4-video-smoke.md   # Modify: note 720p/30fps + capture stops on disconnect
```

## Parallel Groups

- **Group A (agent):** Tasks 1–3, sequential (Tasks 1 touches sck_capturer.rs, 2 touches input.rs, 3 touches convert.rs+mod.rs — but keep sequential per same-dir git discipline).
- **Group B (web):** Tasks 4–5, sequential (both touch SessionView.tsx). Disjoint from A → dispatch concurrently with the agent track.
- **Task 6 (docs):** after A + B.

---

## Task 1: SckCapturer — owner-thread stream lifecycle + 720p/30fps

**Files:** Modify `agent/src/video/sck_capturer.rs`

**Interfaces:**
- Produces: `SckCapturer { fps: u32 }` (a private `stop` field is added) still implementing `ScreenCapturer`; capture stops when the `SckCapturer` is dropped. `SckCapturer::new(fps)` is not required (struct-literal construction stays, but add the `stop: None` field default via a constructor to keep call sites simple — `make_source` builds `SckCapturer { fps, stop: None }`).

- [ ] **Step 1: Replace the leaked-stream capture with an owner thread**

The current `start` builds the stream and `std::mem::forget`s it, and captures at native resolution/fps. Replace the `SckCapturer` struct + its `ScreenCapturer` impl with an owner-thread design: the stream is built and owned entirely on a dedicated thread (so `SCStream`, which is not `Send`, never crosses a thread boundary), and a stop channel tears it down on `Drop`. Also configure 720×1280 / 30 fps at the source.

Replace the `pub struct SckCapturer { pub fps: u32 }` and its `impl ScreenCapturer` with:

```rust
/// Captures the main display via ScreenCaptureKit at 1280x720 / 30fps,
/// delivering BGRA `Frame`s. The `SCStream` is created and owned on a dedicated
/// thread (SCStream is not `Send`); dropping the capturer signals that thread to
/// `stop_capture()` and release the stream — no per-session leak.
pub struct SckCapturer {
    pub fps: u32,
    stop: Option<std::sync::mpsc::Sender<()>>,
}

impl SckCapturer {
    pub fn new(fps: u32) -> Self {
        Self { fps, stop: None }
    }
}

impl ScreenCapturer for SckCapturer {
    fn start(&mut self, sink: Sender<Frame>) -> anyhow::Result<()> {
        let fps = self.fps;
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        // Report setup success/failure back to start() so permission/stream
        // errors surface synchronously.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();

        std::thread::spawn(move || {
            // Build + start the stream on THIS thread; it never moves.
            let built = (|| -> anyhow::Result<SCStream> {
                let content = SCShareableContent::get()
                    .map_err(|e| anyhow::anyhow!("SCShareableContent: {e:?}"))?;
                let display = content
                    .displays()
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("no display found"))?;
                let filter = SCContentFilter::create()
                    .with_display(&display)
                    .with_excluding_windows(&[])
                    .build();
                let config = SCStreamConfiguration::new()
                    .with_width(1280)
                    .with_height(720)
                    .with_fps(fps)
                    .with_pixel_format(PixelFormat::BGRA);
                let mut stream = SCStream::new(&filter, &config);
                stream.add_output_handler(
                    FrameHandler { sink, start: std::time::Instant::now() },
                    SCStreamOutputType::Screen,
                );
                stream
                    .start_capture()
                    .map_err(|e| anyhow::anyhow!("start_capture: {e:?}"))?;
                Ok(stream)
            })();

            match built {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    // Keep the stream alive on this thread until stop is signaled
                    // (or the capturer is dropped, which drops stop_tx).
                    let _ = stop_rx.recv();
                    if let Err(e) = stream.stop_capture() {
                        tracing::warn!("stop_capture failed: {e:?}");
                    }
                    // stream drops here, on its owner thread.
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.stop = Some(stop_tx);
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow::anyhow!("capture owner thread exited during setup")),
        }
    }
}

impl Drop for SckCapturer {
    fn drop(&mut self) {
        // Signal the owner thread to stop_capture + release the stream.
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}
```

Remove the `use` of nothing else changes (the imports at the top stay). Delete the old `std::mem::forget(stream);` path entirely (it no longer exists in the new impl).

- [ ] **Step 2: Update `make_source` construction**

In `agent/src/video/mod.rs`, the macOS branch of `make_source` currently builds `Box::new(sck_capturer::SckCapturer { fps })`. Change it to `Box::new(sck_capturer::SckCapturer::new(fps))` (the struct now has a private `stop` field, so the struct literal won't compile outside the module).

- [ ] **Step 3: Verify build + the ignored real-capture test still compiles**

The `#[ignore]` `captures_a_real_frame` test builds a capturer via `SckCapturer { fps: 30 }` — update it to `SckCapturer::new(30)`.

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build --manifest-path agent/Cargo.toml && cargo test --manifest-path agent/Cargo.toml`
Expected: builds clean; full suite green; `captures_a_real_frame` listed ignored. (Verify `with_width`/`with_height`/`with_fps`/`stop_capture` compile against screencapturekit 8.0.0; if a name differs, correct + report.)

- [ ] **Step 4: Clippy + commit**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo clippy --manifest-path agent/Cargo.toml --all-targets` → clean.

```bash
git add agent/src/video/sck_capturer.rs agent/src/video/mod.rs
git commit -m "fix(agent): SckCapturer owns its SCStream (stop on drop) + 720p/30fps capture

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

Note: real capture stop + 720p/30fps output are validated by the manual smoke (Task 6), since they need a display + Screen Recording permission.

---

## Task 2: InputInjector — release held keys/buttons on channel close

**Files:** Modify `agent/src/input.rs`

**Interfaces:**
- Consumes: `InputEvent`, `MouseButton`, `inject` (existing).
- Produces: a private `PressedState` with `apply(&mut self, &InputEvent)` and `pending_releases(&self) -> Vec<InputEvent>`; `injector_loop` releases all held keys/buttons after the channel closes. `MouseButton` gains `Eq, Hash`.

- [ ] **Step 1: Write the failing unit test**

Add to the existing `#[cfg(test)] mod injector_tests` in `agent/src/input.rs` (or a new `pressed_tests` module):

```rust
#[cfg(test)]
mod pressed_tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn tracks_and_releases_held_keys_and_buttons() {
        let mut s = PressedState::default();
        s.apply(&InputEvent::KDown { code: "ShiftLeft".into() });
        s.apply(&InputEvent::KDown { code: "KeyA".into() });
        s.apply(&InputEvent::KUp { code: "KeyA".into() }); // released, should not linger
        s.apply(&InputEvent::MDown { button: MouseButton::Left });
        let rel: HashSet<String> = s
            .pending_releases()
            .into_iter()
            .map(|e| format!("{e:?}"))
            .collect();
        // ShiftLeft still down, KeyA released, Left button still down
        assert!(rel.contains(&format!("{:?}", InputEvent::KUp { code: "ShiftLeft".into() })));
        assert!(rel.contains(&format!("{:?}", InputEvent::MUp { button: MouseButton::Left })));
        assert!(!rel.contains(&format!("{:?}", InputEvent::KUp { code: "KeyA".into() })));
        assert_eq!(rel.len(), 2);
    }

    #[test]
    fn nothing_to_release_when_all_up() {
        let mut s = PressedState::default();
        s.apply(&InputEvent::KDown { code: "KeyB".into() });
        s.apply(&InputEvent::KUp { code: "KeyB".into() });
        assert!(s.pending_releases().is_empty());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib pressed_tests`
Expected: FAIL to compile — `PressedState` not found.

- [ ] **Step 3: Add `Eq, Hash` to `MouseButton` and implement `PressedState`**

In `agent/src/input.rs`, extend the `MouseButton` derive (currently `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`) to include `Eq, Hash`:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}
```

Add `PressedState` (near `InputInjector`):

```rust
/// Tracks which keys/buttons are currently pressed, so they can be released if
/// the input channel closes mid-hold (web crash / tab close / disconnect) —
/// otherwise a held modifier or button would stick on the被控端.
#[derive(Default)]
struct PressedState {
    keys: std::collections::HashSet<String>,
    buttons: std::collections::HashSet<MouseButton>,
}

impl PressedState {
    fn apply(&mut self, ev: &InputEvent) {
        match ev {
            InputEvent::KDown { code } => {
                self.keys.insert(code.clone());
            }
            InputEvent::KUp { code } => {
                self.keys.remove(code);
            }
            InputEvent::MDown { button } => {
                self.buttons.insert(button.clone());
            }
            InputEvent::MUp { button } => {
                self.buttons.remove(button);
            }
            _ => {}
        }
    }

    fn pending_releases(&self) -> Vec<InputEvent> {
        let mut out = Vec::new();
        for code in &self.keys {
            out.push(InputEvent::KUp { code: code.clone() });
        }
        for button in &self.buttons {
            out.push(InputEvent::MUp { button: button.clone() });
        }
        out
    }
}
```

- [ ] **Step 4: Wire release-on-close into `injector_loop`**

In `injector_loop`, track pressed state as events are injected, and release everything still held after the channel closes (the `while` loop exits when all `Sender`s drop). Replace the `while let Ok(ev) = rx.recv()` block:

```rust
    let mut pressed = PressedState::default();
    while let Ok(ev) = rx.recv() {
        pressed.apply(&ev);
        if let Err(e) = inject(&mut enigo, &ev) {
            tracing::warn!("failed to inject {ev:?}: {e}");
        }
    }
    // Channel closed (session ended / web gone): release anything still held so
    // no key or button sticks down on the被控端.
    for ev in pressed.pending_releases() {
        if let Err(e) = inject(&mut enigo, &ev) {
            tracing::warn!("failed to release {ev:?} on shutdown: {e}");
        }
    }
```

- [ ] **Step 5: Run to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib pressed_tests`
Expected: PASS (2 tests).

- [ ] **Step 6: Full suite + clippy + commit**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets`
Expected: all green, clippy clean.

```bash
git add agent/src/input.rs
git commit -m "fix(agent): release held keys/buttons when the input channel closes

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: convert 0×0 guard + padded-stride test + env #[serial]

**Files:** Modify `agent/src/video/convert.rs`, `agent/src/video/mod.rs`

**Interfaces:** `bgra_to_i420` returns an empty `I420` for a 0×0 target instead of panicking; behavior for non-zero targets unchanged.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `agent/src/video/convert.rs`:

```rust
    #[test]
    fn zero_target_returns_empty_without_panic() {
        let i = bgra_to_i420(&solid_bgra(16, 16, 0, 0, 0), 0, 0);
        assert_eq!((i.width, i.height), (0, 0));
        assert!(i.y.is_empty() && i.u.is_empty() && i.v.is_empty());
    }

    #[test]
    fn handles_padded_source_stride() {
        // Source with row padding: stride > width*4. Fill visible pixels white,
        // padding black; converting must read via stride and yield high luma.
        let (w, h) = (16usize, 16usize);
        let stride = w * 4 + 32; // padded
        let mut data = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * stride + x * 4;
                data[i] = 255; data[i + 1] = 255; data[i + 2] = 255; data[i + 3] = 255;
            }
        }
        let frame = Frame { width: w as u32, height: h as u32, stride, data, ts_micros: 0 };
        let out = bgra_to_i420(&frame, w, h);
        // Assert a NON-ZERO row too: y[0] is at offset 0 regardless of stride, so
        // only a later row discriminates correct stride math from a width*4 bug
        // (which would read into the previous row's black padding for rows >= 1).
        assert!(out.y[0] > 200, "row 0 white luma {} too low", out.y[0]);
        assert!(out.y[out.y_stride] > 200, "row 1 white luma {} too low — stride mishandled", out.y[out.y_stride]);
        assert!(out.y[out.y_stride * (h - 1)] > 200, "last-row white luma too low — stride mishandled");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib video::convert`
Expected: `zero_target_returns_empty_without_panic` panics (the `.expect` on a 0×0 conversion), or fails.

- [ ] **Step 3: Guard the 0×0 case**

At the top of `bgra_to_i420` in `agent/src/video/convert.rs`, before the `resize_bgra`/conversion:

```rust
pub fn bgra_to_i420(frame: &Frame, dst_w: usize, dst_h: usize) -> I420 {
    if dst_w == 0 || dst_h == 0 {
        return I420 { width: 0, height: 0, y: Vec::new(), u: Vec::new(), v: Vec::new(), y_stride: 0, uv_stride: 0 };
    }
    // ... existing body unchanged ...
```

- [ ] **Step 4: Run to verify both pass**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib video::convert`
Expected: PASS (existing + 2 new).

- [ ] **Step 5: Add `#[serial]` to the env-mutating test**

In `agent/src/video/mod.rs`, the `source_selection_tests` module's `testpattern_env_forces_synthetic_source` mutates `RD_VIDEO_SOURCE`. Import and annotate:

```rust
    use serial_test::serial;

    #[test]
    #[serial]
    fn testpattern_env_forces_synthetic_source() {
        // ... unchanged body ...
    }
```

(`serial_test` is already a dev-dependency; add the `use` inside the test module.)

- [ ] **Step 6: Run + clippy + commit**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets`
Expected: all green, clippy clean.

```bash
git add agent/src/video/convert.rs agent/src/video/mod.rs
git commit -m "fix(agent): guard 0x0 convert target + padded-stride test + serial env test

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Web — letterbox-aware coordinate mapping

**Files:** Modify `packages/web/src/rtc.ts`, `packages/web/src/rtc.test.ts`, `packages/web/src/pages/SessionView.tsx`

**Interfaces:**
- Produces: `export function contentRect(el: { width: number; height: number }, videoW: number, videoH: number): { left: number; top: number; width: number; height: number }` — the `object-fit: contain` content box within the element (offsets relative to the element's top-left). Falls back to the full element when `videoW`/`videoH` ≤ 0.

- [ ] **Step 1: Write the failing test**

Add to `packages/web/src/rtc.test.ts`:

```ts
import { contentRect } from "./rtc.js";

test("contentRect: wide video in a square element letterboxes top/bottom", () => {
  const r = contentRect({ width: 400, height: 400 }, 1600, 900);
  expect(r.width).toBe(400);
  expect(r.height).toBe(225);
  expect(r.left).toBe(0);
  expect(r.top).toBe(87.5);
});

test("contentRect: tall video in a wide element pillarboxes left/right", () => {
  const r = contentRect({ width: 400, height: 200 }, 100, 200);
  expect(r.height).toBe(200);
  expect(r.width).toBe(100);
  expect(r.top).toBe(0);
  expect(r.left).toBe(150);
});

test("contentRect: no stream falls back to the element box", () => {
  expect(contentRect({ width: 320, height: 240 }, 0, 0)).toEqual({
    left: 0, top: 0, width: 320, height: 240,
  });
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `npm test -- rtc.test`
Expected: FAIL — `contentRect` not exported.

- [ ] **Step 3: Implement `contentRect`**

Add to `packages/web/src/rtc.ts` (near `mouseCoords`):

```ts
/** The `object-fit: contain` content box (offsets relative to the element's
 *  top-left) for a video of intrinsic size `videoW`×`videoH` shown in an
 *  element of size `el`. Falls back to the full element when the size is unknown. */
export function contentRect(
  el: { width: number; height: number },
  videoW: number,
  videoH: number,
): { left: number; top: number; width: number; height: number } {
  if (videoW <= 0 || videoH <= 0) {
    return { left: 0, top: 0, width: el.width, height: el.height };
  }
  const scale = Math.min(el.width / videoW, el.height / videoH);
  const width = videoW * scale;
  const height = videoH * scale;
  return { left: (el.width - width) / 2, top: (el.height - height) / 2, width, height };
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `npm test -- rtc.test`
Expected: PASS.

- [ ] **Step 5: Use it in SessionView's mouse mapping**

In `packages/web/src/pages/SessionView.tsx`, import `contentRect` alongside `mouseCoords`/`mouseButtonName`. In `onMouseMove`, replace the direct `mouseCoords(e.clientX, e.clientY, rect)` with a content-box-adjusted rect derived from the video's intrinsic size:

```tsx
  function onMouseMove(e: React.MouseEvent) {
    if (!connected || !surfaceRef.current) return;
    const el = surfaceRef.current;
    const rect = el.getBoundingClientRect();
    const box = contentRect({ width: rect.width, height: rect.height }, el.videoWidth, el.videoHeight);
    const adj = { left: rect.left + box.left, top: rect.top + box.top, width: box.width, height: box.height };
    pendingMove.current = mouseCoords(e.clientX, e.clientY, adj);
    // ... existing rAF scheduling unchanged ...
  }
```

(`el` is the `<video>` element — `videoWidth`/`videoHeight` are its intrinsic frame size, 0 until a frame arrives, in which case `contentRect` returns the full element box, preserving today's behavior. `mouseCoords` already clamps to [0,1], so pointer positions in the letterbox bars clamp to the edge.)

- [ ] **Step 6: Build + typecheck + commit**

Run: `npm run -w @rd/web build && npm run typecheck && npm test`
Expected: build clean, typecheck clean, tests pass (59 + 3 new).

```bash
git add packages/web/src/rtc.ts packages/web/src/rtc.test.ts packages/web/src/pages/SessionView.tsx
git commit -m "fix(web): map pointer coords to the letterboxed video content box

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Web — release held keys/buttons on blur

**Files:** Modify `packages/web/src/rtc.ts`, `packages/web/src/rtc.test.ts`, `packages/web/src/pages/SessionView.tsx`

**Interfaces:**
- Produces: `export function releaseEvents(keys: string[], buttons: number[]): InputEvent[]` — the `kup`/`mup` events to release a set of held keys/buttons.

- [ ] **Step 1: Write the failing test**

Add to `packages/web/src/rtc.test.ts`:

```ts
import { releaseEvents } from "./rtc.js";
import { parseInputEvent } from "@rd/protocol";

test("releaseEvents produces kup/mup for held keys and buttons", () => {
  const evs = releaseEvents(["ShiftLeft", "KeyA"], [0, 2]);
  // every event is protocol-valid
  evs.forEach((e) => expect(() => parseInputEvent(e)).not.toThrow());
  expect(evs).toContainEqual({ t: "kup", code: "ShiftLeft" });
  expect(evs).toContainEqual({ t: "kup", code: "KeyA" });
  expect(evs).toContainEqual({ t: "mup", button: "left" });
  expect(evs).toContainEqual({ t: "mup", button: "right" });
  expect(evs.length).toBe(4);
});

test("releaseEvents skips unknown button ids", () => {
  expect(releaseEvents([], [5])).toEqual([]);
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `npm test -- rtc.test`
Expected: FAIL — `releaseEvents` not exported.

- [ ] **Step 3: Implement `releaseEvents`**

Add to `packages/web/src/rtc.ts` (uses the existing `mouseButtonName`):

```ts
/** The kup/mup events needed to release a set of currently-held keys/buttons
 *  (used when the capture surface loses focus so nothing sticks down remotely). */
export function releaseEvents(keys: string[], buttons: number[]): InputEvent[] {
  const out: InputEvent[] = [];
  for (const code of keys) out.push({ t: "kup", code });
  for (const b of buttons) {
    const button = mouseButtonName(b);
    if (button) out.push({ t: "mup", button });
  }
  return out;
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `npm test -- rtc.test`
Expected: PASS.

- [ ] **Step 5: Track pressed state + release on blur in SessionView**

In `packages/web/src/pages/SessionView.tsx`:

Import `releaseEvents`. Add refs for the held sets near the other refs:

```tsx
  const pressedKeys = useRef<Set<string>>(new Set());
  const pressedButtons = useRef<Set<number>>(new Set());
```

Update the handlers to track state (add to the existing `emit` calls, do not remove them):
- in `onKeyDown` (the non-Escape path): `pressedKeys.current.add(e.code);` before `emit({ t: "kdown", code: e.code })`.
- in `onKeyUp`: `pressedKeys.current.delete(e.code);`.
- in `onMouseDown`: after computing `button`, `pressedButtons.current.add(e.button);`.
- in `onMouseUp`: `pressedButtons.current.delete(e.button);`.

Add a release-all helper and call it on blur + unmount:

```tsx
  function releaseAll() {
    for (const ev of releaseEvents([...pressedKeys.current], [...pressedButtons.current])) {
      sessionRef.current?.sendInput(ev);
    }
    pressedKeys.current.clear();
    pressedButtons.current.clear();
  }
```

Add `onBlur={releaseAll}` and `onMouseLeave={releaseAll}` to the `<video>` element, and call `releaseAll()` in the `useEffect` cleanup (the one that closes the session), before `session.close()`. (Esc already blurs the surface — that triggers `onBlur` → `releaseAll`.)

- [ ] **Step 6: Build + typecheck + commit**

Run: `npm run -w @rd/web build && npm run typecheck && npm test`
Expected: build clean, typecheck clean, tests pass (62 + 2 new = 64... i.e. all green).

```bash
git add packages/web/src/rtc.ts packages/web/src/rtc.test.ts packages/web/src/pages/SessionView.tsx
git commit -m "fix(web): release held keys/buttons when the video surface loses focus

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: Smoke doc note

**Files:** Modify `docs/superpowers/plan4-video-smoke.md`

- [ ] **Step 1: Add hardening notes**

Append a short "## Hardening (Plan 5)" section to `docs/superpowers/plan4-video-smoke.md` noting the verifiable behaviors: (a) video arrives at 720p/30fps (check the agent isn't pegging CPU at native retina/60fps); (b) disconnecting the browser / closing the tab stops agent capture (no lingering CPU) and releases any held keys; (c) holding a modifier then pressing Esc / moving the pointer off the video releases it on the被控端; (d) with a differing aspect ratio, pointer alignment tracks the visible video, not the letterbox bars.

```markdown
## Hardening (Plan 5) — what to verify
- **720p/30fps at the source:** the agent captures at 1280×720/30fps (not native retina/60fps); CPU should be markedly lower than the first Plan 4 cut.
- **Capture stops on disconnect:** close the browser tab / disconnect → the agent stops capturing (no lingering CPU), and reconnecting does not accumulate capture load.
- **No stuck keys:** hold Shift (or a mouse button), then press Esc / move the pointer off the video / close the tab → the被控端 releases it (no stuck modifier/button).
- **Letterbox coords:** if the remote aspect ratio differs from the `<video>` box, the cursor tracks the visible image; positions over the black bars clamp to the edge.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/plan4-video-smoke.md
git commit -m "docs: Plan 5 hardening smoke notes (720p/30fps, capture stop, stuck-key, letterbox)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## After all tasks
- Whole-branch review; then update `docs/BACKLOG.md` (mark Plan 5 done, drop the resolved follow-ups, refresh counts); then `superpowers:finishing-a-development-branch`.

## Self-Review (spec coverage)
- Spec §2.1 SCStream lifecycle → Task 1. §2.2 SCK 720p/30fps → Task 1. §2.3 agent stuck-key → Task 2. §2.4 env serial → Task 3. §2.5 convert → Task 3. §2.6 letterbox → Task 4. §2.7 web stuck-key → Task 5. §4 tests → per-task units + manual smoke. §5 split → Parallel Groups.
- Type consistency: `PressedState`/`pending_releases` used only in Task 2; `contentRect`/`releaseEvents` defined in Tasks 4/5 and consumed in SessionView same task; `SckCapturer::new` used in Task 1's `make_source` + ignored test.
- Leaf-API risk flagged: `screencapturekit` `with_width/with_height/with_fps/stop_capture` (Task 1) verified-or-reported.
