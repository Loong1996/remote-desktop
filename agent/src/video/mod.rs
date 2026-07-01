use std::time::Duration;

pub mod convert;
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
