use crate::video::{Frame, ScreenCapturer};
use std::sync::mpsc::Sender;
use screencapturekit::{
    cm::{CMSampleBuffer, CMSampleBufferExt},
    shareable_content::SCShareableContent,
    stream::{
        configuration::{pixel_format::PixelFormat, SCStreamConfiguration},
        content_filter::SCContentFilter,
        output_trait::SCStreamOutputTrait,
        output_type::SCStreamOutputType,
        SCStream,
    },
};

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
        let Some(pixel_buffer) = sample.image_buffer() else {
            return;
        };
        let Ok(guard) = pixel_buffer.lock_read_only() else {
            return;
        };
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
        let content =
            SCShareableContent::get().map_err(|e| anyhow::anyhow!("SCShareableContent: {e:?}"))?;
        let display = content
            .displays()
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no display found"))?;
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build();
        let config = SCStreamConfiguration::new().with_pixel_format(PixelFormat::BGRA);
        let mut stream = SCStream::new(&filter, &config);
        stream.add_output_handler(
            FrameHandler { sink, start: std::time::Instant::now() },
            SCStreamOutputType::Screen,
        );
        stream
            .start_capture()
            .map_err(|e| anyhow::anyhow!("start_capture: {e:?}"))?;
        // Keep the stream alive for the process; SCK drives frames on its own queue.
        std::mem::forget(stream);
        Ok(())
    }
}

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
