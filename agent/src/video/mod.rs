use std::time::Duration;

use crate::control::ResolutionPreset;

pub mod convert;
pub mod openh264_encoder;
pub mod pipeline;
#[cfg(target_os = "windows")]
pub mod xcap_capturer;
pub mod testpattern;

#[cfg(target_os = "macos")]
pub mod sck_capturer;

/// Round down to an even number (H.264 needs even width/height).
fn even(n: u32) -> u32 {
    n & !1
}

/// Pick a capture size that preserves the display's aspect ratio, scaled to fit
/// within `max_w`×`max_h`, with even dimensions.
///
/// Capturing at the display's own aspect ratio is what keeps the remote cursor
/// aligned: if we captured at a fixed 16:9 while the display is 16:10,
/// ScreenCaptureKit letterboxes the content inside the frame, but the browser
/// maps the pointer against the *whole* frame — so the injected position drifts
/// horizontally (correct at center, worse toward the edges). Matching the aspect
/// removes the in-frame letterbox entirely.
pub fn fit_aspect(disp_w: u32, disp_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if disp_w == 0 || disp_h == 0 {
        return (even(max_w).max(2), even(max_h).max(2));
    }
    let scale = f64::min(max_w as f64 / disp_w as f64, max_h as f64 / disp_h as f64);
    let w = (disp_w as f64 * scale).round() as u32;
    let h = (disp_h as f64 * scale).round() as u32;
    (even(w).max(2), even(h).max(2))
}

/// Map a resolution preset to a capture size for a display of logical size
/// `dw`×`dh`. Pure so the mapping is unit-testable without a display.
pub fn preset_size(preset: ResolutionPreset, dw: u32, dh: u32) -> (u32, u32) {
    match preset {
        ResolutionPreset::Sd => fit_aspect(dw, dh, 1280, 720),
        ResolutionPreset::Hd => (even(dw).max(2), even(dh).max(2)),
        // Retina physical pixels (2× logical). On a non-Retina display this is
        // an SCK upscale — accepted per the design spec.
        ResolutionPreset::Native => (even(dw * 2).max(2), even(dh * 2).max(2)),
    }
}

/// Capture size for `preset` on the main display. Falls back to 1280×720 when
/// the display query fails (same fallback as target_capture_size).
pub fn preset_capture_size(preset: ResolutionPreset) -> (u32, u32) {
    #[cfg(target_os = "macos")]
    {
        match sck_capturer::main_display_size() {
            Ok((dw, dh)) => return preset_size(preset, dw, dh),
            Err(e) => {
                tracing::warn!("main display size query failed ({e}); using 1280x720");
            }
        }
    }
    // `preset` is only consulted when the display query succeeds (macOS);
    // the fallback is a fixed 1280x720 regardless of preset.
    let _ = preset;
    (1280, 720)
}

/// The capture size to use for the main display: its aspect ratio fit within
/// `max_w`×`max_h`. On macOS this queries the real display; elsewhere (no query
/// wired up) it falls back to the bounding box.
pub fn target_capture_size(max_w: u32, max_h: u32) -> (u32, u32) {
    #[cfg(target_os = "macos")]
    {
        match sck_capturer::main_display_size() {
            Ok((dw, dh)) => return fit_aspect(dw, dh, max_w, max_h),
            Err(e) => {
                tracing::warn!("main display size query failed ({e}); using {max_w}x{max_h}");
            }
        }
    }
    (even(max_w).max(2), even(max_h).max(2))
}

/// Select the capture source by `RD_VIDEO_SOURCE`: `testpattern` → synthetic,
/// anything else (default `screen`) → real capture where available.
pub fn make_source(w: u32, h: u32, fps: u32) -> Box<dyn ScreenCapturer> {
    let want = std::env::var("RD_VIDEO_SOURCE").unwrap_or_else(|_| "screen".to_string());
    if want == "testpattern" {
        return Box::new(testpattern::TestPatternSource { width: w, height: h, fps });
    }
    #[cfg(target_os = "macos")]
    {
        Box::new(sck_capturer::SckCapturer::new(w, h, fps))
    }
    #[cfg(target_os = "windows")]
    {
        let _ = (w, h); // xcap captures at the monitor's native size
        Box::new(xcap_capturer::XcapCapturer::new(fps))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        tracing::warn!("no screen capture backend on this platform yet; using test pattern");
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
    /// Change the target bitrate for subsequent frames. Default: no-op.
    fn set_bitrate(&mut self, _bitrate_bps: u32) {}
    /// Drop internal codec state so the next frame re-initializes at the
    /// incoming frame's dimensions and emits fresh SPS/PPS + IDR (used when the
    /// capture resolution changes). Default: no-op.
    fn reset(&mut self) {}
}

/// Where the pipeline delivers encoded samples (a WebRTC track in production,
/// a recorder in tests).
pub trait SampleSink: Send + Sync {
    fn write(&self, sample: EncodedSample);
}

#[cfg(test)]
mod fit_aspect_tests {
    use super::fit_aspect;

    #[test]
    fn preserves_16_10_display_within_720p_box() {
        // 16:10 display (e.g. 2560x1664) → no 16:9 letterbox; height-limited.
        let (w, h) = fit_aspect(2560, 1664, 1280, 720);
        assert_eq!(h, 720);
        assert_eq!(w, 1108); // 720 * 2560/1664 = 1107.7 → round 1108, even
        assert!(w % 2 == 0 && h % 2 == 0);
    }

    #[test]
    fn exact_16_9_display_fills_the_box() {
        assert_eq!(fit_aspect(1920, 1080, 1280, 720), (1280, 720));
    }

    #[test]
    fn taller_display_is_width_limited() {
        // A 16:10 portrait-ish case fits by width.
        let (w, h) = fit_aspect(1000, 2000, 1280, 720);
        assert_eq!(w, 360); // 720/2 aspect: 720*1000/2000=360
        assert_eq!(h, 720);
    }

    #[test]
    fn zero_display_falls_back_to_box() {
        assert_eq!(fit_aspect(0, 0, 1280, 720), (1280, 720));
    }

    #[test]
    fn dimensions_are_always_even() {
        let (w, h) = fit_aspect(1471, 957, 1280, 720); // odd inputs
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
    }
}

#[cfg(test)]
mod preset_size_tests {
    use super::preset_size;
    use crate::control::ResolutionPreset;

    #[test]
    fn sd_fits_720p_box() {
        // 1512x982 logical display → aspect-fit inside 1280x720, even dims.
        assert_eq!(preset_size(ResolutionPreset::Sd, 1512, 982), (1108, 720));
    }

    #[test]
    fn hd_is_display_logical_size_evened() {
        assert_eq!(preset_size(ResolutionPreset::Hd, 1512, 982), (1512, 982));
        assert_eq!(preset_size(ResolutionPreset::Hd, 1513, 983), (1512, 982));
    }

    #[test]
    fn native_is_double_logical_evened() {
        assert_eq!(preset_size(ResolutionPreset::Native, 1512, 982), (3024, 1964));
    }
}

#[cfg(test)]
mod source_selection_tests {
    use serial_test::serial;

    #[test]
    #[serial]
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
