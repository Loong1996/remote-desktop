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
