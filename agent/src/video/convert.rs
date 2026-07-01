use crate::video::{Frame, I420};
use yuv::{bgra_to_yuv420, YuvConversionMode, YuvPlanarImageMut, YuvRange, YuvStandardMatrix};

/// Nearest-neighbour resize of a BGRA buffer into a tightly-packed BGRA Vec.
fn resize_bgra(frame: &Frame, dst_w: usize, dst_h: usize) -> Vec<u8> {
    let (sw, sh, sstride) = (frame.width as usize, frame.height as usize, frame.stride);
    if sw == dst_w && sh == dst_h && sstride == dst_w * 4 {
        return frame.data.clone();
    }
    let mut out = vec![0u8; dst_w * dst_h * 4];
    for dy in 0..dst_h {
        let sy = dy * sh / dst_h;
        for dx in 0..dst_w {
            let sx = dx * sw / dst_w;
            let s = sy * sstride + sx * 4;
            let d = (dy * dst_w + dx) * 4;
            out[d..d + 4].copy_from_slice(&frame.data[s..s + 4]);
        }
    }
    out
}

/// Convert a BGRA frame to I420, resizing to `dst_w`×`dst_h` first if needed.
pub fn bgra_to_i420(frame: &Frame, dst_w: usize, dst_h: usize) -> I420 {
    if dst_w == 0 || dst_h == 0 {
        return I420 { width: 0, height: 0, y: Vec::new(), u: Vec::new(), v: Vec::new(), y_stride: 0, uv_stride: 0 };
    }
    let bgra = resize_bgra(frame, dst_w, dst_h);
    let mut planar = YuvPlanarImageMut::<u8>::alloc(dst_w as u32, dst_h as u32, yuv::YuvChromaSubsampling::Yuv420);
    bgra_to_yuv420(
        &mut planar,
        &bgra,
        (dst_w * 4) as u32,
        YuvRange::Limited,
        YuvStandardMatrix::Bt601,
        YuvConversionMode::Balanced,
    )
    .expect("bgra_to_yuv420");
    I420 {
        width: dst_w,
        height: dst_h,
        y: planar.y_plane.borrow().to_vec(),
        u: planar.u_plane.borrow().to_vec(),
        v: planar.v_plane.borrow().to_vec(),
        y_stride: planar.y_stride as usize,
        uv_stride: planar.u_stride as usize,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::Frame;

    fn solid_bgra(w: usize, h: usize, b: u8, g: u8, r: u8) -> Frame {
        let stride = w * 4;
        let mut data = vec![0u8; stride * h];
        for px in data.chunks_exact_mut(4) {
            px[0] = b; px[1] = g; px[2] = r; px[3] = 255;
        }
        Frame { width: w as u32, height: h as u32, stride, data, ts_micros: 0 }
    }

    #[test]
    fn i420_dimensions_and_plane_sizes() {
        let f = solid_bgra(16, 16, 0, 0, 0);
        let i = bgra_to_i420(&f, 16, 16);
        assert_eq!((i.width, i.height), (16, 16));
        assert_eq!(i.y.len(), i.y_stride * 16);
        assert_eq!(i.u.len(), i.uv_stride * 8);
        assert_eq!(i.v.len(), i.uv_stride * 8);
    }

    #[test]
    fn black_and_white_luma_ordering() {
        let black = bgra_to_i420(&solid_bgra(16, 16, 0, 0, 0), 16, 16);
        let white = bgra_to_i420(&solid_bgra(16, 16, 255, 255, 255), 16, 16);
        // white luma must be much brighter than black luma
        assert!(black.y[0] < 40, "black luma {} too high", black.y[0]);
        assert!(white.y[0] > 200, "white luma {} too low", white.y[0]);
    }

    #[test]
    fn resizes_to_target() {
        let i = bgra_to_i420(&solid_bgra(32, 32, 0, 0, 0), 16, 16);
        assert_eq!((i.width, i.height), (16, 16));
    }

    #[test]
    fn zero_target_returns_empty_without_panic() {
        let i = bgra_to_i420(&solid_bgra(16, 16, 0, 0, 0), 0, 0);
        assert_eq!((i.width, i.height), (0, 0));
        assert!(i.y.is_empty() && i.u.is_empty() && i.v.is_empty());
    }

    #[test]
    fn handles_padded_source_stride() {
        // Source with row padding: stride > width*4. Fill visible pixels white,
        // padding black; converting must read via stride and yield high luma.
        let (w, h) = (16usize, 16usize);
        let stride = w * 4 + 32; // padded
        let mut data = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * stride + x * 4;
                data[i] = 255; data[i + 1] = 255; data[i + 2] = 255; data[i + 3] = 255;
            }
        }
        let frame = Frame { width: w as u32, height: h as u32, stride, data, ts_micros: 0 };
        let out = bgra_to_i420(&frame, w, h);
        assert!(out.y[0] > 200, "white luma {} too low — stride mishandled", out.y[0]);
    }
}
