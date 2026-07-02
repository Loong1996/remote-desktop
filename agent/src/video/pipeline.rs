use crate::video::convert::bgra_to_i420;
use crate::video::{ScreenCapturer, SampleSink, VideoEncoder};
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
        bitrate_rx: mpsc::Receiver<u32>,
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
            // Stop when the VideoPipeline is dropped: its `_stop` Sender drops,
            // so try_recv() returns Disconnected (NOT Empty). Loop only while
            // Empty; anything else (including Disconnected) exits.
            // (`.is_ok()` would miss Disconnected.)
            while let Err(std::sync::mpsc::TryRecvError::Empty) = stop_rx.try_recv() {
                let frame = match frame_rx.recv_timeout(std::time::Duration::from_millis(200)) {
                    Ok(f) => f,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                // Apply any live bitrate changes requested via the control channel.
                while let Ok(bps) = bitrate_rx.try_recv() {
                    encoder.set_bitrate(bps);
                }
                let i420 = bgra_to_i420(&frame, dst_w, dst_h);
                let force_idr = n.is_multiple_of(keyframe_interval);
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
