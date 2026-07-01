use crate::video::{Frame, ScreenCapturer};
use std::sync::mpsc::Sender;

/// A synthetic animated source (moving gradient + a marching white square) used
/// to prove the encode→transport→browser pipeline without any capture backend.
/// Runs a dedicated thread that emits frames at ~`fps` until `sink` is dropped.
pub struct TestPatternSource {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl ScreenCapturer for TestPatternSource {
    fn start(&mut self, sink: Sender<Frame>) -> anyhow::Result<()> {
        let (w, h, fps) = (self.width, self.height, self.fps.max(1));
        std::thread::spawn(move || {
            let stride = (w as usize) * 4;
            let frame_gap = std::time::Duration::from_micros(1_000_000 / fps as u64);
            let mut n: u64 = 0;
            loop {
                let mut data = vec![0u8; stride * h as usize];
                let phase = (n * 4) as u32;
                for y in 0..h {
                    for x in 0..w {
                        let i = (y as usize) * stride + (x as usize) * 4;
                        // BGRA: animated gradient
                        data[i] = ((x + phase) % 256) as u8; // B
                        data[i + 1] = ((y + phase) % 256) as u8; // G
                        data[i + 2] = ((x + y + phase) % 256) as u8; // R
                        data[i + 3] = 255; // A
                    }
                }
                // marching white square as a motion/time reference
                let sq = 8u32;
                let sx = (n as u32 * 2) % w.saturating_sub(sq).max(1);
                let sy = (n as u32) % h.saturating_sub(sq).max(1);
                for y in sy..(sy + sq).min(h) {
                    for x in sx..(sx + sq).min(w) {
                        let i = (y as usize) * stride + (x as usize) * 4;
                        data[i] = 255;
                        data[i + 1] = 255;
                        data[i + 2] = 255;
                        data[i + 3] = 255;
                    }
                }
                let frame = Frame { width: w, height: h, stride, data, ts_micros: n * frame_gap.as_micros() as u64 };
                if sink.send(frame).is_err() {
                    break; // receiver dropped → stop
                }
                n += 1;
                std::thread::sleep(frame_gap);
            }
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn testpattern_emits_frames_of_expected_size_that_change() {
        let mut src = TestPatternSource { width: 64, height: 48, fps: 30 };
        let (tx, rx) = mpsc::channel();
        src.start(tx).unwrap();
        let f0 = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        let f1 = rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(f0.width, 64);
        assert_eq!(f0.height, 48);
        assert_eq!(f0.stride, 64 * 4);
        assert_eq!(f0.data.len(), 64 * 48 * 4);
        // successive frames differ (animation)
        assert_ne!(f0.data, f1.data);
        // timestamps are monotonic
        assert!(f1.ts_micros > f0.ts_micros);
    }
}
