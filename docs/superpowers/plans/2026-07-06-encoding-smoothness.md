# Encoding / Transport Smoothness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Kill the periodic remote-screen stutter (real-time RTP timing + on-demand keyframes) and add macOS hardware H.264 encoding via VideoToolbox.

**Architecture:** The pipeline stamps each WebRTC sample with the *measured* wall-clock inter-frame gap (not a fixed 1/fps), forces keyframes on a time interval plus on RTCP PLI/FIR, and the `VideoEncoder` trait consumes BGRA frames so a new VideoToolbox encoder can hardware-encode without the CPU I420 convert. openh264 stays for non-macOS and as fallback.

**Tech Stack:** Rust, webrtc-rs 0.11, openh264, objc2-video-toolbox 0.3 (macOS), screencapturekit.

## Global Constraints

- No protocol/web changes. Bitrate clamp stays `[250_000, 20_000_000]`.
- Browser-decodable H.264: ConstrainedBaseline/Baseline profile, no B-frames, Annex-B with SPS(7)+PPS(8)+IDR(5) on every keyframe.
- openh264 must stay fully functional (non-macOS + fallback). `RD_VIDEO_ENCODER=openh264` forces software on macOS.
- cargo: `export PATH="$HOME/.cargo/bin:$PATH"`; manifest `agent/Cargo.toml`; modules registered in `agent/src/lib.rs` (no new top-level modules except `videotoolbox_encoder`, added under `video/mod.rs`); clippy gate `cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings`.
- Commit messages English, ending with:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`
- VideoToolbox tests are `#[cfg(target_os = "macos")]` (this machine has the hardware).

## Current-state facts (as of plan authoring)

- `agent/src/video/mod.rs`: `pub trait VideoEncoder { fn encode(&mut self, frame: &I420, force_idr: bool) -> anyhow::Result<EncodedSample>; fn set_bitrate(&mut self, _:u32){} fn reset(&mut self){} }`. `EncodedSample { data: Vec<u8>, duration: Duration, keyframe: bool }`. `Frame { width:u32, height:u32, stride:usize, data:Vec<u8> /*BGRA*/, ts_micros:u64 }`.
- `agent/src/video/pipeline.rs`: `pub enum PipelineCmd { Bitrate(u32), Resolution(u32,u32) }`. `VideoPipeline::start(capturer, encoder, sink, dst_w, dst_h, keyframe_interval: u64, cmd_rx, source_factory)`. Loop: drains cmds, drop-to-latest frame, `let i420 = bgra_to_i420(&frame, dst_w, dst_h); let force_idr = n.is_multiple_of(keyframe_interval) || force_next_idr; encoder.encode(&i420, force_idr) -> sink.write`.
- `agent/src/video/convert.rs`: `fn resize_bgra(frame:&Frame,dst_w,dst_h)->Vec<u8>` (private), `pub fn bgra_to_i420(frame:&Frame,dst_w,dst_h)->I420`.
- `agent/src/video/openh264_encoder.rs`: `encode(&mut self, frame:&I420, force_idr)`, with `needs_rebuild`/`force_idr_next` deferred-rebuild, `set_bitrate` (rebuild), `reset` (sets needs_rebuild). Tests build I420 via `gray_i420`/`solid`.
- `agent/src/webrtc_peer.rs`: builds encoder at ~line 437 via `Openh264Encoder::new(dst_w,dst_h,3_000_000,fps as f32)`; `pc.add_track(video_track.clone()).await?;` (discards the sender) at ~451; `VideoPipeline::start(capturer, encoder, sink, dst_w as usize, dst_h as usize, 60, cmd_rx, factory)` at ~459; `let factory = Box::new(move |w,h| make_source(w,h,fps))`. `cmd_tx`/`cmd_rx` created ~line 404; `cmd_tx` cloned as `ctl_cmd_tx`.
- `agent/tests/video_pipeline.rs`: integration test calling `VideoPipeline::start(... keyframe_interval ...)` with a `TestPatternSource` and a `PipelineCmd` channel.
- `TrackSampleSink::write` builds `Sample { data, duration: sample.duration, ..Default::default() }`.

---

### Task 1: Real-time frame duration

**Files:**
- Modify: `agent/src/video/pipeline.rs`

**Interfaces:**
- Produces: `pub fn sample_duration(prev: Option<Instant>, now: Instant, fallback: Duration) -> Duration` in `pipeline.rs`; the pipeline overrides each emitted sample's `duration` with it.

- [ ] **Step 1: Write the failing test** — add to `pipeline.rs`'s `cmd_tests` module (or a new `mod duration_tests`):

