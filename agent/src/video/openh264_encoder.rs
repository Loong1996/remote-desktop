use crate::video::{EncodedSample, VideoEncoder};
use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate};
use openh264::formats::YUVSlices;
use std::time::Duration;

/// Software H.264 encoder (openh264). Owns the encoder, per-frame duration, and
/// the parameters needed to rebuild the encoder when the bitrate changes.
pub struct Openh264Encoder {
    encoder: Encoder,
    frame_dur: Duration,
    fps: f32,
    bitrate_bps: u32,
    force_idr_next: bool,
    needs_rebuild: bool,
}

impl Openh264Encoder {
    pub fn new(width: u32, height: u32, bitrate_bps: u32, fps: f32) -> anyhow::Result<Self> {
        let _ = (width, height); // resolution is taken from the YUVSource at encode time
        let encoder = Self::build_encoder(bitrate_bps, fps)?;
        Ok(Self {
            encoder,
            frame_dur: Duration::from_secs_f32(1.0 / fps),
            fps,
            bitrate_bps,
            force_idr_next: false,
            needs_rebuild: false,
        })
    }

    fn build_encoder(bitrate_bps: u32, fps: f32) -> anyhow::Result<Encoder> {
        let config = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bitrate_bps))
            .max_frame_rate(FrameRate::from_hz(fps));
        Ok(Encoder::with_api_config(openh264::OpenH264API::from_source(), config)?)
    }
}

impl VideoEncoder for Openh264Encoder {
    fn encode(&mut self, frame: &crate::video::Frame, force_idr: bool) -> anyhow::Result<EncodedSample> {
        // A pending reset() rebuild happens here, so a stale codec is never fed
        // frames at new dimensions: if the rebuild fails the frame errors out
        // (caught + logged by the pipeline) instead of reaching the old encoder.
        if self.needs_rebuild {
            self.encoder = Self::build_encoder(self.bitrate_bps, self.fps)?;
            self.needs_rebuild = false;
            // A fresh encoder must open with a keyframe so the decoder re-syncs.
            self.force_idr_next = true;
        }
        let idr = force_idr || self.force_idr_next;
        if idr {
            self.encoder.force_intra_frame();
        }
        self.force_idr_next = false;
        let i420 = crate::video::convert::bgra_to_i420(frame, frame.width as usize, frame.height as usize);
        let yuv = YUVSlices::new(
            (&i420.y, &i420.u, &i420.v),
            (i420.width, i420.height),
            (i420.y_stride, i420.uv_stride, i420.uv_stride),
        );
        let bitstream = self.encoder.encode(&yuv)?;
        let data = bitstream.to_vec();
        // openh264 emits SPS+PPS with each IDR; treat a forced-IDR frame as keyframe.
        Ok(EncodedSample { data, duration: self.frame_dur, keyframe: idr })
    }

    fn set_bitrate(&mut self, bitrate_bps: u32) {
        if bitrate_bps == self.bitrate_bps {
            return;
        }
        match Self::build_encoder(bitrate_bps, self.fps) {
            Ok(enc) => {
                self.encoder = enc;
                self.bitrate_bps = bitrate_bps;
                // A fresh encoder must open with a keyframe so the decoder re-syncs.
                self.force_idr_next = true;
            }
            Err(e) => tracing::warn!("set_bitrate rebuild failed, keeping current bitrate: {e}"),
        }
    }

    fn reset(&mut self) {
        // Defer the rebuild to the next encode(): it can propagate a rebuild
        // failure as a per-frame error, guaranteeing the old codec never sees
        // frames at the new dimensions.
        self.needs_rebuild = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::VideoEncoder;

    fn gray_bgra(w: usize, h: usize) -> crate::video::Frame {
        crate::video::Frame { width: w as u32, height: h as u32, stride: w * 4, data: vec![128u8; w * h * 4], ts_micros: 0 }
    }

    // The first encoded frame must be a keyframe carrying SPS(7)+PPS(8)+IDR(5)
    // NAL units (Annex-B start codes), so a fresh browser decoder can start.
    #[test]
    fn first_frame_is_keyframe_with_parameter_sets() {
        let mut enc = Openh264Encoder::new(64, 64, 1_000_000, 30.0).unwrap();
        let sample = enc.encode(&gray_bgra(64, 64), true).unwrap();
        assert!(!sample.data.is_empty());
        assert!(sample.keyframe);
        let types = nal_types(&sample.data);
        assert!(types.contains(&7), "missing SPS; got {types:?}");
        assert!(types.contains(&8), "missing PPS; got {types:?}");
        assert!(types.contains(&5), "missing IDR; got {types:?}");
    }

    #[test]
    fn subsequent_frame_encodes_without_error() {
        let mut enc = Openh264Encoder::new(64, 64, 1_000_000, 30.0).unwrap();
        let _ = enc.encode(&gray_bgra(64, 64), true).unwrap();
        let s = enc.encode(&gray_bgra(64, 64), false).unwrap();
        assert!(!s.data.is_empty());
    }

    #[test]
    fn set_bitrate_rebuild_still_emits_keyframe() {
        let mut enc = Openh264Encoder::new(64, 64, 1_000_000, 30.0).unwrap();
        let _ = enc.encode(&gray_bgra(64, 64), true).unwrap();
        enc.set_bitrate(4_000_000);
        // force_idr=false, but the rebuilt encoder must still emit SPS+PPS+IDR.
        let s = enc.encode(&gray_bgra(64, 64), false).unwrap();
        assert!(s.keyframe);
        let types = nal_types(&s.data);
        assert!(types.contains(&7) && types.contains(&8) && types.contains(&5), "got {types:?}");
    }

    #[test]
    fn reset_then_encode_emits_keyframe_with_parameter_sets() {
        let mut enc = Openh264Encoder::new(64, 64, 1_000_000, 30.0).unwrap();
        let _ = enc.encode(&gray_bgra(64, 64), true).unwrap();
        enc.reset();
        let s = enc.encode(&gray_bgra(64, 64), false).unwrap();
        assert!(s.keyframe);
        let types = nal_types(&s.data);
        assert!(types.contains(&7) && types.contains(&8) && types.contains(&5), "got {types:?}");
    }

    /// Parse Annex-B NAL unit types (5 low bits of the byte after each start code).
    fn nal_types(annexb: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 4 <= annexb.len() {
            let is_sc4 = annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 0 && annexb[i + 3] == 1;
            let is_sc3 = annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 1;
            if is_sc4 { out.push(annexb[i + 4] & 0x1f); i += 5; }
            else if is_sc3 { out.push(annexb[i + 3] & 0x1f); i += 4; }
            else { i += 1; }
        }
        out
    }
}
