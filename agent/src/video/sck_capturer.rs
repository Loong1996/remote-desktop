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

/// Captures the main display via ScreenCaptureKit at 1280x720 / 30fps,
/// delivering BGRA `Frame`s. The `SCStream` is created and owned on a dedicated
/// thread (SCStream is not `Send`); dropping the capturer signals that thread to
/// `stop_capture()` and release the stream — no per-session leak.
pub struct SckCapturer {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    stop: Option<std::sync::mpsc::Sender<()>>,
}

impl SckCapturer {
    /// Capture the main display scaled to `width`×`height`. Callers should pass
    /// a size matching the display's aspect ratio (see
    /// [`crate::video::preset_capture_size`]) so ScreenCaptureKit does not
    /// letterbox the content inside the frame.
    pub fn new(width: u32, height: u32, fps: u32) -> Self {
        Self { width, height, fps, stop: None }
    }
}

/// The main display's size (points), for choosing an aspect-matched capture size.
pub fn main_display_size() -> anyhow::Result<(u32, u32)> {
    let content =
        SCShareableContent::get().map_err(|e| anyhow::anyhow!("SCShareableContent: {e:?}"))?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no display found"))?;
    Ok((display.width(), display.height()))
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
        let fps = self.fps;
        let (cap_w, cap_h) = (self.width.max(2), self.height.max(2));
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        // Report setup success/failure back to start() so permission/stream
        // errors surface synchronously.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();

        std::thread::spawn(move || {
            // Build + start the stream on THIS thread; it never moves.
            let built = (|| -> anyhow::Result<SCStream> {
                let content = SCShareableContent::get()
                    .map_err(|e| anyhow::anyhow!("SCShareableContent: {e:?}"))?;
                let display = content
                    .displays()
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("no display found"))?;
                let filter = SCContentFilter::create()
                    .with_display(&display)
                    .with_excluding_windows(&[])
                    .build();
                let config = SCStreamConfiguration::new()
                    .with_width(cap_w)
                    .with_height(cap_h)
                    .with_fps(fps)
                    .with_pixel_format(PixelFormat::BGRA);
                let mut stream = SCStream::new(&filter, &config);
                stream.add_output_handler(
                    FrameHandler { sink, start: std::time::Instant::now() },
                    SCStreamOutputType::Screen,
                );
                stream
                    .start_capture()
                    .map_err(|e| anyhow::anyhow!("start_capture: {e:?}"))?;
                Ok(stream)
            })();

            match built {
                Ok(stream) => {
                    let _ = ready_tx.send(Ok(()));
                    // Keep the stream alive on this thread until stop is signaled
                    // (or the capturer is dropped, which drops stop_tx).
                    let _ = stop_rx.recv();
                    if let Err(e) = stream.stop_capture() {
                        tracing::warn!("stop_capture failed: {e:?}");
                    }
                    // stream drops here, on its owner thread.
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.stop = Some(stop_tx);
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(anyhow::anyhow!("capture owner thread exited during setup")),
        }
    }
}

impl Drop for SckCapturer {
    fn drop(&mut self) {
        // Signal the owner thread to stop_capture + release the stream.
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
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
        let mut cap = SckCapturer::new(1280, 720, 30);
        let (tx, rx) = std::sync::mpsc::channel();
        cap.start(tx).unwrap();
        let f = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(f.width > 0 && f.height > 0);
        assert_eq!(f.data.len(), f.stride * f.height as usize);
    }
}