```rust
#[cfg(test)]
mod duration_tests {
    use super::sample_duration;
    use std::time::{Duration, Instant};

    #[test]
    fn first_frame_uses_fallback() {
        let t = Instant::now();
        assert_eq!(sample_duration(None, t, Duration::from_millis(33)), Duration::from_millis(33));
    }
    #[test]
    fn normal_gap_passes_through() {
        let t0 = Instant::now();
        let d = sample_duration(Some(t0), t0 + Duration::from_millis(50), Duration::from_millis(33));
        assert_eq!(d, Duration::from_millis(50));
    }
    #[test]
    fn long_idle_gap_is_clamped_high() {
        let t0 = Instant::now();
        let d = sample_duration(Some(t0), t0 + Duration::from_secs(5), Duration::from_millis(33));
        assert_eq!(d, Duration::from_millis(1000));
    }
    #[test]
    fn zero_gap_is_clamped_low() {
        let t0 = Instant::now();
        assert_eq!(sample_duration(Some(t0), t0, Duration::from_millis(33)), Duration::from_millis(1));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml duration_tests`
Expected: compile FAIL — `sample_duration` not found.

- [ ] **Step 3: Implement the helper** — add near the top of `pipeline.rs` (after the `use` lines), and add `use std::time::{Duration, Instant};`:

```rust
/// Wall-clock gap between successive emitted samples, so the RTP media clock
/// tracks real time (webrtc-rs advances the RTP timestamp by sample.duration).
/// `prev` is None for the first sample. Clamped so a long idle gap can't leap
/// the RTP clock and a zero gap can't stall it.
pub fn sample_duration(prev: Option<Instant>, now: Instant, fallback: Duration) -> Duration {
    match prev {
        None => fallback,
        Some(p) => now
            .saturating_duration_since(p)
            .clamp(Duration::from_millis(1), Duration::from_millis(1000)),
    }
}
```

- [ ] **Step 4: Wire it into the loop** — in the pipeline thread, add `let mut last_emit: Option<Instant> = None;` beside the other loop-local state (near `let mut n: u64 = 0;`). Replace the encode+write block:

```rust
                match encoder.encode(&i420, force_idr) {
                    Ok(sample) => sink.write(sample),
                    Err(e) => tracing::warn!("encode failed on frame {n}: {e}"),
                }
```

with:

```rust
                match encoder.encode(&i420, force_idr) {
                    Ok(mut sample) => {
                        let now = Instant::now();
                        sample.duration = sample_duration(last_emit, now, sample.duration);
                        last_emit = Some(now);
                        sink.write(sample);
                    }
                    Err(e) => tracing::warn!("encode failed on frame {n}: {e}"),
                }
```

(`sample.duration`'s fallback is the encoder's own 1/fps value, used only for the first frame — no new parameter needed.)

- [ ] **Step 5: Run tests**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add agent/src/video/pipeline.rs
git commit -m "fix(agent): stamp WebRTC samples with measured inter-frame duration"
```

---

### Task 2: Time-based keyframe interval + ForceKeyframe command

**Files:**
- Modify: `agent/src/video/pipeline.rs`
- Modify: `agent/src/webrtc_peer.rs` (call-site: `60` → `Duration::from_secs(4)`)
- Modify: `agent/tests/video_pipeline.rs` (call-site: keyframe_interval arg → `Duration`)

**Interfaces:**
- Consumes: `sample_duration` (Task 1).
- Produces: `pub fn should_force_keyframe(since_last: Duration, interval: Duration, requested: bool) -> bool`; `PipelineCmd::ForceKeyframe`; `VideoPipeline::start(..., keyframe_interval: Duration, ...)`.

- [ ] **Step 1: Write the failing tests** — add to `pipeline.rs`:

```rust
#[cfg(test)]
mod keyframe_tests {
    use super::should_force_keyframe;
    use std::time::Duration;

    #[test]
    fn forces_when_interval_elapsed() {
        assert!(should_force_keyframe(Duration::from_secs(4), Duration::from_secs(4), false));
        assert!(should_force_keyframe(Duration::from_secs(5), Duration::from_secs(4), false));
    }
    #[test]
    fn no_force_before_interval() {
        assert!(!should_force_keyframe(Duration::from_secs(1), Duration::from_secs(4), false));
    }
    #[test]
    fn request_overrides_interval() {
        assert!(should_force_keyframe(Duration::from_millis(1), Duration::from_secs(4), true));
    }
}
```

And a pipeline behavior test in `cmd_tests` (reuses `LoopingSource`/`RecordingEncoder` from that module — `RecordingEncoder` will gain a `keyframes: Arc<Mutex<Vec<bool>>>` field in Step 4; if the harness there records only bitrates, extend it minimally to also capture the `force_idr` arg):

```rust
    #[test]
    fn force_keyframe_cmd_makes_next_frame_a_keyframe() {
        // Use a long interval so only the explicit request forces a keyframe.
        let started = Arc::new(Mutex::new(Vec::new()));
        let idrs = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel::<PipelineCmd>();
        let st = started.clone();
        let _p = VideoPipeline::start(
            Box::new(LoopingSource { w: 2, h: 2, started: started.clone() }),
            Box::new(IdrRecordingEncoder { idrs: idrs.clone() }),
            Arc::new(NullSink), 2, 2, Duration::from_secs(3600), rx,
            Box::new(move |w, h| Box::new(LoopingSource { w, h, started: st.clone() })),
        );
        assert!(wait_until(2000, || idrs.lock().unwrap().len() > 3));
        let before = idrs.lock().unwrap().iter().filter(|k| **k).count();
        tx.send(PipelineCmd::ForceKeyframe).unwrap();
        assert!(wait_until(2000, || idrs.lock().unwrap().iter().filter(|k| **k).count() > before),
            "no keyframe after ForceKeyframe");
    }
