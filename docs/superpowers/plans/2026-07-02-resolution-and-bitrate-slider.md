# Resolution Hot-Switch + Bitrate Slider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the web client hot-switch capture resolution (sd/hd/native, default hd) and fine-tune bitrate with a slider, without dropping the session.

**Architecture:** A new `resolution` control message flows web → agent over the existing "control" data channel. The agent maps preset → capture size and forwards a command into the video pipeline, whose command channel is generalized from `Receiver<u32>` (bitrate only) to `Receiver<PipelineCmd>`; on a resolution command the pipeline thread swaps its capturer (via an injected source factory) and resets the encoder. The web adds a resolution dropdown and a debounced bitrate slider linked to the existing quality presets.

**Tech Stack:** TypeScript (Vitest, React Testing Library/jsdom), Rust (webrtc-rs, screencapturekit, openh264, serde).

## Global Constraints

- Wire tags must match byte-for-byte between TS and Rust: `{"t":"resolution","preset":"sd"|"hd"|"native"}`.
- Bitrate clamp stays `[250_000, 20_000_000]` (protocol constants `QUALITY_MIN_BPS`/`QUALITY_MAX_BPS`).
- Resolution mapping (dw,dh = display logical size): `sd` → `fit_aspect(dw, dh, 1280, 720)`; `hd` → `(even(dw), even(dh))`; `native` → `(even(dw*2), even(dh*2))`. Display-query failure falls back to `(1280, 720)`.
- Session default resolution is `hd` on BOTH sides (agent initial capture + web dropdown initial state).
- Encoder must be explicitly reset on resolution change (do NOT rely on openh264 adapting to changed frame dims) and the next frame must be an IDR.
- H.264 needs even dimensions — all sizes pass through `even()`.
- Build/test commands: web has NO typecheck script — use `npm run -w @rd/web build`; root `npm run typecheck` covers protocol+server; cargo needs `export PATH="$HOME/.cargo/bin:$PATH"`; agent modules are registered in `agent/src/lib.rs` (all already registered for this plan — no lib.rs changes).
- Commit messages in English, ending with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.

---

### Task 1: Protocol (TS) — `resolution` control message

**Files:**
- Modify: `packages/protocol/src/control.ts`
- Test: `packages/protocol/src/control.test.ts`

**Interfaces:**
- Produces: `type ResolutionPreset = "sd" | "hd" | "native"`, `interface Resolution { t: "resolution"; preset: ResolutionPreset }`, added to the `ControlMessage` union and accepted by `parseControlMessage`. `packages/protocol/src/index.ts` already re-exports `./control.js` wholesale (verify with `grep -n "control" packages/protocol/src/index.ts`; if it lists named exports, add `ResolutionPreset` and `Resolution` there).

- [ ] **Step 1: Write the failing tests** — append to `packages/protocol/src/control.test.ts`:

```ts
describe("resolution", () => {
  it("parses each valid preset", () => {
    for (const preset of ["sd", "hd", "native"] as const) {
      expect(parseControlMessage({ t: "resolution", preset })).toEqual({ t: "resolution", preset });
    }
  });

  it("rejects unknown or missing presets", () => {
    expect(() => parseControlMessage({ t: "resolution", preset: "8k" })).toThrow(/resolution\.preset/);
    expect(() => parseControlMessage({ t: "resolution" })).toThrow(/resolution\.preset/);
    expect(() => parseControlMessage({ t: "resolution", preset: 2 })).toThrow(/resolution\.preset/);
  });
});
```

- [ ] **Step 2: Run to verify failure**

Run: `npm test -w @rd/protocol`
Expected: FAIL — `unknown control type: resolution`.

- [ ] **Step 3: Implement** — in `packages/protocol/src/control.ts`:

Add after the `ClipMode` type (line 1 area):

```ts
export type ResolutionPreset = "sd" | "hd" | "native";
```

Add after the `Quality` interface:

```ts
export interface Resolution { t: "resolution"; preset: ResolutionPreset; }
```

Change the union:

```ts
export type ControlMessage = ClipSet | ClipRequest | ClipModeMsg | Quality | Resolution;
```

Add next to `CLIP_MODES`:

```ts
const RESOLUTION_PRESETS = new Set<ResolutionPreset>(["sd", "hd", "native"]);
```

Add a `case` to `parseControlMessage` before `default`:

