# Plan 4 — Screen Capture + H.264 Video (macOS-first) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stream the被控端 macOS main display to the browser: capture → H.264 encode → WebRTC video track → `<video>`, proving the pipeline with a synthetic test pattern first, then swapping in ScreenCaptureKit.

**Architecture:** A new `agent/src/video/` module behind two traits (`ScreenCapturer`, `VideoEncoder`) plus a `VideoPipeline` that runs on a dedicated thread (capture → BGRA→I420 → openh264 → a `SampleSink`). The real sink writes H.264 Annex-B samples to a webrtc-rs `TrackLocalStaticSample` (agent adds a sendonly H264 track; the web offerer adds a recvonly video transceiver). The Plan 3 input data channel is untouched and coexists on the same PeerConnection.

**Tech Stack:** Rust (`openh264` 0.9 software encoder, `yuv` 0.8 color conversion, `screencapturekit` 8 macOS-only, existing `webrtc` 0.11 / `tokio`); TypeScript/React (native `<video>`, existing WebRTC/Vitest).

## Global Constraints

- Platform scope: **macOS-first**. Capture and encode sit behind traits so Windows/Linux/hardware encoders are later increments. Only `SckCapturer` is `#[cfg(target_os = "macos")]`.
- Toolchains: Node ≥ 20; Rust via rustup, `cargo` in `~/.cargo/bin` — prefix every cargo command with `export PATH="$HOME/.cargo/bin:$PATH"`.
- New deps (exact): `openh264 = { version = "0.9", features = ["source"] }`; `yuv = "0.8"`; and under `[target.'cfg(target_os = "macos")'.dependencies]`: `screencapturekit = "8"`.
- Video track: mime `video/H264`, clock rate 90000, track id `"video"`, stream id `"rd-agent"`. Web offerer adds a **recvonly** video transceiver so the SDP has a video m-line; agent adds a **sendonly** H264 `TrackLocalStaticSample`.
- Source selection via env `RD_VIDEO_SOURCE`: `testpattern` → `TestPatternSource`; `screen` (default after Task 5) → `SckCapturer`. Until Task 5 lands, the default is `testpattern`.
- MVP fixed params (constants, no adaptation): target 1280×720, 30 fps, bitrate 3_000_000 bps, keyframe/IDR every 60 frames (~2 s).
- API-verification rule: the exact leaf signatures below (`openh264` `BitRate`/`FrameRate`/`EncodedBitStream::to_vec`, `screencapturekit` method names, `webrtc_media::Sample` fields) are transcribed from docs but may differ slightly by patch version. If a signature fails to compile, the implementer VERIFIES against docs.rs (`openh264` 0.9.3, `screencapturekit` 8.0.0, `webrtc` 0.11.0) and reports the corrected signature — do NOT invent names.
- TDD: failing test first, watch it fail, minimal implement, watch it pass, commit. Frequent commits. Commit messages in English ending with `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- Do not break Plan 3 baselines: agent `cargo test` (23 passed + 1 ignored) stays green (count grows); `cargo clippy --all-targets` clean; Node `npm test` (57) green; `npm run typecheck` clean; `npm run -w @rd/web build` clean.

## File Structure

```
agent/
  Cargo.toml                       # Modify: add openh264, yuv, (macos) screencapturekit
  src/
    lib.rs                         # Modify: add `pub mod video;`
    video/
      mod.rs                       # Create: Frame, I420, EncodedSample, traits, SampleSink, source selection
      testpattern.rs               # Create: TestPatternSource (ScreenCapturer)
      convert.rs                   # Create: bgra_to_i420 (+ scale to 720p)
      openh264_encoder.rs          # Create: Openh264Encoder (VideoEncoder)
      pipeline.rs                  # Create: VideoPipeline (dedicated thread)
      sck_capturer.rs              # Create (cfg macos): SckCapturer (ScreenCapturer)
    webrtc_peer.rs                 # Modify: add H264 track + start VideoPipeline in build()
    permission.rs                  # Modify: add screen-recording permission check (macos)
    main.rs                        # Modify: call the screen-recording check at startup
  tests/
    video_pipeline.rs              # Create: testpattern → encode → SampleSink integration
    video_sdp.rs                   # Create: agent answer has a sendonly H264 video m-line
packages/web/src/
  rtc.ts                           # Modify: addTransceiver(video, recvonly) + ontrack → onRemoteStream
  rtc.test.ts                      # Modify: unit tests for the pure additions
  pages/SessionView.tsx            # Modify: <video> element; move capture handlers onto it
docs/superpowers/
  plan4-video-smoke.md             # Create: manual smoke (test pattern first, then real screen)
```

## Parallel Groups

- **Group A (agent track):** Tasks 1–5, under `agent/`. Sequential — same directory. Order enforces "test-pattern end-to-end before real capture" (Task 4 is the milestone; Task 5 swaps in SCK).
- **Group B (web track):** Tasks 6–7, under `packages/web/`. Disjoint from Group A → dispatch concurrently. Cross-track contract: the SDP video m-line (web recvonly ⇄ agent sendonly H264) and mime `video/H264`.
- **Task 8 (docs):** after A and B.

---

## Task 1: video module skeleton — types, traits, TestPatternSource

**Files:**
- Create: `agent/src/video/mod.rs`, `agent/src/video/testpattern.rs`
- Modify: `agent/src/lib.rs`
- Test: inline `#[cfg(test)]` in `testpattern.rs`

**Interfaces:**
- Produces:
  - `pub struct Frame { pub width: u32, pub height: u32, pub stride: usize, pub data: Vec<u8>, pub ts_micros: u64 }` (BGRA8888)
  - `pub struct I420 { pub width: usize, pub height: usize, pub y: Vec<u8>, pub u: Vec<u8>, pub v: Vec<u8>, pub y_stride: usize, pub uv_stride: usize }`
  - `pub struct EncodedSample { pub data: Vec<u8>, pub duration: std::time::Duration, pub keyframe: bool }`
  - `pub trait ScreenCapturer: Send { fn start(&mut self, sink: std::sync::mpsc::Sender<Frame>) -> anyhow::Result<()>; }`
  - `pub trait VideoEncoder: Send { fn encode(&mut self, frame: &I420, force_idr: bool) -> anyhow::Result<EncodedSample>; }`
  - `pub trait SampleSink: Send + Sync { fn write(&self, sample: EncodedSample); }`
  - `pub struct TestPatternSource { pub width: u32, pub height: u32, pub fps: u32 }` implementing `ScreenCapturer`.

- [ ] **Step 1: Write the failing test**