```

(Add a tiny `IdrRecordingEncoder { idrs: Arc<Mutex<Vec<bool>>> }` to the test module whose `encode` pushes `force_idr` into `idrs`; it records whether each frame was a keyframe. Its `encode` signature is `&I420` for now — Task 4 flips the whole trait to `&Frame` and updates this too.)

- [ ] **Step 2: Run to verify failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml keyframe_tests`
Expected: compile FAIL — `should_force_keyframe` / `PipelineCmd::ForceKeyframe` not found.

- [ ] **Step 3: Add the helper + enum variant** — in `pipeline.rs`:

```rust
/// Force a keyframe when the interval has elapsed OR one was requested
/// (resolution swap, or a browser PLI/FIR relayed as PipelineCmd::ForceKeyframe).
pub fn should_force_keyframe(since_last: Duration, interval: Duration, requested: bool) -> bool {
    requested || since_last >= interval
}
```

Extend the enum:

```rust
pub enum PipelineCmd {
    Bitrate(u32),
    Resolution(u32, u32),
    /// Emit a keyframe on the next frame (relayed from an RTCP PLI/FIR).
    ForceKeyframe,
}
```

- [ ] **Step 4: Change the interval to `Duration` and drive keyframes by time** — change `start`'s signature `keyframe_interval: u64` → `keyframe_interval: Duration`. In the loop:
  - init `let mut last_keyframe = Instant::now();` and keep `let mut force_next_idr = true;` (first frame is a keyframe).
  - in the cmd drain, add: `PipelineCmd::ForceKeyframe => force_next_idr = true,`
  - replace `let force_idr = n.is_multiple_of(keyframe_interval) || force_next_idr;` with:

```rust
                let frame_time = Instant::now();
                let force_idr = should_force_keyframe(
                    frame_time.duration_since(last_keyframe),
                    keyframe_interval,
                    force_next_idr,
                );
                force_next_idr = false;
                if force_idr {
                    last_keyframe = frame_time;
                }
```

  - keep the existing `n` counter for the `encode failed on frame {n}` log.
  - the existing `Resolution` arm already sets `force_next_idr = true` after a swap — leave it.