```ts
    case "resolution": {
      if (typeof raw.preset !== "string" || !RESOLUTION_PRESETS.has(raw.preset as ResolutionPreset))
        throw new Error("invalid resolution.preset");
      return { t: "resolution", preset: raw.preset as ResolutionPreset };
    }
```

- [ ] **Step 4: Run tests + typecheck**

Run: `npm test -w @rd/protocol && npm run typecheck`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/protocol/src/control.ts packages/protocol/src/control.test.ts
git commit -m "feat(protocol): resolution control message (sd/hd/native)"
```

---

### Task 2: Protocol (Rust) — `Resolution` variant + `ResolutionPreset`

**Files:**
- Modify: `agent/src/control.rs`

**Interfaces:**
- Consumes: wire format from Task 1.
- Produces: `ControlMessage::Resolution { preset: ResolutionPreset }`; `pub enum ResolutionPreset { Sd, Hd, Native }` (Copy). Later tasks import `crate::control::ResolutionPreset`.

- [ ] **Step 1: Write the failing test** — append cases inside the existing `parses_each_variant_from_web_tags` array in `agent/src/control.rs` tests:

```rust
            (r#"{"t":"resolution","preset":"native"}"#, ControlMessage::Resolution { preset: ResolutionPreset::Native }),
            (r#"{"t":"resolution","preset":"sd"}"#, ControlMessage::Resolution { preset: ResolutionPreset::Sd }),
            (r#"{"t":"resolution","preset":"hd"}"#, ControlMessage::Resolution { preset: ResolutionPreset::Hd }),
```

And a new test in the same `mod tests`:

```rust
    #[test]
    fn rejects_unknown_resolution_preset() {
        assert!(serde_json::from_str::<ControlMessage>(r#"{"t":"resolution","preset":"8k"}"#).is_err());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml control::`
Expected: compile FAIL — no variant `Resolution`.

- [ ] **Step 3: Implement** — in `agent/src/control.rs`, add to the `ControlMessage` enum after `Quality`:

```rust
    Resolution { preset: ResolutionPreset },
```

Add below `ClipMode`:

```rust
/// Capture-resolution presets, mirrored from the web's ResolutionPreset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResolutionPreset {
    Sd,
    Hd,
    Native,
}
```

- [ ] **Step 4: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml control:: && cargo clippy --manifest-path agent/Cargo.toml -- -D warnings`
Expected: PASS (a `match` on `ControlMessage` in `webrtc_peer.rs` will now fail to compile — see note). If `wire_control`'s match breaks the build, add a temporary arm there:

```rust
                ControlMessage::Resolution { .. } => { /* wired in a later task */ }
```

- [ ] **Step 5: Commit**

```bash
git add agent/src/control.rs agent/src/webrtc_peer.rs
git commit -m "feat(agent): parse resolution control message"
```

---

### Task 3: Agent — preset → capture size mapping

**Files:**
- Modify: `agent/src/video/mod.rs`

**Interfaces:**
- Consumes: `crate::control::ResolutionPreset` (Task 2); existing `fit_aspect`, `even`, `sck_capturer::main_display_size`.
- Produces: `pub fn preset_size(preset: ResolutionPreset, dw: u32, dh: u32) -> (u32, u32)` (pure) and `pub fn preset_capture_size(preset: ResolutionPreset) -> (u32, u32)` (queries display). Task 5 calls `preset_capture_size`.

- [ ] **Step 1: Write the failing tests** — add a test module in `agent/src/video/mod.rs` next to `fit_aspect_tests`:

```rust
#[cfg(test)]
mod preset_size_tests {
    use super::preset_size;
    use crate::control::ResolutionPreset;

    #[test]
    fn sd_fits_720p_box() {
        // 1512x982 logical display → aspect-fit inside 1280x720, even dims.
        assert_eq!(preset_size(ResolutionPreset::Sd, 1512, 982), (1108, 720));
    }

    #[test]
    fn hd_is_display_logical_size_evened() {
        assert_eq!(preset_size(ResolutionPreset::Hd, 1512, 982), (1512, 982));
        assert_eq!(preset_size(ResolutionPreset::Hd, 1513, 983), (1512, 982));
    }

    #[test]
    fn native_is_double_logical_evened() {
        assert_eq!(preset_size(ResolutionPreset::Native, 1512, 982), (3024, 1964));
    }
}
```

Note: `preset_size(Sd, 1512, 982)` = `fit_aspect(1512, 982, 1280, 720)`: scale = min(1280/1512, 720/982) = 720/982 → w = round(1512·0.7332) = 1108 (even), h = 720.

- [ ] **Step 2: Run to verify failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml preset_size`
Expected: compile FAIL — `preset_size` not found.

- [ ] **Step 3: Implement** — in `agent/src/video/mod.rs`, after `fit_aspect`:

```rust
use crate::control::ResolutionPreset;

/// Map a resolution preset to a capture size for a display of logical size
/// `dw`×`dh`. Pure so the mapping is unit-testable without a display.
pub fn preset_size(preset: ResolutionPreset, dw: u32, dh: u32) -> (u32, u32) {
    match preset {
        ResolutionPreset::Sd => fit_aspect(dw, dh, 1280, 720),
        ResolutionPreset::Hd => (even(dw).max(2), even(dh).max(2)),
        // Retina physical pixels (2× logical). On a non-Retina display this is
        // an SCK upscale — accepted per the design spec.
        ResolutionPreset::Native => (even(dw * 2).max(2), even(dh * 2).max(2)),
    }
}

/// Capture size for `preset` on the main display. Falls back to 1280×720 when
/// the display query fails (same fallback as target_capture_size).
pub fn preset_capture_size(preset: ResolutionPreset) -> (u32, u32) {
    #[cfg(target_os = "macos")]
    {
        match sck_capturer::main_display_size() {
            Ok((dw, dh)) => return preset_size(preset, dw, dh),
            Err(e) => {
                tracing::warn!("main display size query failed ({e}); using 1280x720");
            }
        }
    }
    (1280, 720)
}
```

(`use` goes at the top of the file with the other imports, not mid-file.)

- [ ] **Step 4: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml preset_size && cargo clippy --manifest-path agent/Cargo.toml -- -D warnings`
Expected: PASS. (On non-mac targets clippy may flag unused `preset` — the `match` uses it; fine.)

- [ ] **Step 5: Commit**

```bash
git add agent/src/video/mod.rs
git commit -m "feat(agent): preset -> capture size mapping (sd/hd/native)"
```

---

### Task 4: Agent — pipeline command channel + capturer swap

**Files:**
- Modify: `agent/src/video/pipeline.rs`
- Modify: `agent/src/video/mod.rs` (VideoEncoder trait: add `reset` default method)
- Modify: `agent/src/video/openh264_encoder.rs` (implement `reset`)
- Modify: `agent/src/webrtc_peer.rs` (call-site signature updates ONLY, keep behavior)

**Interfaces:**
- Consumes: `ScreenCapturer`, `VideoEncoder`, `SampleSink` traits.
- Produces:
  - `pub enum PipelineCmd { Bitrate(u32), Resolution(u32, u32) }` in `pipeline.rs`.
  - `pub type SourceFactory = Box<dyn Fn(u32, u32) -> Box<dyn ScreenCapturer> + Send>;` in `pipeline.rs`.
  - `VideoPipeline::start(capturer, encoder, sink, dst_w, dst_h, keyframe_interval, cmd_rx: mpsc::Receiver<PipelineCmd>, source_factory: SourceFactory)`.
  - `VideoEncoder::reset(&mut self)` — default no-op; openh264 impl rebuilds codec at current bitrate and forces an IDR next frame. Task 5 relies on `PipelineCmd::Resolution` triggering `reset`.

- [ ] **Step 1: Write the failing tests** — in `agent/src/video/pipeline.rs`, replace the `bitrate_tests` module with:

```rust
#[cfg(test)]
mod cmd_tests {
    use super::*;
    use crate::video::{EncodedSample, Frame, I420, SampleSink, ScreenCapturer, VideoEncoder};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Emits 2x2 frames forever; records its size into `started` on start().
    struct LoopingSource {
        w: u32,
        h: u32,
        started: Arc<Mutex<Vec<(u32, u32)>>>,
    }
    impl ScreenCapturer for LoopingSource {
        fn start(&mut self, sink: std::sync::mpsc::Sender<Frame>) -> anyhow::Result<()> {
            self.started.lock().unwrap().push((self.w, self.h));
            let (w, h) = (self.w, self.h);
            std::thread::spawn(move || loop {
                let f = Frame {
                    width: w, height: h, stride: (w * 4) as usize,
                    data: vec![0u8; (w * h * 4) as usize], ts_micros: 0,
                };
                if sink.send(f).is_err() { break; }
                std::thread::sleep(Duration::from_millis(5));
            });
            Ok(())
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Ev { Bitrate(u32), Reset }
    struct RecordingEncoder { events: Arc<Mutex<Vec<Ev>>> }
    impl VideoEncoder for RecordingEncoder {
        fn encode(&mut self, _f: &I420, _idr: bool) -> anyhow::Result<EncodedSample> {
            Ok(EncodedSample { data: vec![0], duration: Duration::from_millis(33), keyframe: true })
        }
        fn set_bitrate(&mut self, bps: u32) { self.events.lock().unwrap().push(Ev::Bitrate(bps)); }
        fn reset(&mut self) { self.events.lock().unwrap().push(Ev::Reset); }
    }

    struct NullSink;
    impl SampleSink for NullSink {
        fn write(&self, _s: EncodedSample) {}
    }

    fn wait_until(deadline_ms: u64, mut ok: impl FnMut() -> bool) -> bool {
        let t0 = std::time::Instant::now();
        while t0.elapsed() < Duration::from_millis(deadline_ms) {
            if ok() { return true; }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn pipeline_applies_queued_bitrate_cmds_in_order() {
        let started = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel::<PipelineCmd>();
        tx.send(PipelineCmd::Bitrate(1_500_000)).unwrap();
        tx.send(PipelineCmd::Bitrate(6_000_000)).unwrap();
        let st = started.clone();
        let _p = VideoPipeline::start(
            Box::new(LoopingSource { w: 2, h: 2, started: started.clone() }),
            Box::new(RecordingEncoder { events: events.clone() }),
            Arc::new(NullSink), 2, 2, 60, rx,
            Box::new(move |w, h| Box::new(LoopingSource { w, h, started: st.clone() })),
        );
        assert!(wait_until(2000, || events.lock().unwrap().len() >= 2));
        assert_eq!(&events.lock().unwrap()[..2], &[Ev::Bitrate(1_500_000), Ev::Bitrate(6_000_000)]);
    }

    #[test]
    fn resolution_cmd_swaps_capturer_and_resets_encoder() {
        let started = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel::<PipelineCmd>();
        let st = started.clone();
        let _p = VideoPipeline::start(
            Box::new(LoopingSource { w: 2, h: 2, started: started.clone() }),
            Box::new(RecordingEncoder { events: events.clone() }),
            Arc::new(NullSink), 2, 2, 60, rx,
            Box::new(move |w, h| Box::new(LoopingSource { w, h, started: st.clone() })),
        );
        // let it run, then switch to 4x4
        assert!(wait_until(2000, || !started.lock().unwrap().is_empty()));
        tx.send(PipelineCmd::Resolution(4, 4)).unwrap();
        assert!(wait_until(2000, || started.lock().unwrap().len() >= 2), "factory not invoked");
        assert_eq!(started.lock().unwrap()[1], (4, 4));
        assert!(wait_until(2000, || events.lock().unwrap().contains(&Ev::Reset)), "encoder not reset");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml cmd_tests`
Expected: compile FAIL — `PipelineCmd` not found / `reset` not a trait member.

- [ ] **Step 3: Add `reset` to the VideoEncoder trait** — in `agent/src/video/mod.rs`:

```rust
/// Encodes I420 frames to H.264 Annex-B. `force_idr` requests a keyframe.
pub trait VideoEncoder: Send {
    fn encode(&mut self, frame: &I420, force_idr: bool) -> anyhow::Result<EncodedSample>;
    /// Change the target bitrate for subsequent frames. Default: no-op.
    fn set_bitrate(&mut self, _bitrate_bps: u32) {}
    /// Drop internal codec state so the next frame re-initializes at the
    /// incoming frame's dimensions and emits fresh SPS/PPS + IDR (used when the
    /// capture resolution changes). Default: no-op.
    fn reset(&mut self) {}
}
```

- [ ] **Step 4: Implement `reset` for openh264** — in `agent/src/video/openh264_encoder.rs`, add inside `impl VideoEncoder for Openh264Encoder`:

```rust
    fn reset(&mut self) {
        match Self::build_encoder(self.bitrate_bps, self.fps) {
            Ok(enc) => {
                self.encoder = enc;
                // A fresh encoder must open with a keyframe so the decoder re-syncs.
                self.force_idr_next = true;
            }
            Err(e) => tracing::warn!("encoder reset failed, keeping current encoder: {e}"),
        }
    }
```

And add to that file's test module:

```rust
    #[test]
    fn reset_then_encode_emits_keyframe_with_parameter_sets() {
        let mut enc = Openh264Encoder::new(64, 64, 1_000_000, 30.0).unwrap();
        let _ = enc.encode(&gray_i420(64, 64), true).unwrap();
        enc.reset();
        let s = enc.encode(&gray_i420(64, 64), false).unwrap();
        assert!(s.keyframe);
        let types = nal_types(&s.data);
        assert!(types.contains(&7) && types.contains(&8) && types.contains(&5), "got {types:?}");
    }
```

- [ ] **Step 5: Rework the pipeline** — replace `VideoPipeline::start` in `agent/src/video/pipeline.rs`:

```rust
use crate::video::convert::bgra_to_i420;
use crate::video::{ScreenCapturer, SampleSink, VideoEncoder};
use std::sync::mpsc;
use std::sync::Arc;

/// Commands applied by the pipeline thread between frames.
pub enum PipelineCmd {
    Bitrate(u32),
    /// Switch capture to a new size: swap the capturer (via the source factory)
    /// and reset the encoder so the stream re-opens with SPS/PPS+IDR.
    Resolution(u32, u32),
}

/// Builds a capturer at the requested size (used for hot resolution switches).
pub type SourceFactory = Box<dyn Fn(u32, u32) -> Box<dyn ScreenCapturer> + Send>;

/// Runs capture → BGRA→I420 → encode → sink on a dedicated thread. The first
/// frame and every `keyframe_interval`-th frame force an IDR. Dropping the
/// `VideoPipeline` closes the frame channel and the thread exits.
pub struct VideoPipeline {
    _stop: mpsc::Sender<()>,
}

impl VideoPipeline {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        mut capturer: Box<dyn ScreenCapturer>,
        mut encoder: Box<dyn VideoEncoder>,
        sink: Arc<dyn SampleSink>,
        dst_w: usize,
        dst_h: usize,
        keyframe_interval: u64,
        cmd_rx: mpsc::Receiver<PipelineCmd>,
        source_factory: SourceFactory,
    ) -> VideoPipeline {
        let (frame_tx, frame_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        // capture delivers frames onto frame_tx; if start() fails, log and bail.
        if let Err(e) = capturer.start(frame_tx) {
            tracing::error!("video capture failed to start: {e}");
            return VideoPipeline { _stop: stop_tx };
        }
        std::thread::spawn(move || {
            let mut capturer = capturer; // owned by the thread; swapped on resolution change
            let mut frame_rx = frame_rx;
            let (mut dst_w, mut dst_h) = (dst_w, dst_h);
            let mut n: u64 = 0;
            let mut force_next_idr = false;
            // Stop when the VideoPipeline is dropped: its `_stop` Sender drops,
            // so try_recv() returns Disconnected (NOT Empty). Loop only while
            // Empty; anything else (including Disconnected) exits.
            while let Err(mpsc::TryRecvError::Empty) = stop_rx.try_recv() {
                // Apply queued control commands between frames.
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        PipelineCmd::Bitrate(bps) => encoder.set_bitrate(bps),
                        PipelineCmd::Resolution(w, h) => {
                            // Drop the old capturer first (stops its stream); a
                            // fresh channel discards stale frames at the old size.
                            let old = (dst_w as u32, dst_h as u32);
                            drop(capturer);
                            match start_source(&source_factory, w, h) {
                                Ok((cap, rx)) => {
                                    capturer = cap;
                                    frame_rx = rx;
                                    dst_w = w as usize;
                                    dst_h = h as usize;
                                    encoder.reset();
                                    force_next_idr = true;
                                }
                                Err(e) => {
                                    tracing::warn!("capture restart at {w}x{h} failed ({e}); restoring {}x{}", old.0, old.1);
                                    match start_source(&source_factory, old.0, old.1) {
                                        Ok((cap, rx)) => {
                                            capturer = cap;
                                            frame_rx = rx;
                                            encoder.reset();
                                            force_next_idr = true;
                                        }
                                        Err(e) => {
                                            // No capturer left: video freezes but the
                                            // session (input/clipboard) stays alive.
                                            tracing::error!("capture restore failed, video stopped: {e}");
                                            return;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                let frame = match frame_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(f) => f,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let i420 = bgra_to_i420(&frame, dst_w, dst_h);
                let force_idr = n.is_multiple_of(keyframe_interval) || force_next_idr;
                force_next_idr = false;
                match encoder.encode(&i420, force_idr) {
                    Ok(sample) => sink.write(sample),
                    Err(e) => tracing::warn!("encode failed on frame {n}: {e}"),
                }
                n += 1;
            }
        });
        VideoPipeline { _stop: stop_tx }
    }
}

/// Build + start a capturer at `w`×`h`, returning it with its frame receiver.
fn start_source(
    factory: &SourceFactory,
    w: u32,
    h: u32,
) -> anyhow::Result<(Box<dyn ScreenCapturer>, mpsc::Receiver<crate::video::Frame>)> {
    let (tx, rx) = mpsc::channel();
    let mut cap = factory(w, h);
    cap.start(tx)?;
    Ok((cap, rx))
}
```

Note the resolution-switch failure path `return`s out of the thread closure — the loop cannot continue without a frame source. Keep the `Disconnected => break` on the frame channel: after a successful swap the old channel is gone, so the new `frame_rx` is what's polled.

- [ ] **Step 6: Update the call site (behavior-preserving)** — in `agent/src/webrtc_peer.rs`:

Import (top of file, with the other `crate::video` imports): add `PipelineCmd` to the existing `pipeline` import (currently `use ...video::pipeline::VideoPipeline` — check with `grep -n "VideoPipeline" agent/src/webrtc_peer.rs`), e.g.:

```rust
use crate::video::pipeline::{PipelineCmd, VideoPipeline};
```

Change the channel (line ~404):

```rust
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<PipelineCmd>();
```

Change `wire_control`'s signature and Quality arm (lines ~196, ~231):

```rust
fn wire_control(dc: Arc<RTCDataChannel>, cmd_tx: Sender<PipelineCmd>, last_clipboard: Arc<Mutex<String>>) {
```

```rust
                ControlMessage::Quality { bitrate_bps } => {
                    let clamped = bitrate_bps.clamp(250_000, 20_000_000);
                    let _ = cmd_tx.send(PipelineCmd::Bitrate(clamped));
                }
```

(rename the `let bitrate_tx = bitrate_tx.clone();` capture inside `on_message` to `let cmd_tx = cmd_tx.clone();` accordingly, and `let ctl_bitrate_tx = bitrate_tx.clone();` at the call site to `let ctl_cmd_tx = cmd_tx.clone();` / `wire_control(dc, ctl_cmd_tx.clone(), ...)`).

Change the pipeline start (line ~450):

```rust
        let capturer = make_source(dst_w, dst_h, fps);
        let factory: crate::video::pipeline::SourceFactory =
            Box::new(move |w, h| make_source(w, h, fps));
        let video = VideoPipeline::start(
            capturer, encoder, sink, dst_w as usize, dst_h as usize, 60, cmd_rx, factory,
        );
```

- [ ] **Step 7: Run the full agent suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml -- -D warnings`
Expected: PASS (including `cmd_tests` and the new openh264 `reset` test).

- [ ] **Step 8: Commit**

```bash
git add agent/src/video/pipeline.rs agent/src/video/mod.rs agent/src/video/openh264_encoder.rs agent/src/webrtc_peer.rs
git commit -m "feat(agent): pipeline command channel with hot capturer swap"
```

---

### Task 5: Agent — wire resolution message + default hd

**Files:**
- Modify: `agent/src/webrtc_peer.rs`

**Interfaces:**
- Consumes: `preset_capture_size` (Task 3), `PipelineCmd::Resolution` (Task 4), `ControlMessage::Resolution`/`ResolutionPreset` (Task 2).
- Produces: complete agent-side behavior; nothing downstream.

- [ ] **Step 1: Wire the Resolution arm** — in `wire_control`'s match (replacing the Task 2 placeholder arm if present):

```rust
                ControlMessage::Resolution { preset } => {
                    let (w, h) = crate::video::preset_capture_size(preset);
                    tracing::info!("resolution preset {preset:?} -> {w}x{h}");
                    let _ = cmd_tx.send(PipelineCmd::Resolution(w, h));
                }
```

- [ ] **Step 2: Default the session to hd** — replace the initial size (line ~427):

```rust
        // Capture at the display's aspect ratio. Default preset is hd (the
        // display's logical size) — the web's resolution dropdown matches.
        let fps = 30u32;
        let (dst_w, dst_h) =
            crate::video::preset_capture_size(crate::control::ResolutionPreset::Hd);
```

(The old comment block referencing `target_capture_size(1280, 720)` goes away. `target_capture_size` itself stays — `fit_aspect`/fallback logic is still used via `preset_size`; if `target_capture_size` becomes dead code, delete it and its callers/tests accordingly — check with `grep -rn "target_capture_size" agent/src packages`.)

- [ ] **Step 3: Run the full agent suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml -- -D warnings`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add agent/src/webrtc_peer.rs agent/src/video/mod.rs
git commit -m "feat(agent): hot resolution switch via control channel, default hd"
```

---

### Task 6: Web — resolution dropdown

**Files:**
- Modify: `packages/web/src/pages/SessionView.tsx`
- Test: `packages/web/src/pages/SessionView.test.tsx`

**Interfaces:**
- Consumes: `ResolutionPreset` type from `@rd/protocol` (Task 1); existing `sessionRef.current?.sendControl`.
- Produces: `data-testid="resolution-select"` used by tests/e2e.

- [ ] **Step 1: Write the failing test** — append to the describe block in `SessionView.test.tsx`:

```tsx
  it("sends a resolution message when the preset changes, default hd", () => {
    render(<SessionView token="t" device={device} onExit={() => {}} />);
    act(() => h.opts!.onState("connected"));
    const sel = screen.getByTestId("resolution-select") as HTMLSelectElement;
    expect(sel.value).toBe("hd");
    fireEvent.change(sel, { target: { value: "native" } });
    expect(h.session.sendControl).toHaveBeenCalledWith({ t: "resolution", preset: "native" });
  });
```

- [ ] **Step 2: Run to verify failure**

Run: `npm test -w @rd/web`
Expected: FAIL — `resolution-select` not found.

- [ ] **Step 3: Implement** — in `SessionView.tsx`:

Add to the imports from `@rd/protocol` (check the existing import line with `grep -n "@rd/protocol" packages/web/src/pages/SessionView.tsx`): `type ResolutionPreset`.

Add next to `QUALITY_PRESETS` (top of file):

```tsx
const RESOLUTION_PRESETS: { label: string; preset: ResolutionPreset }[] = [
  { label: "流畅 720p", preset: "sd" },
  { label: "高清", preset: "hd" },
  { label: "视网膜", preset: "native" },
];
```

Add state next to `bitrate` (line ~84):

```tsx
  const [resolution, setResolution] = useState<ResolutionPreset>("hd");
```

Add callback next to `onQuality` (line ~141):

```tsx
  const onResolution = useCallback((preset: ResolutionPreset) => {
    setResolution(preset);
    sessionRef.current?.sendControl({ t: "resolution", preset });
  }, []);
```

Add the select in the toolbar, directly BEFORE the quality select (line ~326):

```tsx
          <select
            data-testid="resolution-select"
            value={resolution}
            disabled={!connected}
            onChange={(e) => onResolution(e.target.value as ResolutionPreset)}
            style={{ fontSize: 12 }}
          >
            {RESOLUTION_PRESETS.map((r) => (
              <option key={r.preset} value={r.preset}>{r.label}</option>
            ))}
          </select>
```

- [ ] **Step 4: Run tests + build**

Run: `npm test -w @rd/web && npm run -w @rd/web build`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/web/src/pages/SessionView.tsx packages/web/src/pages/SessionView.test.tsx
git commit -m "feat(web): resolution preset selector (720p/hd/retina)"
```

---

### Task 7: Web — bitrate slider linked to presets

**Files:**
- Modify: `packages/web/src/pages/SessionView.tsx`
- Test: `packages/web/src/pages/SessionView.test.tsx`

**Interfaces:**
- Consumes: `QUALITY_MIN_BPS`, `QUALITY_MAX_BPS` from `@rd/protocol`; existing `bitrate` state, `onQuality`, `QUALITY_PRESETS`.
- Produces: `data-testid="bitrate-slider"`; quality select shows 自定义 when the slider value matches no preset.

- [ ] **Step 1: Write the failing tests** — append to `SessionView.test.tsx`:

```tsx
  it("slider sends a debounced quality message and flips the preset to 自定义", () => {
    vi.useFakeTimers();
    try {
      render(<SessionView token="t" device={device} onExit={() => {}} />);
      act(() => h.opts!.onState("connected"));
      const slider = screen.getByTestId("bitrate-slider") as HTMLInputElement;
      fireEvent.change(slider, { target: { value: "4750000" } });
      // debounced: nothing sent yet
      expect(h.session.sendControl).not.toHaveBeenCalledWith(
        expect.objectContaining({ t: "quality" }),
      );
      act(() => vi.advanceTimersByTime(250));
      expect(h.session.sendControl).toHaveBeenCalledWith({ t: "quality", bitrateBps: 4750000 });
      const quality = screen.getByTestId("quality-select") as HTMLSelectElement;
      expect(quality.value).toBe("custom");
    } finally {
      vi.useRealTimers();
    }
  });

  it("choosing a preset moves the slider to the preset bitrate", () => {
    render(<SessionView token="t" device={device} onExit={() => {}} />);
    act(() => h.opts!.onState("connected"));
    fireEvent.change(screen.getByTestId("quality-select"), { target: { value: "6000000" } });
    expect((screen.getByTestId("bitrate-slider") as HTMLInputElement).value).toBe("6000000");
  });
```

- [ ] **Step 2: Run to verify failure**

Run: `npm test -w @rd/web`
Expected: FAIL — `bitrate-slider` not found.

- [ ] **Step 3: Implement** — in `SessionView.tsx`:

Add `QUALITY_MIN_BPS` and `QUALITY_MAX_BPS` to the `@rd/protocol` import.

Add a debounce ref near the other refs (~line 126) and the slider callback next to `onQuality`:

```tsx
  const sliderTimer = useRef<number | null>(null);

  const onSlider = useCallback((bps: number) => {
    setBitrate(bps);
    if (sliderTimer.current !== null) window.clearTimeout(sliderTimer.current);
    sliderTimer.current = window.setTimeout(() => {
      sliderTimer.current = null;
      sessionRef.current?.sendControl({ t: "quality", bitrateBps: bps });
    }, 200);
  }, []);

  // Clear a pending slider send on unmount.
  useEffect(
    () => () => {
      if (sliderTimer.current !== null) window.clearTimeout(sliderTimer.current);
    },
    [],
  );
```

Replace the quality `<select>` (line ~326) so it shows 自定义 for non-preset values, and add the slider after it:

```tsx
          <select
            data-testid="quality-select"
            value={QUALITY_PRESETS.some((q) => q.bps === bitrate) ? bitrate : "custom"}
            disabled={!connected}
            onChange={(e) => {
              if (e.target.value !== "custom") onQuality(Number(e.target.value));
            }}
            style={{ fontSize: 12 }}
          >
            {!QUALITY_PRESETS.some((q) => q.bps === bitrate) && (
              <option value="custom">自定义</option>
            )}
            {QUALITY_PRESETS.map((q) => (
              <option key={q.bps} value={q.bps}>{q.label}</option>
            ))}
          </select>
          <input
            type="range"
            data-testid="bitrate-slider"
            min={QUALITY_MIN_BPS}
            max={QUALITY_MAX_BPS}
            step={250_000}
            value={bitrate}
            disabled={!connected}
            onChange={(e) => onSlider(Number(e.target.value))}
            style={{ width: 90 }}
            title={`${(bitrate / 1_000_000).toFixed(2)} Mbps`}
          />
          <span style={{ fontSize: 11, minWidth: 64 }}>{(bitrate / 1_000_000).toFixed(2)} Mbps</span>
```

(`onQuality` already sets `setBitrate(bps)`, which moves the slider — the preset→slider linkage needs no extra code.)

- [ ] **Step 4: Run tests + build**

Run: `npm test -w @rd/web && npm run -w @rd/web build`
Expected: PASS, including the two prior quality-preset tests (they select preset values, which still exist as options).

- [ ] **Step 5: Commit**

```bash
git add packages/web/src/pages/SessionView.tsx packages/web/src/pages/SessionView.test.tsx
git commit -m "feat(web): manual bitrate slider linked to quality presets"
```

---

## Final verification (after all tasks)

1. `npm run typecheck && npm test` (root: protocol + server + web suites).
2. `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml -- -D warnings && cargo build --release --manifest-path agent/Cargo.toml`.
3. Manual e2e on this machine (controller session): connect → HUD shows ~1512×982 (hd default) → switch 视网膜 → HUD shows 3024×1964 within ~2s, input still works → switch 流畅 720p → HUD ~1106×720 → drag slider → kbps burst follows and preset shows 自定义.
