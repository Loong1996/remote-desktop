use crate::video::convert::resize_bgra_frame;
use crate::video::{ScreenCapturer, SampleSink, VideoEncoder};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

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

/// Force a keyframe when the interval has elapsed OR one was requested
/// (resolution swap, or a browser PLI/FIR relayed as PipelineCmd::ForceKeyframe).
pub fn should_force_keyframe(since_last: Duration, interval: Duration, requested: bool) -> bool {
    requested || since_last >= interval
}

/// Commands applied by the pipeline thread between frames.
pub enum PipelineCmd {
    Bitrate(u32),
    /// Switch capture to a new size: swap the capturer (via the source factory)
    /// and reset the encoder so the stream re-opens with SPS/PPS+IDR.
    Resolution(u32, u32),
    /// Emit a keyframe on the next frame (relayed from an RTCP PLI/FIR).
    ForceKeyframe,
}

/// Builds a capturer at the requested size (used for hot resolution switches).
pub type SourceFactory = Box<dyn Fn(u32, u32) -> Box<dyn ScreenCapturer> + Send>;

/// Runs capture → BGRA→I420 → encode → sink on a dedicated thread. The first
/// frame, every frame after `keyframe_interval` has elapsed since the last
/// one, and any frame following a `PipelineCmd::ForceKeyframe` force an IDR.
/// Dropping the `VideoPipeline` closes the frame channel and the thread exits.
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
        keyframe_interval: Duration,
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
            let mut force_next_idr = true; // first frame is a keyframe
            let mut last_keyframe = Instant::now();
            let mut last_emit: Option<Instant> = None;
            // Stop when the VideoPipeline is dropped: its `_stop` Sender drops,
            // so try_recv() returns Disconnected (NOT Empty). Loop only while
            // Empty; anything else (including Disconnected) exits.
            while let Err(mpsc::TryRecvError::Empty) = stop_rx.try_recv() {
                // Apply queued control commands between frames.
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        PipelineCmd::Bitrate(bps) => encoder.set_bitrate(bps),
                        PipelineCmd::ForceKeyframe => force_next_idr = true,
                        PipelineCmd::Resolution(w, h) => {
                            // Already capturing at this size (e.g. a state re-sync
                            // on control-channel open): skip the pointless swap.
                            if w as usize == dst_w && h as usize == dst_h {
                                continue;
                            }
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
                let mut frame = match frame_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(f) => f,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                // Real-time stream: when capture outpaces convert+encode, skip to
                // the newest queued frame. Encoding the backlog would only show
                // the past — latency and memory would grow without bound (at
                // native Retina a single BGRA frame is ~22 MB).
                while let Ok(newer) = frame_rx.try_recv() {
                    frame = newer;
                }
                let sized = resize_bgra_frame(&frame, dst_w, dst_h);
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
                match encoder.encode(&sized, force_idr) {
                    Ok(mut sample) => {
                        let now = Instant::now();
                        sample.duration = sample_duration(last_emit, now, sample.duration);
                        last_emit = Some(now);
                        sink.write(sample);
                    }
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

#[cfg(test)]
mod cmd_tests {
    use super::*;
    use crate::video::{EncodedSample, Frame, SampleSink, ScreenCapturer, VideoEncoder};
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
        fn encode(&mut self, _f: &Frame, _idr: bool) -> anyhow::Result<EncodedSample> {
            Ok(EncodedSample { data: vec![0], duration: Duration::from_millis(33), keyframe: true })
        }
        fn set_bitrate(&mut self, bps: u32) { self.events.lock().unwrap().push(Ev::Bitrate(bps)); }
        fn reset(&mut self) { self.events.lock().unwrap().push(Ev::Reset); }
    }

    struct NullSink;
    impl SampleSink for NullSink {
        fn write(&self, _s: EncodedSample) {}
    }

    /// Records whether each encoded frame was a forced keyframe.
    struct IdrRecordingEncoder { idrs: Arc<Mutex<Vec<bool>>> }
    impl VideoEncoder for IdrRecordingEncoder {
        fn encode(&mut self, _f: &Frame, idr: bool) -> anyhow::Result<EncodedSample> {
            self.idrs.lock().unwrap().push(idr);
            Ok(EncodedSample { data: vec![0], duration: Duration::from_millis(33), keyframe: idr })
        }
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
            Arc::new(NullSink), 2, 2, Duration::from_secs(3600), rx,
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
            Arc::new(NullSink), 2, 2, Duration::from_secs(3600), rx,
            Box::new(move |w, h| Box::new(LoopingSource { w, h, started: st.clone() })),
        );
        // let it run, then switch to 4x4
        assert!(wait_until(2000, || !started.lock().unwrap().is_empty()));
        tx.send(PipelineCmd::Resolution(4, 4)).unwrap();
        assert!(wait_until(2000, || started.lock().unwrap().len() >= 2), "factory not invoked");
        assert_eq!(started.lock().unwrap()[1], (4, 4));
        assert!(wait_until(2000, || events.lock().unwrap().contains(&Ev::Reset)), "encoder not reset");
    }

    /// Emits gray frames whose luma encodes the frame index (brighter = newer)
    /// every 5ms, so a test can tell WHICH frame the encoder actually saw.
    struct CountingSource;
    impl ScreenCapturer for CountingSource {
        fn start(&mut self, sink: std::sync::mpsc::Sender<Frame>) -> anyhow::Result<()> {
            std::thread::spawn(move || {
                let mut i: u32 = 0;
                loop {
                    let gray = (i * 3).min(255) as u8;
                    let f = Frame { width: 2, height: 2, stride: 8, data: vec![gray; 16], ts_micros: 0 };
                    if sink.send(f).is_err() {
                        break;
                    }
                    i += 1;
                    std::thread::sleep(Duration::from_millis(5));
                }
            });
            Ok(())
        }
    }

    /// Takes ~40ms per frame and records the luma of each frame it encodes.
    struct SlowEncoder {
        lumas: Arc<Mutex<Vec<u8>>>,
    }
    impl VideoEncoder for SlowEncoder {
        fn encode(&mut self, f: &Frame, _idr: bool) -> anyhow::Result<EncodedSample> {
            self.lumas.lock().unwrap().push(f.data[0]);
            std::thread::sleep(Duration::from_millis(40));
            Ok(EncodedSample { data: vec![0], duration: Duration::from_millis(33), keyframe: true })
        }
    }

    #[test]
    fn encoder_slower_than_capture_skips_to_the_newest_frame() {
        let lumas = Arc::new(Mutex::new(Vec::new()));
        let (_tx, rx) = std::sync::mpsc::channel::<PipelineCmd>();
        let started = Arc::new(Mutex::new(Vec::new()));
        let st = started.clone();
        let p = VideoPipeline::start(
            Box::new(CountingSource),
            Box::new(SlowEncoder { lumas: lumas.clone() }),
            Arc::new(NullSink), 2, 2, Duration::from_secs(3600), rx,
            Box::new(move |w, h| Box::new(LoopingSource { w, h, started: st.clone() })),
        );
        // Capture emits every 5ms, encode takes 40ms: without drop-to-latest the
        // backlog (and end-to-end latency) grows without bound. After ~500ms the
        // encoder must be seeing RECENT frames, not the 10th-oldest.
        std::thread::sleep(Duration::from_millis(500));
        drop(p);
        let seen = lumas.lock().unwrap().clone();
        assert!(!seen.is_empty());
        // ~500ms / 5ms = ~100 frames emitted (luma ≈ min(3i, 255) through the
        // BGRA→I420 matrix, monotonic in i). Backlog behavior would leave the
        // last encoded frame at i≈12 (luma ≈ 45); drop-to-latest lands near the
        // freshest (i ≥ 60 → luma well above 120).
        let last = *seen.last().unwrap();
        assert!(last > 120, "encoder stuck on stale frames: last luma {last}, seen {seen:?}");
    }

    #[test]
    fn same_size_resolution_cmd_is_a_noop() {
        let started = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel::<PipelineCmd>();
        let st = started.clone();
        let _p = VideoPipeline::start(
            Box::new(LoopingSource { w: 2, h: 2, started: started.clone() }),
            Box::new(RecordingEncoder { events: events.clone() }),
            Arc::new(NullSink), 2, 2, Duration::from_secs(3600), rx,
            Box::new(move |w, h| Box::new(LoopingSource { w, h, started: st.clone() })),
        );
        assert!(wait_until(2000, || !started.lock().unwrap().is_empty()));
        // Requesting the size we are already capturing at must NOT restart the
        // capturer (state re-syncs on control-channel open would otherwise stall
        // the video for no reason) nor reset the encoder.
        tx.send(PipelineCmd::Resolution(2, 2)).unwrap();
        std::thread::sleep(Duration::from_millis(300)); // several frame cycles
        assert_eq!(started.lock().unwrap().len(), 1, "capturer was restarted");
        assert!(!events.lock().unwrap().contains(&Ev::Reset), "encoder was reset");
    }

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
}

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