- [ ] **Step 5: Update call sites** — `agent/src/webrtc_peer.rs`: change the `VideoPipeline::start(... , 60, cmd_rx, factory)` argument `60` to `std::time::Duration::from_secs(4)`. `agent/tests/video_pipeline.rs`: change its keyframe_interval argument to a `Duration` (`std::time::Duration::from_secs(4)`, or a small one if the test asserts keyframe cadence — preserve the test's intent). Update the existing `cmd_tests` `VideoPipeline::start` calls that pass `60`/`2` as keyframe_interval to `Duration::from_secs(...)` values that preserve each test's intent (the same-size no-op and resolution-swap tests don't depend on keyframe timing → `Duration::from_secs(3600)` is fine; the drop-to-latest test likewise).

- [ ] **Step 6: Run the full suite**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add agent/src/video/pipeline.rs agent/src/webrtc_peer.rs agent/tests/video_pipeline.rs
git commit -m "feat(agent): time-based keyframe interval (4s) + ForceKeyframe command"
```

---

### Task 3: RTCP PLI/FIR → on-demand keyframe

**Files:**
- Modify: `agent/src/video/mod.rs` (add `rtcp_requests_keyframe` + re-export)
- Modify: `agent/src/webrtc_peer.rs` (capture the RTP sender; spawn a read_rtcp loop)

**Interfaces:**
- Consumes: `PipelineCmd::ForceKeyframe` (Task 2); the `cmd_tx` sender in `webrtc_peer.rs`.
- Produces: `pub fn rtcp_requests_keyframe(pkts: &[Box<dyn rtcp::packet::Packet + Send + Sync>]) -> bool`.

- [ ] **Step 1: Confirm the rtcp types** (research step, no code):

Run: `grep -rn "fn as_any" ~/.cargo/registry/src/index.crates.io-*/rtcp-*/src/packet.rs; ls ~/.cargo/registry/src/index.crates.io-*/rtcp-*/src/payload_feedbacks/`
Expected: `Packet` trait exposes `fn as_any(&self) -> &(dyn Any + Send + Sync)`; directories `picture_loss_indication` and `full_intra_request` exist. Note the exact module paths for the imports below; adjust if they differ.

- [ ] **Step 2: Write the failing test** — add to `agent/src/video/mod.rs`:

```rust
#[cfg(test)]
mod rtcp_tests {
    use super::rtcp_requests_keyframe;
    use rtcp::packet::Packet;
    use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
    use rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
    use rtcp::receiver_report::ReceiverReport;

    fn boxed(p: impl Packet + Send + Sync + 'static) -> Box<dyn Packet + Send + Sync> {
        Box::new(p)
    }

    #[test]
    fn pli_requests_keyframe() {
        let pkts = vec![boxed(PictureLossIndication { sender_ssrc: 1, media_ssrc: 2 })];
        assert!(rtcp_requests_keyframe(&pkts));
    }
    #[test]
    fn fir_requests_keyframe() {
        let pkts = vec![boxed(FullIntraRequest { sender_ssrc: 1, media_ssrc: 2, fir: vec![] })];
        assert!(rtcp_requests_keyframe(&pkts));
    }
    #[test]
    fn receiver_report_does_not() {
        let pkts = vec![boxed(ReceiverReport::default())];
        assert!(!rtcp_requests_keyframe(&pkts));
    }
    #[test]
    fn empty_does_not() {
        assert!(!rtcp_requests_keyframe(&[]));
    }
}
```

(If `FullIntraRequest`/`PictureLossIndication` field names differ from the crate version, fix them per Step 1's findings — keep the assertions.)

- [ ] **Step 3: Run to verify failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml rtcp_tests`
Expected: compile FAIL — `rtcp_requests_keyframe` not found.

- [ ] **Step 4: Implement the classifier** — in `agent/src/video/mod.rs`:

```rust
/// True if a received RTCP compound contains a keyframe request (PLI or FIR).
/// The browser sends these on packet loss; the agent replies with a fresh IDR.
pub fn rtcp_requests_keyframe(pkts: &[Box<dyn rtcp::packet::Packet + Send + Sync>]) -> bool {
    use rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
    use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
    pkts.iter().any(|p| {
        let a = p.as_any();
        a.is::<PictureLossIndication>() || a.is::<FullIntraRequest>()
    })
}
```

(The `rtcp` crate is a transitive dep of `webrtc`; if it is not directly nameable, add `rtcp = "<version matching webrtc 0.11>"` to `agent/Cargo.toml` — check `grep '^name = "rtcp"' -A1 agent/Cargo.lock` for the exact version.)

- [ ] **Step 5: Spawn the read_rtcp loop** — in `agent/src/webrtc_peer.rs`, change `pc.add_track(video_track.clone()).await?;` to capture the sender and spawn the loop (place it right after `add_track`, while `cmd_tx` is in scope):

```rust
        let rtp_sender = pc.add_track(video_track.clone()).await?;
        // Relay browser keyframe requests (PLI/FIR) to the pipeline so a lost
        // frame recovers immediately instead of waiting for the 4s interval.
        let kf_cmd_tx = cmd_tx.clone();
        tokio::spawn(async move {
            while let Ok((pkts, _)) = rtp_sender.read_rtcp().await {
                if crate::video::rtcp_requests_keyframe(&pkts) {
                    let _ = kf_cmd_tx.send(crate::video::pipeline::PipelineCmd::ForceKeyframe);
                }
            }
        });
```

(`read_rtcp` returns `Result<(Vec<Box<dyn rtcp::packet::Packet + Send + Sync>>, Attributes)>`; the loop ends when the sender closes.)

- [ ] **Step 6: Run the suite + build**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings && cargo build --manifest-path agent/Cargo.toml`
Expected: PASS + build clean.

- [ ] **Step 7: Commit**

```bash
git add agent/src/video/mod.rs agent/src/webrtc_peer.rs agent/Cargo.toml agent/Cargo.lock
git commit -m "feat(agent): request keyframe on RTCP PLI/FIR"
```

---

### Task 4: `VideoEncoder::encode` consumes BGRA `Frame`

**Files:**
- Modify: `agent/src/video/mod.rs` (trait signature)
- Modify: `agent/src/video/convert.rs` (expose a `resize_bgra_frame` helper)
- Modify: `agent/src/video/openh264_encoder.rs` (convert BGRA→I420 internally; tests feed BGRA)
- Modify: `agent/src/video/pipeline.rs` (resize BGRA instead of converting to I420; fake encoders take `&Frame`)

**Interfaces:**
- Produces: `VideoEncoder::encode(&mut self, frame: &Frame, force_idr: bool)`; `pub fn resize_bgra_frame(frame: &Frame, dst_w: usize, dst_h: usize) -> Frame` in `convert.rs`.
- Consumes: existing `bgra_to_i420` (now called inside openh264).

- [ ] **Step 1: Add the resize helper + failing test** — in `agent/src/video/convert.rs`, make `resize_bgra` reusable and add:

```rust
/// Resize a BGRA frame to `dst_w`×`dst_h`, returning a tightly-packed BGRA Frame
/// (stride = dst_w*4). A no-op copy when the source already matches.
pub fn resize_bgra_frame(frame: &Frame, dst_w: usize, dst_h: usize) -> Frame {
    let data = resize_bgra(frame, dst_w, dst_h);
    Frame { width: dst_w as u32, height: dst_h as u32, stride: dst_w * 4, data, ts_micros: frame.ts_micros }
}
```

Add a test to `convert.rs`'s `tests`:

```rust
    #[test]
    fn resize_bgra_frame_targets_and_packs() {
        let f = resize_bgra_frame(&solid_bgra(32, 32, 10, 20, 30), 16, 16);
        assert_eq!((f.width, f.height, f.stride), (16, 16, 16 * 4));
        assert_eq!(f.data.len(), 16 * 16 * 4);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml resize_bgra_frame`
Expected: compile FAIL — not found.

- [ ] **Step 3: Change the trait** — in `agent/src/video/mod.rs`:

```rust
pub trait VideoEncoder: Send {
    /// Encode a BGRA frame (already at the target size) to H.264 Annex-B.
    fn encode(&mut self, frame: &Frame, force_idr: bool) -> anyhow::Result<EncodedSample>;
    fn set_bitrate(&mut self, _bitrate_bps: u32) {}
    fn reset(&mut self) {}
}
```

- [ ] **Step 4: openh264 converts internally** — in `agent/src/video/openh264_encoder.rs`, change `encode`'s signature and body to convert BGRA→I420 first:

```rust
    fn encode(&mut self, frame: &crate::video::Frame, force_idr: bool) -> anyhow::Result<EncodedSample> {
        if self.needs_rebuild {
            self.encoder = Self::build_encoder(self.bitrate_bps, self.fps)?;
            self.needs_rebuild = false;
            self.force_idr_next = true;
        }
        let idr = force_idr || self.force_idr_next;
        if idr {
            self.encoder.force_intra_frame();
        }
        self.force_idr_next = false;
        let i420 = crate::video::convert::bgra_to_i420(frame, frame.width as usize, frame.height as usize);
        let yuv = YUVSlices::new(
            (&i420.y, &i420.u, &i420.v),
            (i420.width, i420.height),
            (i420.y_stride, i420.uv_stride, i420.uv_stride),
        );
        let bitstream = self.encoder.encode(&yuv)?;
        let data = bitstream.to_vec();
        Ok(EncodedSample { data, duration: self.frame_dur, keyframe: idr })
    }
```

Fix imports: this file no longer needs `I420` directly (it produces one via `bgra_to_i420`); keep `use crate::video::{EncodedSample, VideoEncoder};` and drop the now-unused `I420` import if clippy flags it. Update the encoder's own tests: replace the `gray_i420(w,h)` helper with a BGRA `Frame` builder and call `encode(&frame, ...)`:

```rust
    fn gray_bgra(w: usize, h: usize) -> crate::video::Frame {
        crate::video::Frame { width: w as u32, height: h as u32, stride: w * 4, data: vec![128u8; w * h * 4], ts_micros: 0 }
    }
```

and change every `enc.encode(&gray_i420(64, 64), ...)` to `enc.encode(&gray_bgra(64, 64), ...)`. The NAL-type assertions (7/8/5 on keyframe) stay unchanged.

- [ ] **Step 5: Pipeline resizes instead of converting** — in `agent/src/video/pipeline.rs`:
  - change the import `use crate::video::convert::bgra_to_i420;` → `use crate::video::convert::resize_bgra_frame;`
  - replace `let i420 = bgra_to_i420(&frame, dst_w, dst_h);` and the `encoder.encode(&i420, force_idr)` call with:

```rust
                let sized = resize_bgra_frame(&frame, dst_w, dst_h);
                match encoder.encode(&sized, force_idr) {
```

  - update the fake encoders in the test modules (`RecordingEncoder`, `IdrRecordingEncoder`, `SlowEncoder`, and any other `impl VideoEncoder` in tests) so `encode` takes `&crate::video::Frame`. Where a test read `f.y[0]` (I420 luma), read `f.data[0]` (BGRA byte) instead — in the drop-to-latest `SlowEncoder`/`CountingSource` test, `CountingSource` already emits gray `Frame`s whose `data` is filled with the frame-index gray value, so assert on `f.data[0]` (the luma proxy) and keep the ">120 means recent frame" logic.

- [ ] **Step 6: Run the full suite + build**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings`
Expected: PASS (openh264 NAL tests, pipeline tests, convert tests all green).

- [ ] **Step 7: Commit**

```bash
git add agent/src/video/mod.rs agent/src/video/convert.rs agent/src/video/openh264_encoder.rs agent/src/video/pipeline.rs
git commit -m "refactor(agent): VideoEncoder::encode consumes BGRA frames"
```

---

### Task 5: VideoToolbox hardware encoder (macOS)

> **Model note:** dispatch on the most capable model — this is FFI against `objc2-video-toolbox` 0.3 and may need iteration. The exact objc2 binding calls must be written against the crate docs (docs.rs/objc2-video-toolbox/0.3.2 and docs.rs/objc2-core-media, objc2-core-video). The property-key names, algorithm, trait contract, and tests below are exact; the binding *syntax* is the implementer's to fill from the crate.

**Files:**
- Modify: `agent/Cargo.toml` (macOS target deps)
- Create: `agent/src/video/videotoolbox_encoder.rs`
- Modify: `agent/src/video/mod.rs` (register `#[cfg(target_os="macos")] pub mod videotoolbox_encoder;`)

**Interfaces:**
- Consumes: `VideoEncoder` trait (Task 4, `encode(&Frame, force_idr)`), `Frame`, `EncodedSample`.
- Produces: `pub struct VideoToolboxEncoder` with `pub fn new(width: u32, height: u32, bitrate_bps: u32, fps: f32) -> anyhow::Result<Self>` implementing `VideoEncoder`. Task 6 selects it.

- [ ] **Step 1: Add dependencies** — in `agent/Cargo.toml`, under a macOS target table (create if absent):

```toml
[target.'cfg(target_os = "macos")'.dependencies]
objc2 = "0.6"
objc2-video-toolbox = "0.3"
objc2-core-media = "0.3"
objc2-core-video = "0.3"
objc2-core-foundation = "0.3"
```

(Match the `objc2-core-media`/`objc2-core-video` versions already resolved in `Cargo.lock` — 0.3.2 — so the objc2 major aligns. Move any of these already declared elsewhere for `screencapturekit` as needed; do not duplicate.)

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo build --manifest-path agent/Cargo.toml`
Expected: builds (deps resolve).

- [ ] **Step 2: Write the failing macOS test** — create `agent/src/video/videotoolbox_encoder.rs` with only the test module first:

```rust
#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::VideoToolboxEncoder;
    use crate::video::{Frame, VideoEncoder};

    fn bgra(w: usize, h: usize) -> Frame {
        Frame { width: w as u32, height: h as u32, stride: w * 4, data: vec![128u8; w * h * 4], ts_micros: 0 }
    }
    fn nal_types(annexb: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 4 <= annexb.len() {
            if annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 0 && annexb[i + 3] == 1 {
                if i + 4 < annexb.len() { out.push(annexb[i + 4] & 0x1f); }
                i += 5;
            } else { i += 1; }
        }
        out
    }

    #[test]
    fn first_frame_is_keyframe_with_parameter_sets() {
        let mut enc = VideoToolboxEncoder::new(320, 240, 1_000_000, 30.0).unwrap();
        let s = enc.encode(&bgra(320, 240), true).unwrap();
        assert!(s.keyframe && !s.data.is_empty());
        let t = nal_types(&s.data);
        assert!(t.contains(&7) && t.contains(&8) && t.contains(&5), "want SPS+PPS+IDR, got {t:?}");
    }
    #[test]
    fn second_frame_encodes_and_bitrate_reset_ok() {
        let mut enc = VideoToolboxEncoder::new(320, 240, 1_000_000, 30.0).unwrap();
        let _ = enc.encode(&bgra(320, 240), true).unwrap();
        enc.set_bitrate(2_000_000);
        let s = enc.encode(&bgra(320, 240), false).unwrap();
        assert!(!s.data.is_empty());
        enc.reset();
        let s2 = enc.encode(&bgra(320, 240), false).unwrap();
        assert!(s2.keyframe, "reset must re-open with a keyframe");
    }
}
```

Register the module in `agent/src/video/mod.rs`: `#[cfg(target_os = "macos")] pub mod videotoolbox_encoder;`.

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml videotoolbox`
Expected: compile FAIL — `VideoToolboxEncoder` not defined.

- [ ] **Step 3: Implement the encoder.** Build `VideoToolboxEncoder` implementing `VideoEncoder`. Exact requirements (fill objc2 syntax from the crate docs):

  - **Struct/state:** owns the `VTCompressionSession`, `width`, `height`, `bitrate_bps`, `fps`, and a `needs_reopen: bool` (mirrors openh264's deferred rebuild). Mark `unsafe impl Send` only after confirming the session is used solely from the pipeline thread (it is).
  - **`new`:** `VTCompressionSessionCreate` for `kCMVideoCodecType_H264` at `width`×`height` (no pixel-buffer attributes needed for `CVPixelBufferCreateWithBytes` input). Then `VTSessionSetProperty` for:
    - `kVTCompressionPropertyKey_RealTime` = `kCFBooleanTrue`
    - `kVTCompressionPropertyKey_AllowFrameReordering` = `kCFBooleanFalse`
    - `kVTCompressionPropertyKey_ProfileLevel` = `kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel` (fall back to `..._Baseline_AutoLevel` if the constrained constant is absent in the binding)
    - `kVTCompressionPropertyKey_AverageBitRate` = `bitrate_bps` (CFNumber)
    - `kVTCompressionPropertyKey_MaxKeyFrameInterval` = `(fps * 8) as i32` (safety ceiling; we drive keyframes ourselves)
    - `VTCompressionSessionPrepareToEncodeFrames`.
    - Log `kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder` (read via `VTSessionCopyProperty`) at info level.
  - **Output delivery:** VT is async. Use the output callback given to `VTCompressionSessionCreate` (an `extern "C"` fn + refcon), or the block variant if the binding exposes it. In the callback, on success, convert the `CMSampleBuffer` to an Annex-B `Vec<u8>` (below) and push `(data, is_keyframe)` onto a `std::sync::mpsc::Sender<...>` carried via the refcon. `encode` calls `VTCompressionSessionEncodeFrame` then blocks on the paired `Receiver::recv()` for that frame (one frame in flight at a time — the pipeline is single-threaded, so this is safe and low-latency). Optionally call `VTCompressionSessionCompleteFrames(session, presentationTimeStamp)` to flush.
  - **CVPixelBuffer input:** wrap the BGRA `frame.data` with `CVPixelBufferCreateWithBytes`, `kCVPixelFormatType_32BGRA`, `width`, `height`, `bytesPerRow = frame.stride`. (A copy is acceptable for v1; an IOSurface-backed pool is a later optimization — note it in a comment, do not build it now.)
  - **Force keyframe:** pass a per-frame options `CFDictionary` with `kVTEncodeFrameOptionKey_ForceKeyFrame` = `kCFBooleanTrue` when `force_idr` (or when `needs_reopen` was just handled).
  - **Presentation timestamps:** pass a monotonically increasing `CMTime` per frame (e.g. a frame counter over `fps` timescale). VT requires strictly increasing PTS.
  - **AVCC → Annex-B conversion:**
    - Frame NALs: `CMSampleBufferGetDataBuffer` → `CMBlockBufferGetDataPointer` gives an AVCC buffer of `[4-byte big-endian length][NAL]...`. Walk it: for each NAL, replace the 4-byte length with the start code `00 00 00 01`, append the NAL bytes.
    - Parameter sets (only needed on keyframes): `CMSampleBufferGetFormatDescription` → `CMVideoFormatDescriptionGetH264ParameterSetAtIndex` for index 0 (SPS) and 1 (PPS) (use the count out-param); prepend each as `00 00 00 01` + param-set bytes, BEFORE the frame NALs.
    - Keyframe detection: read the sample attachments array (`CMSampleBufferGetSampleAttachmentsArray`); a frame is a keyframe unless `kCMSampleAttachmentKey_NotSync` is present and true. (You may also treat the frame as a keyframe when you requested `ForceKeyFrame` — but prefer the attachment for correctness.)
  - **`set_bitrate(bps)`:** if `bps != self.bitrate_bps`, `VTSessionSetProperty(kVTCompressionPropertyKey_AverageBitRate, bps)` on the live session (no reopen); update `self.bitrate_bps`. Log + keep old on error.
  - **`reset()`:** set `needs_reopen = true`. On the next `encode`, if `needs_reopen`: `VTCompressionSessionInvalidate` the old session, create a fresh one (same `new` property setup), clear the flag, and force a keyframe on that frame. A reopen failure propagates as an `Err` from `encode` (the pipeline catches + logs, per the openh264 contract).
  - **`Drop`:** `VTCompressionSessionInvalidate` the session.

- [ ] **Step 4: Run the macOS tests + clippy**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml videotoolbox && cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings`
Expected: both tests PASS (this machine has hardware VideoToolbox); clippy clean.

- [ ] **Step 5: Commit**

```bash
git add agent/Cargo.toml agent/Cargo.lock agent/src/video/videotoolbox_encoder.rs agent/src/video/mod.rs
git commit -m "feat(agent): VideoToolbox hardware H.264 encoder (macOS)"
```

---

### Task 6: Encoder selection + fallback

**Files:**
- Modify: `agent/src/video/mod.rs` (a `build_encoder` factory)
- Modify: `agent/src/webrtc_peer.rs` (use `build_encoder`; resolution factory too)

**Interfaces:**
- Consumes: `Openh264Encoder`, `VideoToolboxEncoder` (macOS), the `VideoEncoder` trait.
- Produces: `pub fn build_encoder(width: u32, height: u32, bitrate_bps: u32, fps: f32) -> anyhow::Result<Box<dyn VideoEncoder>>`.

- [ ] **Step 1: Write the failing test** — in `agent/src/video/mod.rs`:

```rust
#[cfg(test)]
mod build_encoder_tests {
    use super::{build_encoder, Frame};

    #[test]
    fn builds_a_working_encoder_and_encodes_a_frame() {
        // On macOS this is VideoToolbox (or openh264 if VT init fails); elsewhere
        // openh264. Either way we must get an encoder that produces a keyframe.
        let mut enc = build_encoder(320, 240, 1_000_000, 30.0).expect("encoder");
        let f = Frame { width: 320, height: 240, stride: 320 * 4, data: vec![128u8; 320 * 240 * 4], ts_micros: 0 };
        let s = enc.encode(&f, true).unwrap();
        assert!(s.keyframe && !s.data.is_empty());
    }

    #[test]
    fn env_forces_software_encoder() {
        // RD_VIDEO_ENCODER=openh264 must yield a functioning software encoder.
        std::env::set_var("RD_VIDEO_ENCODER", "openh264");
        let r = build_encoder(320, 240, 1_000_000, 30.0);
        std::env::remove_var("RD_VIDEO_ENCODER");
        assert!(r.is_ok());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml build_encoder`
Expected: compile FAIL — `build_encoder` not found.

- [ ] **Step 3: Implement the factory** — in `agent/src/video/mod.rs`:

```rust
/// Build the H.264 encoder for the platform: VideoToolbox on macOS (hardware),
/// falling back to openh264 if the session can't be created; openh264 elsewhere.
/// `RD_VIDEO_ENCODER=openh264` forces software (debugging).
pub fn build_encoder(
    width: u32,
    height: u32,
    bitrate_bps: u32,
    fps: f32,
) -> anyhow::Result<Box<dyn VideoEncoder>> {
    let force_sw = std::env::var("RD_VIDEO_ENCODER").as_deref() == Ok("openh264");
    #[cfg(target_os = "macos")]
    {
        if !force_sw {
            match videotoolbox_encoder::VideoToolboxEncoder::new(width, height, bitrate_bps, fps) {
                Ok(enc) => return Ok(Box::new(enc)),
                Err(e) => tracing::warn!("VideoToolbox init failed, falling back to openh264: {e}"),
            }
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = force_sw;
    Ok(Box::new(openh264_encoder::Openh264Encoder::new(width, height, bitrate_bps, fps)?))
}
```

(Confirm `openh264_encoder` and `videotoolbox_encoder` are `pub mod` in `mod.rs` so the factory can name them.)

- [ ] **Step 4: Use it in webrtc_peer** — in `agent/src/webrtc_peer.rs`, replace the encoder-build block (currently `match Openh264Encoder::new(dst_w, dst_h, 3_000_000, fps as f32) { ... }`) with:

```rust
        let encoder: Box<dyn crate::video::VideoEncoder> =
            match crate::video::build_encoder(dst_w, dst_h, 3_000_000, fps as f32) {
                Ok(e) => e,
                Err(e) => {
                    tracing::error!("H264 encoder init failed, video disabled: {e}");
                    return Self::finish(pc, input_tx, None, keep_awake);
                }
            };
```

Drop the now-unused `use ...Openh264Encoder` import if clippy flags it. The resolution-swap path rebuilds the encoder inside the pipeline via `encoder.reset()` (not a new `build_encoder` call), so it already stays on whatever encoder was selected — no change needed there.

- [ ] **Step 5: Run the full suite + clippy + release build**

Run: `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings && cargo build --release --manifest-path agent/Cargo.toml`
Expected: PASS + release build clean.

- [ ] **Step 6: Commit**

```bash
git add agent/src/video/mod.rs agent/src/webrtc_peer.rs
git commit -m "feat(agent): select VideoToolbox with openh264 fallback"
```

---

## Final verification (after all tasks)

1. `export PATH="$HOME/.cargo/bin:$PATH" && cargo test --manifest-path agent/Cargo.toml && cargo clippy --manifest-path agent/Cargo.toml --all-targets -- -D warnings && cargo build --release --manifest-path agent/Cargo.toml`.
2. Rebuild+restart the release agent (`RUST_LOG=info`), reconnect the browser session, and confirm on this Mac:
   - agent log shows the VideoToolbox hardware encoder in use (`UsingHardwareAcceleratedVideoEncoder`) and keyframes only ~every 4s / on PLI (not every 2s);
   - playback is smooth (no periodic stutter) at 720p AND native Retina; CPU is lower than the openh264 baseline;
   - resolution hot-switch and the bitrate slider still work (encoder reset re-opens VT with a keyframe; live `AverageBitRate` change);
   - `RD_VIDEO_ENCODER=openh264` still runs (software fallback path) and is smooth at 720p.
```
