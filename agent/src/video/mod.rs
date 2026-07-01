use std::time::Duration;

pub mod convert;
pub mod openh264_encoder;
pub mod pipeline;
pub mod testpattern;

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
        Box::new(sck_capturer::SckCapturer { fps })
    }
    #[cfg(not(target_os = "macos"))]
    {
        tracing::warn!("no screen capture backend on this platform; using test pattern");
        Box::new(testpattern::TestPatternSource { width: w, height: h, fps })
    }
}

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