Create `agent/src/video/testpattern.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn testpattern_emits_frames_of_expected_size_that_change() {
        let mut src = TestPatternSource { width: 64, height: 48, fps: 30 };
        let (tx, rx) = mpsc::channel();
        src.start(tx).unwrap();
        let f0 = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        let f1 = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(f0.width, 64);
        assert_eq!(f0.height, 48);
        assert_eq!(f0.stride, 64 * 4);
        assert_eq!(f0.data.len(), 64 * 48 * 4);
        // successive frames differ (animation)
        assert_ne!(f0.data, f1.data);
        // timestamps are monotonic
        assert!(f1.ts_micros > f0.ts_micros);
    }
}
```

- [ ] **Step 2: Add the module + types and run to verify it fails**

Create `agent/src/video/mod.rs`:

```rust
use std::time::Duration;

pub mod testpattern;

/// One captured frame of raw BGRA8888 pixels. `stride` is bytes per row
/// (>= width*4) to allow row padding from the capture source.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub data: Vec<u8>,
    pub ts_micros: u64,
}

/// Planar YUV 4:2:0 (I420), ready for an H.264 encoder.
pub struct I420 {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
    pub y_stride: usize,
    pub uv_stride: usize,
}

/// An encoded H.264 Annex-B sample plus timing/keyframe metadata.
pub struct EncodedSample {
    pub data: Vec<u8>,
    pub duration: Duration,
    pub keyframe: bool,
}

/// A source of raw screen frames. `start` begins delivering frames on `sink`
/// until the source is dropped.
pub trait ScreenCapturer: Send {
    fn start(&mut self, sink: std::sync::mpsc::Sender<Frame>) -> anyhow::Result<()>;
}

/// Encodes I420 frames to H.264 Annex-B. `force_idr` requests a keyframe.
pub trait VideoEncoder: Send {
    fn encode(&mut self, frame: &I420, force_idr: bool) -> anyhow::Result<EncodedSample>;
}

/// Where the pipeline delivers encoded samples (a WebRTC track in production,
/// a recorder in tests).
pub trait SampleSink: Send + Sync {
    fn write(&self, sample: EncodedSample);
}
```

Add to `agent/src/lib.rs`:

```rust
pub mod video;
```

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib video::testpattern`
Expected: FAIL to compile — `TestPatternSource` not found.

- [ ] **Step 3: Implement `TestPatternSource`**

Prepend to `agent/src/video/testpattern.rs` (above the test module):

```rust
use crate::video::{Frame, ScreenCapturer};
use std::sync::mpsc::Sender;

/// A synthetic animated source (moving gradient + a marching white square) used
/// to prove the encode→transport→browser pipeline without any capture backend.
/// Runs a dedicated thread that emits frames at ~`fps` until `sink` is dropped.
pub struct TestPatternSource {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl ScreenCapturer for TestPatternSource {
    fn start(&mut self, sink: Sender<Frame>) -> anyhow::Result<()> {
        let (w, h, fps) = (self.width, self.height, self.fps.max(1));
        std::thread::spawn(move || {
            let stride = (w as usize) * 4;
            let frame_gap = std::time::Duration::from_micros(1_000_000 / fps as u64);
            let mut n: u64 = 0;
            loop {
                let mut data = vec![0u8; stride * h as usize];
                let phase = (n * 4) as u32;
                for y in 0..h {
                    for x in 0..w {
                        let i = (y as usize) * stride + (x as usize) * 4;
                        // BGRA: animated gradient
                        data[i] = ((x + phase) % 256) as u8; // B
                        data[i + 1] = ((y + phase) % 256) as u8; // G
                        data[i + 2] = ((x + y + phase) % 256) as u8; // R
                        data[i + 3] = 255; // A
                    }
                }
                // marching white square as a motion/time reference
                let sq = 8u32;
                let sx = (n as u32 * 2) % w.saturating_sub(sq).max(1);
                let sy = (n as u32) % h.saturating_sub(sq).max(1);
                for y in sy..(sy + sq).min(h) {
                    for x in sx..(sx + sq).min(w) {
                        let i = (y as usize) * stride + (x as usize) * 4;
                        data[i] = 255;
                        data[i + 1] = 255;
                        data[i + 2] = 255;
                        data[i + 3] = 255;
                    }
                }
                let frame = Frame { width: w, height: h, stride, data, ts_micros: n * frame_gap.as_micros() as u64 };
                if sink.send(frame).is_err() {
                    break; // receiver dropped → stop
                }
                n += 1;
                std::thread::sleep(frame_gap);
            }
        });
        Ok(())
    }
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib video::testpattern`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
git add agent/src/video/mod.rs agent/src/video/testpattern.rs agent/src/lib.rs
git commit -m "feat(agent): video module skeleton + animated test-pattern source

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: BGRA→I420 conversion

**Files:**
- Modify: `agent/Cargo.toml` (add `yuv`), `agent/src/video/mod.rs` (add `pub mod convert;`)
- Create: `agent/src/video/convert.rs`
- Test: inline `#[cfg(test)]` in `convert.rs`

**Interfaces:**
- Consumes: `Frame`, `I420` (Task 1).
- Produces: `pub fn bgra_to_i420(frame: &Frame, dst_w: usize, dst_h: usize) -> I420` — converts (and, when the frame size differs from dst, resizes) a BGRA frame to I420 at `dst_w`×`dst_h`.

- [ ] **Step 1: Add the `yuv` dependency**

In `agent/Cargo.toml` under `[dependencies]`:

```toml
yuv = "0.8"
```

- [ ] **Step 2: Write the failing test**

Create `agent/src/video/convert.rs` with the test first (solid colors have known luma; use tolerant bounds because color-matrix constants vary):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::Frame;

    fn solid_bgra(w: usize, h: usize, b: u8, g: u8, r: u8) -> Frame {
        let stride = w * 4;
        let mut data = vec![0u8; stride * h];
        for px in data.chunks_exact_mut(4) {
            px[0] = b; px[1] = g; px[2] = r; px[3] = 255;
        }
        Frame { width: w as u32, height: h as u32, stride, data, ts_micros: 0 }
    }

    #[test]
    fn i420_dimensions_and_plane_sizes() {
        let f = solid_bgra(16, 16, 0, 0, 0);
        let i = bgra_to_i420(&f, 16, 16);
        assert_eq!((i.width, i.height), (16, 16));
        assert_eq!(i.y.len(), i.y_stride * 16);
        assert_eq!(i.u.len(), i.uv_stride * 8);
        assert_eq!(i.v.len(), i.uv_stride * 8);
    }

