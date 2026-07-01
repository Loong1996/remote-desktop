use crate::video::{Frame, ScreenCapturer};
use std::sync::mpsc::Sender;
use xcap::Monitor;

/// Cross-platform screen capturer (Windows + Linux) via the `xcap` crate.
/// macOS uses `SckCapturer` (ScreenCaptureKit) instead. Like the SCK capturer,
/// the capture loop runs on a dedicated owner thread and stops on `Drop`.
pub struct XcapCapturer {
    pub fps: u32,
    stop: Option<Sender<()>>,
}

impl XcapCapturer {
    pub fn new(fps: u32) -> Self {
        Self { fps, stop: None }
    }
}

/// Pick the primary monitor (or the first one if none reports primary).
fn pick_monitor() -> anyhow::Result<Monitor> {
    let monitors = Monitor::all().map_err(|e| anyhow::anyhow!("xcap Monitor::all: {e}"))?;
    let idx = monitors
        .iter()
        .position(|m| m.is_primary().unwrap_or(false))
        .unwrap_or(0);
    monitors
        .into_iter()
        .nth(idx)
        .ok_or_else(|| anyhow::anyhow!("no monitor found"))
}

impl ScreenCapturer for XcapCapturer {
    fn start(&mut self, sink: Sender<Frame>) -> anyhow::Result<()> {
        let fps = self.fps.max(1);
        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<anyhow::Result<()>>();

        std::thread::spawn(move || {
            let monitor = match pick_monitor() {
                Ok(m) => m,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));

            let frame_gap = std::time::Duration::from_micros(1_000_000 / fps as u64);
            let start = std::time::Instant::now();
            // Loop until the pipeline drops us: the `_stop` Sender drops →
            // try_recv() returns Disconnected (not Empty) → the while-let ends.
            while let Err(std::sync::mpsc::TryRecvError::Empty) = stop_rx.try_recv() {
                match monitor.capture_image() {
                    Ok(rgba) => {
                        // capture_image() returns an RgbaImage (ImageBuffer<Rgba<u8>>).
                        let (w, h) = (rgba.width(), rgba.height());
                        let mut data = rgba.into_raw(); // RGBA8888
                        // xcap gives RGBA; the pipeline wants BGRA → swap R and B.
                        for px in data.chunks_exact_mut(4) {
                            px.swap(0, 2);
                        }
                        let frame = Frame {
                            width: w,
                            height: h,
                            stride: (w as usize) * 4,
                            data,
                            ts_micros: start.elapsed().as_micros() as u64,
                        };
                        if sink.send(frame).is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(e) => tracing::warn!("xcap capture failed: {e}"),
                }
                std::thread::sleep(frame_gap);
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

impl Drop for XcapCapturer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Requires a real display. Run explicitly:
    // cargo test --manifest-path agent/Cargo.toml -- --ignored xcap
    #[test]
    #[ignore]
    fn captures_a_real_frame() {
        let mut cap = XcapCapturer::new(30);
        let (tx, rx) = std::sync::mpsc::channel();
        cap.start(tx).unwrap();
        let f = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(f.width > 0 && f.height > 0);
        assert_eq!(f.data.len(), f.stride * f.height as usize);
    }
}
