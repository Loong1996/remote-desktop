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

    #[test]
    fn same_size_resolution_cmd_is_a_noop() {
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
        assert!(wait_until(2000, || !started.lock().unwrap().is_empty()));
        // Requesting the size we are already capturing at must NOT restart the
        // capturer (state re-syncs on control-channel open would otherwise stall
        // the video for no reason) nor reset the encoder.
        tx.send(PipelineCmd::Resolution(2, 2)).unwrap();
        std::thread::sleep(Duration::from_millis(300)); // several frame cycles
        assert_eq!(started.lock().unwrap().len(), 1, "capturer was restarted");
        assert!(!events.lock().unwrap().contains(&Ev::Reset), "encoder was reset");
    }
}