    #[test]
    fn black_and_white_luma_ordering() {
        let black = bgra_to_i420(&solid_bgra(16, 16, 0, 0, 0), 16, 16);
        let white = bgra_to_i420(&solid_bgra(16, 16, 255, 255, 255), 16, 16);
        // white luma must be much brighter than black luma
        assert!(black.y[0] < 40, "black luma {} too high", black.y[0]);
        assert!(white.y[0] > 200, "white luma {} too low", white.y[0]);
    }

    #[test]
    fn resizes_to_target() {
        let i = bgra_to_i420(&solid_bgra(32, 32, 0, 0, 0), 16, 16);
        assert_eq!((i.width, i.height), (16, 16));
    }
}
```

- [ ] **Step 3: Register the module and run to verify it fails**

Add to `agent/src/video/mod.rs` (next to `pub mod testpattern;`):

```rust
pub mod convert;
```

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib video::convert`
Expected: FAIL to compile — `bgra_to_i420` not found (first `yuv` build).

- [ ] **Step 4: Implement `bgra_to_i420`**

Prepend to `agent/src/video/convert.rs`. This uses the `yuv` crate's `bgra_to_yuv420` (verify the exact arg order + enum names against docs.rs/yuv/0.8.16 — if it differs, report and adjust; do not guess silently). Resize is done with a simple nearest-neighbour pre-pass when the source size differs from the target (keeps deps minimal; quality is a later concern):

```rust
use crate::video::{Frame, I420};
use yuv::{bgra_to_yuv420, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix};

/// Nearest-neighbour resize of a BGRA buffer into a tightly-packed BGRA Vec.
fn resize_bgra(frame: &Frame, dst_w: usize, dst_h: usize) -> Vec<u8> {
    let (sw, sh, sstride) = (frame.width as usize, frame.height as usize, frame.stride);
    if sw == dst_w && sh == dst_h && sstride == dst_w * 4 {
        return frame.data.clone();
    }
    let mut out = vec![0u8; dst_w * dst_h * 4];
    for dy in 0..dst_h {
        let sy = dy * sh / dst_h;
        for dx in 0..dst_w {
            let sx = dx * sw / dst_w;
            let s = sy * sstride + sx * 4;
            let d = (dy * dst_w + dx) * 4;
            out[d..d + 4].copy_from_slice(&frame.data[s..s + 4]);
        }
    }
    out
}

/// Convert a BGRA frame to I420, resizing to `dst_w`×`dst_h` first if needed.
pub fn bgra_to_i420(frame: &Frame, dst_w: usize, dst_h: usize) -> I420 {
    let bgra = resize_bgra(frame, dst_w, dst_h);
    let mut planar = YuvPlanarImageMut::<u8>::alloc(dst_w as u32, dst_h as u32, yuv::YuvChromaSubsampling::Yuv420);
    bgra_to_yuv420(
        &mut planar,
        &bgra,
        (dst_w * 4) as u32,
        YuvRange::Limited,
        YuvStandardMatrix::Bt601,
        YuvConversionMode::Balanced,
    )
    .expect("bgra_to_yuv420");
    I420 {
        width: dst_w,
        height: dst_h,
        y: planar.y_plane.borrow().to_vec(),
        u: planar.u_plane.borrow().to_vec(),
        v: planar.v_plane.borrow().to_vec(),
        y_stride: planar.y_stride as usize,
        uv_stride: planar.u_stride as usize,
    }
}
```

Note: `YuvPlanarImageMut::alloc`, the `.y_plane.borrow()` accessors, and `bgra_to_yuv420`'s exact argument order/return are from `yuv` 0.8 — verify against docs.rs and adjust field/method names if the patch version differs; report any change.

- [ ] **Step 5: Run to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib video::convert`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add agent/Cargo.toml agent/Cargo.lock agent/src/video/mod.rs agent/src/video/convert.rs
git commit -m "feat(agent): BGRA->I420 conversion (+ nearest-neighbour resize)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: openh264 encoder

**Files:**
- Modify: `agent/Cargo.toml` (add `openh264`), `agent/src/video/mod.rs` (add `pub mod openh264_encoder;`)
- Create: `agent/src/video/openh264_encoder.rs`
- Test: inline `#[cfg(test)]` in `openh264_encoder.rs`

**Interfaces:**
- Consumes: `I420`, `EncodedSample`, `VideoEncoder` (Task 1).
- Produces: `pub struct Openh264Encoder` with `pub fn new(width: u32, height: u32, bitrate_bps: u32, fps: f32) -> anyhow::Result<Self>`, implementing `VideoEncoder`.

- [ ] **Step 1: Add the `openh264` dependency**

In `agent/Cargo.toml` under `[dependencies]`:

```toml
openh264 = { version = "0.9", features = ["source"] }
```

(The `source` feature builds libopenh264 from vendored source. If the build fails for lack of a C toolchain/nasm, report it — do not switch strategies silently.)

- [ ] **Step 2: Write the failing test**

Create `agent/src/video/openh264_encoder.rs` with the test first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::{VideoEncoder, I420};

    fn gray_i420(w: usize, h: usize) -> I420 {
        I420 {
            width: w, height: h,
            y: vec![128u8; w * h],
            u: vec![128u8; (w / 2) * (h / 2)],
            v: vec![128u8; (w / 2) * (h / 2)],
            y_stride: w, uv_stride: w / 2,
        }
    }

    // The first encoded frame must be a keyframe carrying SPS(7)+PPS(8)+IDR(5)
    // NAL units (Annex-B start codes), so a fresh browser decoder can start.
    #[test]
    fn first_frame_is_keyframe_with_parameter_sets() {
        let mut enc = Openh264Encoder::new(64, 64, 1_000_000, 30.0).unwrap();
        let sample = enc.encode(&gray_i420(64, 64), true).unwrap();
        assert!(!sample.data.is_empty());
        assert!(sample.keyframe);
        let types = nal_types(&sample.data);
        assert!(types.contains(&7), "missing SPS; got {types:?}");
        assert!(types.contains(&8), "missing PPS; got {types:?}");
        assert!(types.contains(&5), "missing IDR; got {types:?}");
    }

    #[test]
    fn subsequent_frame_encodes_without_error() {
        let mut enc = Openh264Encoder::new(64, 64, 1_000_000, 30.0).unwrap();
        let _ = enc.encode(&gray_i420(64, 64), true).unwrap();
        let s = enc.encode(&gray_i420(64, 64), false).unwrap();
        assert!(!s.data.is_empty());
    }

