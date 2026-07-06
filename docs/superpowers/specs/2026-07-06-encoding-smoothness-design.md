# Encoding / Transport Smoothness — Design

Date: 2026-07-06
Status: approved (user: VT medium depth = encoder eats BGRA; keyframe 4s + PLI; parallelize where safe)

## Problem

Remote screen stutters periodically ("一卡一卡") even at 720p — i.e. independent
of resolution. Root-cause investigation (traced frame → WebRTC send path) found:

1. **RTP media clock is decoupled from wall-clock.** `TrackSampleSink`
   (`webrtc_peer.rs`) writes every sample with a **fixed** `duration =
   1/fps = 33ms` (from `Openh264Encoder`'s `frame_dur`). webrtc-rs
   (`track_local_static_sample.rs:117`) advances the RTP timestamp by
   `duration * clock_rate` **per sample**, so the media timeline is the sum of
   frame durations, not real time. Real emitted frame rate is variable
   (ScreenCaptureKit skips static frames; encode jitter; the drop-to-latest
   backlog fix), so the media clock drifts from wall-clock → the browser's
   jitter buffer under-/over-runs → periodic stutter. Resolution-independent.
2. **Keyframe bursts every 2s.** `keyframe_interval = 60` forces an IDR every
   60 emitted frames (~2s at 30fps). Each IDR is ~100× an inter frame; over a
   constrained/WAN path the burst causes periodic congestion stutter.
3. **Software encoding.** openh264 is a CPU encoder; no hardware acceleration.
   Not the direct cause of the stutter, but a ceiling on CPU/quality headroom,
   especially at native Retina.

## Scope

Three changes, sequenced by dependency and risk (①② ship smoothness first; ③
is the large hardware-encode change, isolated behind the encoder trait):

1. Real-time frame duration (fix #1).
2. Time-based keyframe interval (4s) + on-demand keyframes from RTCP PLI/FIR
   (fix #2).
3. VideoToolbox hardware H.264 encoder on macOS, openh264 retained for
   non-macOS and as fallback (fix #3), with the `VideoEncoder` trait changed to
   consume BGRA frames (skips the CPU I420 convert on the VT path).

Out of scope: changing the bitrate clamp, resolution presets, or the web UI;
Windows/Linux hardware encoding; audio.

## Global constraints

- Wire/behavior compatibility: no protocol changes. Web is untouched.
- Bitrate clamp stays `[250_000, 20_000_000]`.
- H.264 for browser decode: ConstrainedBaseline/Baseline profile, no B-frames
  (`AllowFrameReordering=false`), Annex-B bitstream with SPS(7)+PPS(8)+IDR(5)
  on every keyframe.
- openh264 must remain fully functional (non-macOS + fallback). `RD_VIDEO_ENCODER=openh264`
  forces software even on macOS.
- cargo: `export PATH="$HOME/.cargo/bin:$PATH"`; agent manifest `agent/Cargo.toml`;
  modules registered in `agent/src/lib.rs`; clippy gate `cargo clippy
  --manifest-path agent/Cargo.toml --all-targets -- -D warnings`.
- Commit messages English, ending with the project trailer.

## Component 1 — Real-time frame duration

**Files:** `agent/src/video/pipeline.rs` (+ a pure helper), `agent/src/video/mod.rs`
(EncodedSample already carries `duration`).

- Add a pure helper (unit-testable without timing):

  ```rust
  /// Wall-clock gap between successive emitted samples, clamped so a long idle
  /// gap can't leap the RTP clock. `prev` is None for the first sample.
  pub fn sample_duration(prev: Option<Instant>, now: Instant, fallback: Duration) -> Duration
  ```

  Returns `fallback` (= 1/fps) when `prev` is None; otherwise `(now -
  prev).clamp(1ms, 1000ms)`.
- The pipeline tracks `last_emit: Option<Instant>` and, before `sink.write`,
  overrides the sample's `duration` with `sample_duration(last_emit, Instant::now(), frame_dur)`,
  then sets `last_emit = Some(now)`. `frame_dur` comes from the pipeline's fps
  (no longer relies on the encoder's fixed value; the encoder's `duration`
  field becomes advisory/ignored by the sink override).

Testing: deterministic unit tests of `sample_duration` (None→fallback;
small gap passthrough; >1s clamp; <1ms clamp).

## Component 2 — Keyframe strategy

**Files:** `agent/src/video/pipeline.rs`, `agent/src/webrtc_peer.rs`.

- **Time-based interval.** Replace the frame-count `keyframe_interval: u64`
  parameter with `keyframe_interval: Duration` (default 4s). The pipeline forces
  an IDR when `now - last_keyframe >= keyframe_interval`, tracking `last_keyframe:
  Instant`. Pure helper for the decision:

  ```rust
  pub fn should_force_keyframe(since_last: Duration, interval: Duration, requested: bool) -> bool
  ```

- **On-demand via RTCP.** `pc.add_track` returns `Arc<RTCRtpSender>`. Spawn a
  task looping `sender.read_rtcp().await`; classify each packet with a pure
  helper:

  ```rust
  /// True if any packet in the compound is a keyframe request (PLI or FIR).
  pub fn rtcp_requests_keyframe(pkts: &[Box<dyn rtcp::packet::Packet + ...>]) -> bool
  ```

  On true, send `PipelineCmd::ForceKeyframe`. Add that variant to `PipelineCmd`;
  the pipeline sets a `keyframe_requested` flag consumed on the next frame
  (folded into `should_force_keyframe`).

Testing: `should_force_keyframe` (interval elapsed / not / requested overrides);
`rtcp_requests_keyframe` (PLI→true, FIR→true, RR/SR→false); a pipeline test that
a `ForceKeyframe` cmd makes the next encoded frame a keyframe. The `read_rtcp`
loop itself is covered by e2e (browser sends PLI on packet loss).

## Component 3 — VideoToolbox encoder + BGRA trait

**Files:** `agent/src/video/mod.rs` (trait change + resize move), `agent/src/video/openh264_encoder.rs`,
`agent/src/video/pipeline.rs` (stop converting; resize BGRA), new `agent/src/video/videotoolbox_encoder.rs`,
`agent/src/webrtc_peer.rs` (encoder selection), `agent/Cargo.toml` (`objc2-video-toolbox = "0.3"`, macOS target dep).

- **Trait change (medium depth).** `VideoEncoder::encode(&mut self, frame:
  &Frame, force_idr: bool)` now takes the BGRA `Frame` (not `&I420`). The
  pipeline hands the encoder a BGRA frame **already at the target size**:

  - The pipeline resizes BGRA to `dst_w×dst_h` before `encode` (no-op copy-skip
    when the source already matches — macOS SCK captures at target size, so the
    resize only does work for sources that capture native, e.g. Windows xcap).
    Extract `resize_bgra` (currently inside `bgra_to_i420`) so the pipeline can
    call it directly and pass an owned target-sized BGRA `Frame`.
  - `Openh264Encoder::encode` calls `bgra_to_i420` on the incoming (already
    target-sized) frame internally, then encodes — same output as today.
  - `bgra_to_i420` moves out of the pipeline; its resize responsibility splits
    into the pipeline's `resize_bgra` step + openh264's internal color convert.

- **VideoToolboxEncoder** (`#[cfg(target_os = "macos")]`), implements
  `VideoEncoder`:
  - `new(width, height, bitrate_bps, fps)`: `VTCompressionSessionCreate` for
    H.264; properties `RealTime=true`, `AllowFrameReordering=false`,
    `ProfileLevel = ConstrainedBaseline_AutoLevel`, `AverageBitRate = bitrate`,
    `MaxKeyFrameInterval = fps*8` (we drive keyframes ourselves; this is a safety
    ceiling). Log `UsingHardwareAcceleratedVideoEncoder`.
  - `encode(&Frame, force_idr)`: wrap the BGRA bytes in a `CVPixelBuffer`
    (`kCVPixelFormatType_32BGRA`, width×height, stride from the frame);
    `VTCompressionSessionEncodeFrame` with a per-frame options dict carrying
    `ForceKeyFrame = force_idr`; VT is async (callback) — the encode blocks on a
    channel until this frame's `CMSampleBuffer` returns (acceptable: the pipeline
    owns a dedicated OS thread). Convert the AVCC (length-prefixed) output to
    Annex-B and prepend SPS/PPS (from the `CMFormatDescription`) on keyframes.
    Return `EncodedSample { data, duration (advisory), keyframe }`.
  - `set_bitrate(bps)`: set `AverageBitRate` on the live session (no rebuild).
  - `reset()`: mark for session recreation on the next `encode` (VT sessions are
    bound to a fixed width×height; a resolution swap already rebuilds the encoder
    via the pipeline, and `reset` must re-open with a keyframe). Recreation
    failure is logged and propagates as a per-frame error (pipeline catches it),
    matching the openh264 deferred-rebuild contract.

- **Selection / fallback** (`webrtc_peer.rs`, where the encoder is built):

  ```
  if macOS and RD_VIDEO_ENCODER != "openh264":
      try VideoToolboxEncoder::new(...) -> use it
      on Err: warn + fall back to Openh264Encoder
  else: Openh264Encoder
  ```

  A small `fn build_encoder(w,h,bitrate,fps) -> Box<dyn VideoEncoder>` centralizes
  this; the resolution hot-swap path (which rebuilds the encoder) uses the same
  factory so switches stay on the hardware encoder.

Testing:
- openh264 tests updated to feed BGRA `Frame`s (helper builds a solid-color
  BGRA frame); existing NAL-type assertions (7/8/5 on keyframe) unchanged.
- VideoToolbox tests `#[cfg(target_os = "macos")]` (this machine has the
  hardware): create session; encode a synthetic BGRA frame → first frame is a
  keyframe carrying SPS+PPS+IDR (Annex-B); a second frame encodes; `set_bitrate`
  and `reset` don't error and `reset` yields a keyframe. Assert the session
  reports `UsingHardwareAcceleratedVideoEncoder`.
- Pipeline tests updated for the `Frame`-based encoder signature and the
  `Duration` keyframe interval; the fake encoder records BGRA input.

## Data flow (macOS / VT, after changes)

```
SCStream (BGRA @ target size)
  → Frame → pipeline: resize_bgra (no-op when already target) ; real-duration stamp
  → VideoToolboxEncoder.encode(&Frame, force_idr)
       wrap CVPixelBuffer(BGRA) → VTCompressionSessionEncodeFrame
       → CMSampleBuffer → AVCC→AnnexB + SPS/PPS on keyframe
  → EncodedSample (duration = measured) → TrackSampleSink.write_sample
RTCRtpSender.read_rtcp() loop → PLI/FIR → PipelineCmd::ForceKeyframe → next frame IDR
```

## Risk & rollout

- ③ (VideoToolbox) is the main risk: FFI, async encode, AVCC→Annex-B glue,
  objc2 version alignment. Mitigations: the encoder trait isolates it; openh264
  stays a full-featured fallback; `RD_VIDEO_ENCODER=openh264` forces software;
  VT session-create failure auto-falls-back. Ship ①② first so smoothness lands
  even if ③ needs iteration.
- The `encode(&Frame)` trait change is contained to the two encoder impls, the
  pipeline call site, and their tests — no capturer or web change.
- Verification: agent unit tests + clippy + release build; live e2e on this Mac
  — reconnect, confirm smooth playback (no periodic stutter) at 720p and native,
  CPU drop vs openh264, HUD sane, agent log shows the hardware encoder in use and
  keyframes only every ~4s / on PLI.

## Implementation order

1. Component 1 (real-time duration) — small, independent, ships first.
2. Component 2 (time-based keyframe + PLI) — pipeline + RTCP loop.
3. Trait change to BGRA `encode` (openh264 + pipeline resize) — prep for ③.
4. Component 3 (VideoToolbox encoder + selection/fallback).

All four touch `pipeline.rs` and/or `webrtc_peer.rs`, so they run sequentially
(no parallel implementers); reviews and the objc2/VT feasibility checks run
alongside.