    /// Parse Annex-B NAL unit types (5 low bits of the byte after each start code).
    fn nal_types(annexb: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 4 <= annexb.len() {
            let is_sc4 = annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 0 && annexb[i + 3] == 1;
            let is_sc3 = annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 1;
            if is_sc4 { out.push(annexb[i + 4] & 0x1f); i += 5; }
            else if is_sc3 { out.push(annexb[i + 3] & 0x1f); i += 4; }
            else { i += 1; }
        }
        out
    }
}
```

- [ ] **Step 3: Register the module and run to verify it fails**

Add to `agent/src/video/mod.rs`:

```rust
pub mod openh264_encoder;
```

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib video::openh264_encoder`
Expected: FAIL to compile — `Openh264Encoder` not found (first `openh264` source build — slow).

- [ ] **Step 4: Implement `Openh264Encoder`**

Prepend to `agent/src/video/openh264_encoder.rs`. Signatures for `EncoderConfig::bitrate/max_frame_rate`, `BitRate`/`FrameRate` constructors, `force_intra_frame`, and `EncodedBitStream::to_vec` are from openh264 0.9.3 — verify against docs.rs and report if a name differs:

```rust
use crate::video::{EncodedSample, VideoEncoder, I420};
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate};
use openh264::formats::YUVSlices;
use std::time::Duration;

/// Software H.264 encoder (openh264). Owns the encoder and the per-frame duration.
pub struct Openh264Encoder {
    encoder: Encoder,
    frame_dur: Duration,
}

impl Openh264Encoder {
    pub fn new(width: u32, height: u32, bitrate_bps: u32, fps: f32) -> anyhow::Result<Self> {
        let _ = (width, height); // resolution is taken from the YUVSource at encode time
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bitrate_bps))
            .max_frame_rate(FrameRate::from_hz(fps));
        let encoder = Encoder::with_api_config(openh264::OpenH264API::from_source(), config)?;
        Ok(Self { encoder, frame_dur: Duration::from_secs_f32(1.0 / fps) })
    }
}

impl VideoEncoder for Openh264Encoder {
    fn encode(&mut self, frame: &I420, force_idr: bool) -> anyhow::Result<EncodedSample> {
        if force_idr {
            self.encoder.force_intra_frame();
        }
        let yuv = YUVSlices::new(
            (&frame.y, &frame.u, &frame.v),
            (frame.width, frame.height),
            (frame.y_stride, frame.uv_stride, frame.uv_stride),
        );
        let bitstream = self.encoder.encode(&yuv)?;
        let data = bitstream.to_vec();
        // openh264 emits SPS+PPS with each IDR; treat a forced-IDR frame as keyframe.
        Ok(EncodedSample { data, duration: self.frame_dur, keyframe: force_idr })
    }
}
```

- [ ] **Step 5: Run to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib video::openh264_encoder`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add agent/Cargo.toml agent/Cargo.lock agent/src/video/mod.rs agent/src/video/openh264_encoder.rs
git commit -m "feat(agent): openh264 software H.264 encoder

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: VideoPipeline + wire H264 track into PeerSession (test-pattern end-to-end)

**Files:**
- Create: `agent/src/video/pipeline.rs`, `agent/tests/video_pipeline.rs`, `agent/tests/video_sdp.rs`
- Modify: `agent/src/video/mod.rs` (add `pub mod pipeline;` + `pub fn make_source()`), `agent/src/webrtc_peer.rs`
- Test: `agent/tests/video_pipeline.rs`, `agent/tests/video_sdp.rs`

**Interfaces:**
- Consumes: `ScreenCapturer`, `VideoEncoder`, `SampleSink`, `EncodedSample`, `Frame`, `I420`, `bgra_to_i420`, `Openh264Encoder`, `TestPatternSource` (Tasks 1–3).
- Produces:
  - `pub struct VideoPipeline` with `pub fn start(capturer: Box<dyn ScreenCapturer>, encoder: Box<dyn VideoEncoder>, sink: std::sync::Arc<dyn SampleSink>, dst_w: usize, dst_h: usize, keyframe_interval: u64) -> VideoPipeline`. Dropping it stops the pipeline.
  - `pub fn make_source(w: u32, h: u32, fps: u32) -> Box<dyn ScreenCapturer>` in `mod.rs` — selects by `RD_VIDEO_SOURCE` (until Task 5: always `TestPatternSource`).
  - In `webrtc_peer.rs`: a `TrackSampleSink` wrapping `Arc<TrackLocalStaticSample>` + a `tokio::runtime::Handle`; `PeerSession` gains a `_video: Option<VideoPipeline>` field and adds a sendonly H264 track in `build`.

- [ ] **Step 1: Write the failing pipeline integration test**

Create `agent/tests/video_pipeline.rs`:

```rust
use rd_agent::video::pipeline::VideoPipeline;
use rd_agent::video::testpattern::TestPatternSource;
use rd_agent::video::openh264_encoder::Openh264Encoder;
use rd_agent::video::{EncodedSample, SampleSink};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingSink(Mutex<Vec<EncodedSample>>);
impl SampleSink for RecordingSink {
    fn write(&self, sample: EncodedSample) {
        self.0.lock().unwrap().push(sample);
    }
}

#[test]
fn testpattern_pipeline_produces_encoded_keyframe() {
    let sink = Arc::new(RecordingSink::default());
    let capturer = Box::new(TestPatternSource { width: 128, height: 72, fps: 30 });
    let encoder = Box::new(Openh264Encoder::new(64, 64, 1_000_000, 30.0).unwrap());
    let pipeline = VideoPipeline::start(capturer, encoder, sink.clone(), 64, 64, 60);
    std::thread::sleep(std::time::Duration::from_millis(500));
    drop(pipeline); // stop
    let samples = sink.0.lock().unwrap();
    assert!(!samples.is_empty(), "pipeline produced no samples");
    assert!(samples[0].keyframe, "first sample should be a forced keyframe");
    assert!(!samples[0].data.is_empty());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --test video_pipeline`
Expected: FAIL to compile — `VideoPipeline` not found.

- [ ] **Step 3: Implement `VideoPipeline` + `make_source`**

Create `agent/src/video/pipeline.rs`:

```rust
use crate::video::convert::bgra_to_i420;
use crate::video::{ScreenCapturer, VideoEncoder, SampleSink};
use std::sync::mpsc;
use std::sync::Arc;

/// Runs capture → BGRA→I420 → encode → sink on a dedicated thread. The first
/// frame and every `keyframe_interval`-th frame force an IDR. Dropping the
/// `VideoPipeline` closes the frame channel and the thread exits.
pub struct VideoPipeline {
    _stop: mpsc::Sender<()>,
}

impl VideoPipeline {
    pub fn start(
        mut capturer: Box<dyn ScreenCapturer>,
        mut encoder: Box<dyn VideoEncoder>,
        sink: Arc<dyn SampleSink>,
        dst_w: usize,
        dst_h: usize,
        keyframe_interval: u64,
    ) -> VideoPipeline {
        let (frame_tx, frame_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        // capture delivers frames onto frame_tx; if start() fails, log and bail.
        if let Err(e) = capturer.start(frame_tx) {
            tracing::error!("video capture failed to start: {e}");
            return VideoPipeline { _stop: stop_tx };
        }
        std::thread::spawn(move || {
            let mut n: u64 = 0;
            // hold capturer alive for the life of the thread
            let _capturer = capturer;
            loop {
                // Stop when the VideoPipeline is dropped: its `_stop` Sender
                // drops, so try_recv() returns Disconnected (NOT Empty). Break on
                // anything that isn't Empty. (`.is_ok()` would miss Disconnected.)
                match stop_rx.try_recv() {
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    _ => break,
                }
                let frame = match frame_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(f) => f,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let i420 = bgra_to_i420(&frame, dst_w, dst_h);
                let force_idr = n % keyframe_interval == 0;
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
```

Note: dropping `VideoPipeline` drops `_stop` (the only `Sender`), so `stop_rx` disconnects; the worker also exits when the frame channel disconnects. Add to `agent/src/video/mod.rs`:

```rust
pub mod pipeline;

/// Select the capture source. Until the SckCapturer lands (Task 5) this is
/// always the test pattern; afterwards `RD_VIDEO_SOURCE=screen` (the default)
/// selects real capture and `testpattern` forces the synthetic source.
pub fn make_source(w: u32, h: u32, fps: u32) -> Box<dyn ScreenCapturer> {
    Box::new(testpattern::TestPatternSource { width: w, height: h, fps })
}
```

- [ ] **Step 4: Run the pipeline test to verify it passes**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --test video_pipeline`
Expected: PASS.

- [ ] **Step 5: Write the failing SDP test**

Create `agent/tests/video_sdp.rs` (the agent's answer must contain a video m-line when the offer includes a recvonly video transceiver):

```rust
use rd_agent::webrtc_peer::PeerSession;
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc::api::APIBuilder;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTPCodecType;
use webrtc::rtp_transceiver::rtp_transceiver_direction::RTCRtpTransceiverDirection;
use webrtc::rtp_transceiver::RTCRtpTransceiverInit;

#[tokio::test]
async fn agent_answer_includes_video_when_offer_requests_it() {
    let (agent_ice_tx, _rx) = mpsc::unbounded_channel::<serde_json::Value>();
    let agent = PeerSession::new(vec![], agent_ice_tx).await.unwrap();

    // web side: a media engine with default codecs (H264 included), offering
    // to RECEIVE video.
    let mut m = MediaEngine::default();
    m.register_default_codecs().unwrap();
    let mut reg = Registry::new();
    reg = register_default_interceptors(reg, &mut m).unwrap();
    let api = APIBuilder::new().with_media_engine(m).with_interceptor_registry(reg).build();
    let web = Arc::new(api.new_peer_connection(RTCConfiguration::default()).await.unwrap());
    web.add_transceiver_from_kind(
        RTPCodecType::Video,
        Some(RTCRtpTransceiverInit { direction: RTCRtpTransceiverDirection::Recvonly, send_encodings: vec![] }),
    ).await.unwrap();

    let offer = web.create_offer(None).await.unwrap();
    let mut gather = web.gathering_complete_promise().await;
    web.set_local_description(offer).await.unwrap();
    let _ = gather.recv().await;
    let full_offer = web.local_description().await.unwrap();

    let answer_sdp = agent.accept_offer(&full_offer.sdp).await.unwrap();
    assert!(answer_sdp.contains("m=video"), "answer has no video m-line:\n{answer_sdp}");
    // agent sends video → its m-line is sendonly (or sendrecv)
    assert!(answer_sdp.contains("a=sendonly") || answer_sdp.contains("a=sendrecv"),
        "video not sendable in answer:\n{answer_sdp}");
    // the agent's H264 track must be negotiated: the answer advertises an H264
    // rtpmap (e.g. `a=rtpmap:96 H264/90000`). This is the cheap first-line gate
    // that the video track/codec is actually offered; full RTP-flow delivery is
    // covered by the manual smoke (Task 8).
    assert!(answer_sdp.contains("H264"), "answer video has no H264 rtpmap:\n{answer_sdp}");

    let _ = RTCSessionDescription::answer(answer_sdp).unwrap();
    agent.close().await.unwrap();
    web.close().await.unwrap();
}
```

- [ ] **Step 6: Run to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --test video_sdp`
Expected: FAIL — answer has no `m=video` (agent adds no track yet).

- [ ] **Step 7: Add the H264 track + start the pipeline in `PeerSession::build`**

In `agent/src/webrtc_peer.rs`, add imports:

```rust
use crate::video::pipeline::VideoPipeline;
use crate::video::{make_source, EncodedSample, SampleSink};
use crate::video::openh264_encoder::Openh264Encoder;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::media::Sample;
use bytes::Bytes;
```

Add a sink that writes encoded samples to the track using a captured runtime handle:

```rust
/// SampleSink that forwards encoded H.264 to a WebRTC track. `write_sample` is
/// async, so we block on it via the runtime handle captured at construction.
struct TrackSampleSink {
    track: Arc<TrackLocalStaticSample>,
    handle: tokio::runtime::Handle,
}
impl SampleSink for TrackSampleSink {
    fn write(&self, sample: EncodedSample) {
        let track = self.track.clone();
        let s = Sample { data: Bytes::from(sample.data), duration: sample.duration, ..Default::default() };
        // block_on is safe here: this runs on the pipeline's own OS thread,
        // never on a runtime worker.
        if let Err(e) = self.handle.block_on(track.write_sample(&s)) {
            tracing::warn!("write_sample failed: {e}");
        }
    }
}
```

Add fields to `PeerSession` (keep the Task 1/3/6 fields from Plan 3):

```rust
pub struct PeerSession {
    pc: Arc<RTCPeerConnection>,
    ice_buffer: Mutex<IceBuffer<RTCIceCandidateInit>>,
    _injector: Option<InputInjector>,
    _video: Option<VideoPipeline>,
}
```

In `build`, after the peer connection and data-channel wiring, add the video track and start the pipeline (constants come from Global Constraints):

```rust
        // Video: add a sendonly H264 track and start the capture→encode pipeline.
        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability { mime_type: "video/H264".to_owned(), clock_rate: 90000, ..Default::default() },
            "video".to_owned(),
            "rd-agent".to_owned(),
        ));
        pc.add_track(video_track.clone()).await?;

        let (dst_w, dst_h, fps) = (1280u32, 720u32, 30u32);
        let sink: Arc<dyn SampleSink> = Arc::new(TrackSampleSink {
            track: video_track,
            handle: tokio::runtime::Handle::current(),
        });
        let capturer = make_source(dst_w, dst_h, fps);
        let encoder: Box<dyn crate::video::VideoEncoder> = match Openh264Encoder::new(dst_w, dst_h, 3_000_000, fps as f32) {
            Ok(e) => Box::new(e),
            Err(e) => {
                tracing::error!("H264 encoder init failed, video disabled: {e}");
                // still return a session (input-only) — mirror the injector-fail path
                return Self::finish(pc, input_tx, None);
            }
        };
        let video = VideoPipeline::start(capturer, encoder, sink, dst_w as usize, dst_h as usize, 60);
        Self::finish(pc, input_tx, Some(video))
```

Refactor the tail of `build` so both the encoder-fail path and the normal path construct the session through one helper (avoids duplicating the struct literal). Replace the final `Ok(PeerSession { .. })` with:

```rust
    }

    fn finish(
        pc: Arc<RTCPeerConnection>,
        _input_tx: Sender<InputEvent>,
        video: Option<VideoPipeline>,
    ) -> Result<PeerSession> {
        Ok(PeerSession {
            pc,
            ice_buffer: Mutex::new(IceBuffer::new()),
            _injector: None,
            _video: video,
        })
    }
```

(The `_input_tx` is already consumed by `wire_input` earlier in `build`; `finish` only assembles the struct. `new` still sets `_injector` after building, as in Plan 3.) Keep `new`/`new_with_input_sink` working: `new` calls `build(..)` then sets `session._injector = Some(injector)` as before — unchanged.

- [ ] **Step 8: Run both video tests + the full suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml`
Expected: PASS — `video_sdp` now finds `m=video`; `video_pipeline` passes; Plan 3 tests (input_loopback, ice_trickle, protocol_roundtrip, lib units) still green; the `#[ignore]` injection test still ignored.

- [ ] **Step 9: Commit**

```bash
git add agent/src/video/pipeline.rs agent/src/video/mod.rs agent/src/webrtc_peer.rs agent/tests/video_pipeline.rs agent/tests/video_sdp.rs
git commit -m "feat(agent): video pipeline + sendonly H264 track (test-pattern e2e)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: ScreenCaptureKit capture + screen-recording permission

**Files:**
- Modify: `agent/Cargo.toml` (macOS `screencapturekit`), `agent/src/video/mod.rs` (`make_source` selects by env; add `#[cfg(macos)] pub mod sck_capturer;`), `agent/src/permission.rs`, `agent/src/main.rs`
- Create: `agent/src/video/sck_capturer.rs`
- Test: inline unit test for `make_source` selection; `#[ignore]` real-capture integration in `sck_capturer.rs`

**Interfaces:**
- Consumes: `Frame`, `ScreenCapturer` (Task 1).
- Produces: `pub struct SckCapturer { pub fps: u32 }` implementing `ScreenCapturer` (macOS); `make_source` returns it when `RD_VIDEO_SOURCE != "testpattern"`; `permission::check_screen_recording_permission() -> bool`.

- [ ] **Step 1: Add the macOS dependency**

In `agent/Cargo.toml`, extend the existing macOS target section (from Plan 3's `macos-accessibility-client`):

```toml
[target.'cfg(target_os = "macos")'.dependencies]
macos-accessibility-client = "0.0.2"
screencapturekit = "8"
```

- [ ] **Step 2: Write the failing `make_source` selection test**

Add to `agent/src/video/mod.rs` a test module:

```rust
#[cfg(test)]
mod source_selection_tests {
    #[test]
    fn testpattern_env_forces_synthetic_source() {
        // With RD_VIDEO_SOURCE=testpattern, make_source must not touch the OS.
        std::env::set_var("RD_VIDEO_SOURCE", "testpattern");
        let mut src = super::make_source(64, 48, 30);
        let (tx, rx) = std::sync::mpsc::channel();
        src.start(tx).unwrap();
        let f = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!((f.width, f.height), (64, 48));
        std::env::remove_var("RD_VIDEO_SOURCE");
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo test --manifest-path agent/Cargo.toml --lib video::source_selection`
Expected: FAIL — `make_source` currently ignores the env (returns the test pattern regardless, so the assertion on size passes but the env branch doesn't exist yet). To make this a real RED, first update `make_source` to branch on the env (Step 4) — if the test already passes because both branches yield a test pattern today, that's acceptable; the meaningful behavior change is Step 4 wiring `screen` → `SckCapturer`.

- [ ] **Step 4: Implement `make_source` env branch + `SckCapturer`**

Update `make_source` in `agent/src/video/mod.rs`:

```rust
#[cfg(target_os = "macos")]
pub mod sck_capturer;

/// Select the capture source by `RD_VIDEO_SOURCE`: `testpattern` → synthetic,
/// anything else (default `screen`) → real capture where available.
pub fn make_source(w: u32, h: u32, fps: u32) -> Box<dyn ScreenCapturer> {
    let want = std::env::var("RD_VIDEO_SOURCE").unwrap_or_else(|_| "screen".to_string());
    if want == "testpattern" {
        return Box::new(testpattern::TestPatternSource { width: w, height: h, fps });
    }
    #[cfg(target_os = "macos")]
    {
        return Box::new(sck_capturer::SckCapturer { fps });
    }
    #[cfg(not(target_os = "macos"))]
    {
        tracing::warn!("no screen capture backend on this platform; using test pattern");
        Box::new(testpattern::TestPatternSource { width: w, height: h, fps })
    }
}
```

Create `agent/src/video/sck_capturer.rs`. The `screencapturekit` 8.0 API (SCShareableContent / SCContentFilter / SCStreamConfiguration / SCStream / SCStreamOutputTrait / CMSampleBuffer / CVPixelBuffer) is transcribed from docs.rs — **verify each type/method against docs.rs/screencapturekit/8.0.0 and report any name that does not compile; do not guess**:

```rust
use crate::video::{Frame, ScreenCapturer};
use std::sync::mpsc::Sender;
use screencapturekit::{
    shareable_content::SCShareableContent,
    stream::{
        configuration::{pixel_format::PixelFormat, SCStreamConfiguration},
        content_filter::SCContentFilter,
        output_trait::SCStreamOutputTrait,
        output_type::SCStreamOutputType,
        SCStream,
    },
};
use screencapturekit::output::CMSampleBuffer;

/// Captures the main display via ScreenCaptureKit, delivering BGRA `Frame`s.
pub struct SckCapturer {
    pub fps: u32,
}

struct FrameHandler {
    sink: Sender<Frame>,
    start: std::time::Instant,
}

impl SCStreamOutputTrait for FrameHandler {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, _of_type: SCStreamOutputType) {
        let Some(pixel_buffer) = sample.image_buffer() else { return };
        let Ok(guard) = pixel_buffer.lock() else { return };
        let (w, h) = (guard.width() as u32, guard.height() as u32);
        let bytes = guard.as_slice();
        let stride = bytes.len() / h.max(1) as usize;
        let frame = Frame {
            width: w,
            height: h,
            stride,
            data: bytes.to_vec(),
            ts_micros: self.start.elapsed().as_micros() as u64,
        };
        let _ = self.sink.send(frame);
    }
}

impl ScreenCapturer for SckCapturer {
    fn start(&mut self, sink: Sender<Frame>) -> anyhow::Result<()> {
        let content = SCShareableContent::get().map_err(|e| anyhow::anyhow!("SCShareableContent: {e:?}"))?;
        let display = content
            .displays()
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no display found"))?;
        let filter = SCContentFilter::new().with_display_excluding_windows(&display, &[]);
        let config = SCStreamConfiguration::new()
            .set_pixel_format(PixelFormat::BGRA)?;
        let mut stream = SCStream::new(&filter, &config);
        stream.add_output_handler(
            FrameHandler { sink, start: std::time::Instant::now() },
            SCStreamOutputType::Screen,
        );
        stream.start_capture().map_err(|e| anyhow::anyhow!("start_capture: {e:?}"))?;
        // Keep the stream alive for the process; SCK drives frames on its own queue.
        std::mem::forget(stream);
        Ok(())
    }
}
```

(Method names such as `with_display_excluding_windows`, `set_pixel_format`, `image_buffer`, `lock`/`as_slice`/`width`/`height`, and the module paths are the leaf details to verify against docs.rs 8.0.0. `std::mem::forget(stream)` is a pragmatic MVP choice to keep the stream running; a later task can hold it in the capturer for clean shutdown.)

- [ ] **Step 5: Add the screen-recording permission check**

In `agent/src/permission.rs`, add:

```rust
/// Check whether the process can capture the screen on macOS, logging guidance
/// if not. Elsewhere it's a no-op returning true. MVP heuristic: on macOS,
/// SCShareableContent::get() succeeds only when Screen Recording is authorized.
pub fn check_screen_recording_permission() -> bool {
    #[cfg(target_os = "macos")]
    {
        match screencapturekit::shareable_content::SCShareableContent::get() {
            Ok(_) => true,
            Err(_) => {
                tracing::warn!(
                    "macOS Screen Recording permission not granted — the remote screen \
                     will be blank. Approve this program under System Settings → Privacy & \
                     Security → Screen Recording, then restart it."
                );
                false
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
```

In `agent/src/main.rs`, after the existing input-permission check, add:

```rust
    if !rd_agent::permission::check_screen_recording_permission() {
        tracing::warn!("continuing without screen-recording permission; video will be blank");
    }
```

- [ ] **Step 6: Write the `#[ignore]` real-capture integration test**

Add to `agent/src/video/sck_capturer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Requires a real display + Screen Recording permission. Run explicitly:
    // cargo test --manifest-path agent/Cargo.toml -- --ignored sck
    #[test]
    #[ignore]
    fn captures_a_real_frame() {
        let mut cap = SckCapturer { fps: 30 };
        let (tx, rx) = std::sync::mpsc::channel();
        cap.start(tx).unwrap();
        let f = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(f.width > 0 && f.height > 0);
        assert_eq!(f.data.len(), f.stride * f.height as usize);
    }
}
```

- [ ] **Step 7: Build, run the non-ignored suite, and verify the ignored test is listed**

Run: `export PATH="$HOME/.cargo/bin:$PATH"; cargo build --manifest-path agent/Cargo.toml && cargo test --manifest-path agent/Cargo.toml`
Expected: builds clean; `make_source` selection test passes; `captures_a_real_frame` shows as ignored; all prior tests green. (Optionally run `cargo test -- --ignored sck` locally with permission granted to confirm real capture.)

- [ ] **Step 8: Commit**

```bash
git add agent/Cargo.toml agent/Cargo.lock agent/src/video/mod.rs agent/src/video/sck_capturer.rs agent/src/permission.rs agent/src/main.rs
git commit -m "feat(agent): ScreenCaptureKit capture + screen-recording permission

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: web — recvonly video transceiver + ontrack

**Files:**
- Modify: `packages/web/src/rtc.ts`, `packages/web/src/rtc.test.ts`

**Interfaces:**
- Consumes: existing `connectSession`/`Session` (Plan 3).
- Produces: `SessionCallbacks.onRemoteStream?: (stream: MediaStream) => void`; `connectSession` adds `pc.addTransceiver("video", { direction: "recvonly" })` and wires `pc.ontrack` to call `onRemoteStream`.

- [ ] **Step 1: Write the failing test**

The live WebRTC parts aren't unit-testable in jsdom (as noted in `rtc.ts`), so factor the ontrack→stream extraction into a pure helper and test that. Add to `packages/web/src/rtc.test.ts`:

```ts
import { streamFromTrackEvent } from "./rtc.js";

test("streamFromTrackEvent prefers event.streams[0]", () => {
  const stream = { id: "s1" } as unknown as MediaStream;
  const ev = { streams: [stream], track: { kind: "video" } } as unknown as RTCTrackEvent;
  expect(streamFromTrackEvent(ev)).toBe(stream);
});

test("streamFromTrackEvent falls back to a new stream from the track", () => {
  const track = { kind: "video" } as unknown as MediaStreamTrack;
  const ev = { streams: [], track } as unknown as RTCTrackEvent;
  const s = streamFromTrackEvent(ev, (t) => ({ tracks: [t] }) as unknown as MediaStream);
  expect((s as unknown as { tracks: MediaStreamTrack[] }).tracks[0]).toBe(track);
});
```

- [ ] **Step 2: Run to verify it fails**

Run: `npm test -- rtc.test`
Expected: FAIL — `streamFromTrackEvent` not exported.

- [ ] **Step 3: Implement the helper + wire ontrack/transceiver**

In `packages/web/src/rtc.ts`:

Add the pure helper near the other pure helpers:

```ts
/** Resolve the MediaStream for an incoming track: prefer the negotiated
 *  stream, else build one from the track. `mk` is injectable for testing. */
export function streamFromTrackEvent(
  ev: RTCTrackEvent,
  mk: (t: MediaStreamTrack) => MediaStream = (t) => new MediaStream([t]),
): MediaStream {
  return ev.streams[0] ?? mk(ev.track);
}
```

Add `onRemoteStream?: (stream: MediaStream) => void;` to `SessionCallbacks`, and destructure it in `connectSession` (`const { onState, onError, onRemoteStream } = callbacks;`).

In `startPeer`, after creating the `RTCPeerConnection` and before/after creating the data channel, add the recvonly video transceiver and the ontrack handler:

```ts
    pc.addTransceiver("video", { direction: "recvonly" });
    pc.ontrack = (ev) => {
      onRemoteStream?.(streamFromTrackEvent(ev));
    };
```

- [ ] **Step 4: Run to verify it passes**

Run: `npm test -- rtc.test`
Expected: PASS (new helper tests + existing rtc tests).

- [ ] **Step 5: Full JS regression + commit**

Run: `npm test`
Expected: all pass (57 + 2 new).

```bash
git add packages/web/src/rtc.ts packages/web/src/rtc.test.ts
git commit -m "feat(web): request recvonly video + expose remote stream on ontrack

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: web — `<video>` element in SessionView

**Files:**
- Modify: `packages/web/src/pages/SessionView.tsx`

**Interfaces:**
- Consumes: `connectSession` with `onRemoteStream` (Task 6); `mouseCoords`/`mouseButtonName`/`sendInput` (Plan 3).
- Produces: UI. The capture surface becomes a `<video>`; input handlers move onto it. Verified by typecheck + build.

- [ ] **Step 1: Replace the placeholder surface with a `<video>` bound to the remote stream**

In `packages/web/src/pages/SessionView.tsx`:

Add a video ref and wire the stream:

```tsx
  const videoRef = useRef<HTMLVideoElement | null>(null);
```

In the `connectSession` call, add the `onRemoteStream` callback:

```tsx
    const session = connectSession(API_BASE, token, device.id, {
      onState: setState,
      onError: setError,
      onRemoteStream: (stream) => {
        if (videoRef.current) videoRef.current.srcObject = stream;
      },
    });
```

Replace the capture-surface `<div ref={surfaceRef} ...>` (the dashed placeholder) with a `<video>` that carries the SAME ref and the SAME capture handlers (mouse/key props + the native wheel effect from Plan 3 keep targeting `surfaceRef`). Point `surfaceRef` at the video element by using one ref for both (assign `videoRef` and `surfaceRef` via a callback ref), or reuse `surfaceRef` as the video ref directly:

```tsx
      <video
        ref={(el) => {
          surfaceRef.current = el;
          videoRef.current = el;
        }}
        data-testid="remote-surface"
        tabIndex={0}
        autoPlay
        muted
        playsInline
        onMouseMove={onMouseMove}
        onMouseDown={onMouseDown}
        onMouseUp={onMouseUp}
        onKeyDown={onKeyDown}
        onKeyUp={onKeyUp}
        onContextMenu={(e) => e.preventDefault()}
        style={{
          width: "100%", height: 360, borderRadius: 8, border: "2px solid #cbd5e1",
          background: "#0f172a", outline: "none", objectFit: "contain",
          cursor: connected ? "crosshair" : "default",
        }}
      />
```

Change `surfaceRef`'s type to `HTMLVideoElement` (from `HTMLDivElement`): `const surfaceRef = useRef<HTMLVideoElement | null>(null);`. The native-wheel `useEffect` (Plan 3) already reads `surfaceRef.current` and calls `addEventListener("wheel", …)` — an `HTMLVideoElement` supports that unchanged. Keep the "Sent events" log and the header/state badge. Remove the placeholder text node that used to sit inside the div.

- [ ] **Step 2: Typecheck + build**

Run: `npm run -w @rd/web build && npm run typecheck`
Expected: both clean. (Ref type is now `HTMLVideoElement`; the mouse/key handlers and wheel effect compile against it.)

- [ ] **Step 3: Full JS regression + commit**

Run: `npm test`
Expected: 59 pass (unchanged — no SessionView test).

```bash
git add packages/web/src/pages/SessionView.tsx
git commit -m "feat(web): render remote video track; capture input on the <video>

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: e2e smoke doc

**Files:**
- Create: `docs/superpowers/plan4-video-smoke.md`

- [ ] **Step 1: Write the smoke guide**

Create `docs/superpowers/plan4-video-smoke.md`, reusing the bring-up from `docs/superpowers/plan3-input-smoke.md` (read it first) and adding the two-stage video check:

```markdown
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
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/plan4-video-smoke.md
git commit -m "docs: Plan 4 screen-capture + video e2e smoke guide

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## After all tasks

- Whole-branch review (final code review).
- Update `docs/BACKLOG.md`: mark Plan 4 ✅ (macOS); refresh test counts; move deferred items (Windows/Linux capture, hardware encoders, bitrate/resolution adaptation, multi-monitor, resolution-change renegotiation, the Plan 3 stuck-key auto-release now that the surface is a `<video>`) into the roadmap/carry-over.
- `superpowers:finishing-a-development-branch`.

## Self-Review (spec coverage)

- Spec §2 staged de-risking → Tasks 1–4 (test pattern e2e) then Task 5 (SCK). §3.1 types/traits → Task 1. §3.2 test pattern → Task 1. §3.3 encoder → Task 3. §3.4 convert → Task 2. §3.5 SCK → Task 5. §3.6 pipeline → Task 4. §3.7 PeerSession track + pipeline → Task 4. §3.8 permission → Task 5. §3.9 web → Tasks 6–7. §4 data flow → Tasks 4/6/7. §5 error handling → encoder-fail/input-only (Task 4), capture-fail/permission (Task 5), warn-and-drop (Task 4 pipeline). §6 tests → per-task units + `video_pipeline`/`video_sdp` integration + `#[ignore]` SCK. §7 task split → Parallel Groups. §8 deps → Global Constraints + Tasks 2/3/5.
- Type consistency: `Frame`/`I420`/`EncodedSample`/`ScreenCapturer`/`VideoEncoder`/`SampleSink` defined in Task 1 and used unchanged in Tasks 2–5; `make_source` signature identical in Tasks 4 and 5; `VideoPipeline::start` signature identical in Task 4 def and its test; `TrackSampleSink`/`finish` naming consistent within Task 4; web `onRemoteStream`/`streamFromTrackEvent` consistent across Tasks 6–7.
- Leaf-API risk is called out explicitly (openh264 `BitRate`/`FrameRate`/`to_vec`, `yuv` fn arg order, `screencapturekit` 8.0 method names, `webrtc_media::Sample` fields): the implementer verifies against docs.rs and reports mismatches rather than guessing — the same protocol that caught `Key::Insert` in Plan 3.
```
