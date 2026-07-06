//! macOS VideoToolbox hardware H.264 encoder implementing [`VideoEncoder`].
//!
//! VideoToolbox delivers compressed frames asynchronously through a C output
//! callback. The pipeline that drives this encoder is single-threaded and feeds
//! one frame at a time, so `encode()` submits a frame, flushes it with
//! `VTCompressionSessionCompleteFrames`, and blocks on a channel until the
//! callback hands back that frame's Annex-B bytes. Exactly one frame is ever in
//! flight, which keeps the synchronous contract simple and low-latency.

use std::ffi::{c_char, c_int, c_void};
use std::ptr::{self, NonNull};
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_media::{
    kCMSampleAttachmentKey_NotSync, kCMVideoCodecType_H264, CMSampleBuffer, CMTime,
    CMVideoFormatDescriptionGetH264ParameterSetAtIndex,
};
use objc2_core_video::{kCVPixelFormatType_32BGRA, CVPixelBuffer, CVPixelBufferCreateWithBytes};
use objc2_video_toolbox::{
    kVTCompressionPropertyKey_AllowFrameReordering, kVTCompressionPropertyKey_AverageBitRate,
    kVTCompressionPropertyKey_MaxKeyFrameInterval, kVTCompressionPropertyKey_ProfileLevel,
    kVTCompressionPropertyKey_RealTime,
    kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
    kVTEncodeFrameOptionKey_ForceKeyFrame, kVTProfileLevel_H264_Baseline_AutoLevel,
    kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel, VTCompressionSession, VTEncodeInfoFlags,
    VTSessionCopyProperty, VTSessionSetProperty,
};

use crate::video::{EncodedSample, Frame, VideoEncoder};

/// A message from the async output callback to the blocking `encode()`: either
/// the finished frame's Annex-B bytes + keyframe flag, or an error string.
type CallbackMsg = Result<(Vec<u8>, bool), String>;

/// Hardware H.264 encoder backed by a `VTCompressionSession`.
pub struct VideoToolboxEncoder {
    session: CFRetained<VTCompressionSession>,
    width: u32,
    height: u32,
    bitrate_bps: u32,
    fps: f32,
    frame_dur: Duration,
    /// Strictly-increasing presentation-timestamp counter (VT requires it).
    frame_count: i64,
    /// Deferred full reopen (mirrors the openh264 `reset()` contract): the next
    /// `encode()` tears down the session, builds a fresh one, and forces an IDR.
    needs_reopen: bool,
    /// Heap-stable `Sender` handed to the C callback via its refcon. Boxed so the
    /// pointer stays valid even though `self` moves out of `new()`.
    tx_ptr: *mut Sender<CallbackMsg>,
    rx: Receiver<CallbackMsg>,
}

// SAFETY: The session, the boxed sender pointer, and the receiver are only ever
// touched from the single pipeline thread that owns the encoder. VideoToolbox's
// callback runs synchronously during `encode()` (we flush with
// CompleteFrames before reading the channel), so there is no concurrent access.
unsafe impl Send for VideoToolboxEncoder {}

impl VideoToolboxEncoder {
    pub fn new(width: u32, height: u32, bitrate_bps: u32, fps: f32) -> anyhow::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel::<CallbackMsg>();
        let tx_ptr = Box::into_raw(Box::new(tx));
        // If session setup fails we must not leak the boxed sender.
        let session = match Self::create_session(width, height, bitrate_bps, fps, tx_ptr) {
            Ok(s) => s,
            Err(e) => {
                // SAFETY: tx_ptr came from Box::into_raw above and was not shared.
                drop(unsafe { Box::from_raw(tx_ptr) });
                return Err(e);
            }
        };
        Ok(Self {
            session,
            width,
            height,
            bitrate_bps,
            fps,
            frame_dur: Duration::from_secs_f32(1.0 / fps),
            frame_count: 0,
            needs_reopen: false,
            tx_ptr,
            rx,
        })
    }

    /// Build and configure a fresh compression session wired to `tx_ptr`.
    fn create_session(
        width: u32,
        height: u32,
        bitrate_bps: u32,
        fps: f32,
        tx_ptr: *mut Sender<CallbackMsg>,
    ) -> anyhow::Result<CFRetained<VTCompressionSession>> {
        let mut out: *mut VTCompressionSession = ptr::null_mut();
        // SAFETY: Standard VTCompressionSessionCreate call. `out` is a valid slot;
        // the callback fn matches VTCompressionOutputCallback; refcon is a live
        // boxed Sender that outlives the session (freed in Drop after invalidate).
        let status = unsafe {
            VTCompressionSession::create(
                None,
                width as i32,
                height as i32,
                kCMVideoCodecType_H264,
                None,
                None,
                None,
                Some(output_callback),
                tx_ptr as *mut c_void,
                NonNull::new(&mut out).unwrap(),
            )
        };
        if status != 0 || out.is_null() {
            anyhow::bail!("VTCompressionSessionCreate failed: status {status}");
        }
        // SAFETY: Create returns a +1 retained session; take ownership without
        // an extra retain.
        let session = unsafe { CFRetained::from_raw(NonNull::new(out).unwrap()) };

        // RealTime + no frame reordering: lowest-latency, decode==display order.
        Self::set_bool(&session, unsafe { kVTCompressionPropertyKey_RealTime }, true)?;
        Self::set_bool(
            &session,
            unsafe { kVTCompressionPropertyKey_AllowFrameReordering },
            false,
        )?;

        // Prefer ConstrainedBaseline; fall back to Baseline if the encoder rejects it.
        let profile_key = unsafe { kVTCompressionPropertyKey_ProfileLevel };
        let constrained = unsafe { kVTProfileLevel_H264_ConstrainedBaseline_AutoLevel };
        // SAFETY: valid session/key/value CF references.
        let pst = unsafe {
            VTSessionSetProperty(session.as_ref(), profile_key, Some(cf(constrained)))
        };
        if pst != 0 {
            let baseline = unsafe { kVTProfileLevel_H264_Baseline_AutoLevel };
            // SAFETY: valid session/key/value CF references.
            let bst =
                unsafe { VTSessionSetProperty(session.as_ref(), profile_key, Some(cf(baseline))) };
            if bst != 0 {
                anyhow::bail!("set ProfileLevel failed: constrained {pst}, baseline {bst}");
            }
        }

        Self::set_i32(
            &session,
            unsafe { kVTCompressionPropertyKey_AverageBitRate },
            bitrate_bps as i32,
        )?;
        // Safety ceiling only; we drive keyframes explicitly per frame.
        Self::set_i32(
            &session,
            unsafe { kVTCompressionPropertyKey_MaxKeyFrameInterval },
            (fps * 8.0) as i32,
        )?;

        // SAFETY: valid session; prepares internal resources before encoding.
        let prep = unsafe { session.prepare_to_encode_frames() };
        if prep != 0 {
            anyhow::bail!("VTCompressionSessionPrepareToEncodeFrames failed: {prep}");
        }

        Self::log_hardware_acceleration(&session);
        Ok(session)
    }

    /// Read and log the `UsingHardwareAcceleratedVideoEncoder` property.
    fn log_hardware_acceleration(session: &VTCompressionSession) {
        let key = unsafe { kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder };
        let mut value: *const CFBoolean = ptr::null();
        // SAFETY: valid session/key; property_value_out points at a pointer slot.
        let st = unsafe {
            VTSessionCopyProperty(
                session.as_ref(),
                key,
                None,
                &mut value as *mut *const CFBoolean as *mut c_void,
            )
        };
        if st == 0 && !value.is_null() {
            // SAFETY: on success VT returned a +1 retained CFBoolean here.
            let boolean = unsafe { &*value };
            let hw = boolean.value();
            tracing::info!("VideoToolbox UsingHardwareAcceleratedVideoEncoder={hw}");
            // SAFETY: VTSessionCopyProperty follows the CF "copy" rule (+1);
            // reclaim it so it is released.
            drop(unsafe { CFRetained::from_raw(NonNull::new_unchecked(value as *mut CFBoolean)) });
        } else {
            tracing::info!("VideoToolbox UsingHardwareAcceleratedVideoEncoder=unknown (status {st})");
        }
    }

    fn set_bool(
        session: &VTCompressionSession,
        key: &CFString,
        val: bool,
    ) -> anyhow::Result<()> {
        let boolean = CFBoolean::new(val);
        // SAFETY: valid session/key/value CF references.
        let st = unsafe { VTSessionSetProperty(session.as_ref(), key, Some(cf(boolean))) };
        if st != 0 {
            anyhow::bail!("VTSessionSetProperty(bool) failed: {st}");
        }
        Ok(())
    }

    fn set_i32(
        session: &VTCompressionSession,
        key: &CFString,
        val: i32,
    ) -> anyhow::Result<()> {
        let num = CFNumber::new_i32(val);
        // SAFETY: valid session/key/value CF references.
        let st = unsafe { VTSessionSetProperty(session.as_ref(), key, Some(cf(&*num))) };
        if st != 0 {
            anyhow::bail!("VTSessionSetProperty(i32) failed: {st}");
        }
        Ok(())
    }

    /// Tear down the current session and build a fresh one (reusing the channel).
    fn reopen(&mut self) -> anyhow::Result<()> {
        // SAFETY: valid session; deterministic teardown before we drop our ref.
        unsafe { self.session.invalidate() };
        self.session =
            Self::create_session(self.width, self.height, self.bitrate_bps, self.fps, self.tx_ptr)?;
        self.frame_count = 0;
        Ok(())
    }
}

/// Coerce any CF type reference down to `&CFType` for the property/value APIs.
fn cf<T: std::ops::Deref<Target = CFType>>(v: &T) -> &CFType {
    v
}

/// The C output callback. `output_ref_con` is the boxed `Sender`.
///
/// # Safety
/// Invoked by VideoToolbox with a valid (or null on drop) `CMSampleBuffer` and
/// the refcon we registered in `create_session`.
unsafe extern "C-unwind" fn output_callback(
    output_ref_con: *mut c_void,
    _source_ref_con: *mut c_void,
    status: i32,
    info_flags: VTEncodeInfoFlags,
    sample_buffer: *mut CMSampleBuffer,
) {
    // SAFETY: refcon is the live `*mut Sender` from create_session.
    let tx = unsafe { &*(output_ref_con as *const Sender<CallbackMsg>) };
    let dropped = info_flags.contains(VTEncodeInfoFlags::FrameDropped);
    if status != 0 || sample_buffer.is_null() || dropped {
        let _ = tx.send(Err(format!("encode failed: status {status}, dropped {dropped}")));
        return;
    }
    // SAFETY: non-null sample buffer owned by VT for the callback's duration.
    let sb = unsafe { &*sample_buffer };
    let _ = tx.send(unsafe { sample_to_annexb(sb) });
}

/// Convert VideoToolbox's AVCC `CMSampleBuffer` into Annex-B, prepending SPS/PPS
/// from the format description on keyframes.
///
/// # Safety
/// `sb` must be a valid `CMSampleBuffer` produced by the compression session.
unsafe fn sample_to_annexb(sb: &CMSampleBuffer) -> CallbackMsg {
    // Keyframe unless the NotSync attachment is present and true.
    let keyframe = unsafe { is_keyframe(sb) };

    let mut out: Vec<u8> = Vec::new();

    if keyframe {
        // SAFETY: format description is valid for the sample's lifetime.
        let fd = match unsafe { sb.format_description() } {
            Some(fd) => fd,
            None => return Err("keyframe without format description".to_string()),
        };
        // Query the parameter-set count via index 0, then emit each.
        let mut count: usize = 0;
        // SAFETY: out-params are valid pointers; other outs are null (allowed).
        let st = unsafe {
            CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                &fd,
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                &mut count,
                ptr::null_mut(),
            )
        };
        if st != 0 {
            return Err(format!("H264 parameter set count query failed: {st}"));
        }
        for idx in 0..count {
            let mut ps_ptr: *const u8 = ptr::null();
            let mut ps_size: usize = 0;
            let mut nal_hdr_len: c_int = 0;
            // SAFETY: valid format description; out-params are valid pointers.
            let st = unsafe {
                CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                    &fd,
                    idx,
                    &mut ps_ptr,
                    &mut ps_size,
                    ptr::null_mut(),
                    &mut nal_hdr_len,
                )
            };
            if st != 0 || ps_ptr.is_null() {
                return Err(format!("H264 parameter set {idx} fetch failed: {st}"));
            }
            // VideoToolbox always emits a 4-byte NAL length prefix for H.264, which
            // the AVCC→Annex-B loop below hard-codes. Assert the invariant so a
            // future toolchain change that returned a different width is caught.
            debug_assert_eq!(nal_hdr_len, 4, "VT H.264 NAL length prefix is always 4 bytes");
            out.extend_from_slice(&[0, 0, 0, 1]);
            // SAFETY: ps_ptr/ps_size describe VT-internal memory valid while `fd`
            // is retained (it is, for this scope).
            out.extend_from_slice(unsafe { std::slice::from_raw_parts(ps_ptr, ps_size) });
        }
    }

    // Frame NALs: AVCC [4-byte BE length][NAL]... → start-code prefixed.
    // SAFETY: sample carries a block buffer of compressed data.
    let block = match unsafe { sb.data_buffer() } {
        Some(b) => b,
        None => return Err("sample without data buffer".to_string()),
    };
    let mut total: usize = 0;
    let mut length_at_offset: usize = 0;
    let mut data_ptr: *mut c_char = ptr::null_mut();
    // SAFETY: valid block buffer; request the contiguous run length at offset 0
    // AND the total logical length so we can detect a non-contiguous buffer.
    let st = unsafe { block.data_pointer(0, &mut length_at_offset, &mut total, &mut data_ptr) };
    if st != 0 || data_ptr.is_null() {
        return Err(format!("CMBlockBufferGetDataPointer failed: {st}"));
    }
    // Fast path: the whole logical range is contiguous, so borrow it copy-free.
    // Slow path: the block buffer is segmented and `data_ptr` is only valid for
    // the first `length_at_offset` bytes; copy the full range out before parsing
    // so we never read past the first block.
    let owned: Vec<u8>;
    let avcc: &[u8] = if length_at_offset == total {
        // SAFETY: data_ptr is valid for `total` contiguous bytes on this path.
        unsafe { std::slice::from_raw_parts(data_ptr as *const u8, total) }
    } else {
        let mut buf = vec![0u8; total];
        // SAFETY: valid block buffer; `buf` owns `total` writable bytes and is a
        // valid, non-null destination for the full logical range.
        let cst = unsafe {
            block.copy_data_bytes(0, total, NonNull::new(buf.as_mut_ptr() as *mut c_void).unwrap())
        };
        if cst != 0 {
            return Err(format!("CMBlockBufferCopyDataBytes failed: {cst}"));
        }
        owned = buf;
        &owned
    };
    let mut i = 0usize;
    while i + 4 <= total {
        let nal_len =
            u32::from_be_bytes([avcc[i], avcc[i + 1], avcc[i + 2], avcc[i + 3]]) as usize;
        i += 4;
        if nal_len == 0 || i + nal_len > total {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&avcc[i..i + nal_len]);
        i += nal_len;
    }

    Ok((out, keyframe))
}

/// Read the sample attachments to decide whether this frame is a sync/IDR frame.
///
/// # Safety
/// `sb` must be a valid `CMSampleBuffer`.
unsafe fn is_keyframe(sb: &CMSampleBuffer) -> bool {
    // SAFETY: do not create the array if absent.
    let Some(array) = (unsafe { sb.sample_attachments_array(false) }) else {
        return true;
    };
    if array.count() == 0 {
        return true;
    }
    // SAFETY: index 0 is in bounds (count checked); value is a CFDictionary.
    let dict_ptr = unsafe { array.value_at_index(0) } as *const CFDictionary;
    if dict_ptr.is_null() {
        return true;
    }
    // SAFETY: attachment entry is a CFDictionary valid for the sample's lifetime.
    let dict = unsafe { &*dict_ptr };
    let key = unsafe { kCMSampleAttachmentKey_NotSync };
    // SAFETY: valid dict/key; value (if any) is a CFBoolean.
    let val = unsafe { dict.value(key as *const CFString as *const c_void) };
    if val.is_null() {
        // No NotSync attachment → sync frame (keyframe).
        return true;
    }
    // SAFETY: NotSync value is a CFBoolean.
    let not_sync = unsafe { &*(val as *const CFBoolean) }.value();
    !not_sync
}

impl VideoEncoder for VideoToolboxEncoder {
    fn encode(&mut self, frame: &Frame, force_idr: bool) -> anyhow::Result<EncodedSample> {
        // A deferred reset() rebuild happens here so a stale session never sees a
        // frame; a rebuild failure surfaces as a per-frame Err (pipeline logs it).
        let mut force = force_idr;
        if self.needs_reopen {
            // A VTCompressionSession is bound to its create-time dimensions, so a
            // resolution hot-switch (pipeline resizes frames, then calls reset())
            // must rebuild the session at the incoming frame's size. Unlike
            // openh264, VT cannot infer size per frame; adopt the authoritative
            // size the pipeline already resized this frame to. Defense-in-depth:
            // a reset() followed by a frame of ANY size reopens correctly.
            self.width = frame.width;
            self.height = frame.height;
            self.reopen()?;
            self.needs_reopen = false;
            force = true; // fresh session must open on a keyframe
        }

        // Guard against a malformed frame: CV reads `stride * height` bytes from
        // the borrowed buffer, so a short `frame.data` would be an OOB read in VT.
        let needed = frame.stride.saturating_mul(frame.height as usize);
        if frame.data.len() < needed {
            anyhow::bail!(
                "frame data too small: {} bytes < stride {} * height {} = {needed}",
                frame.data.len(),
                frame.stride,
                frame.height,
            );
        }

        // Wrap the BGRA bytes in a CVPixelBuffer (a copy-free borrow: the encoder
        // reads them synchronously and we flush before returning, so `frame.data`
        // outlives the buffer). A pooled IOSurface path is a later optimization.
        let base = NonNull::new(frame.data.as_ptr() as *mut c_void)
            .ok_or_else(|| anyhow::anyhow!("empty frame data"))?;
        let mut pb_out: *mut CVPixelBuffer = ptr::null_mut();
        // SAFETY: base/stride describe `frame.data`; None release callback means
        // CV borrows the bytes (we keep them alive for the call). Out slot valid.
        let cv = unsafe {
            CVPixelBufferCreateWithBytes(
                None,
                frame.width as usize,
                frame.height as usize,
                kCVPixelFormatType_32BGRA,
                base,
                frame.stride,
                None,
                ptr::null_mut(),
                None,
                NonNull::new(&mut pb_out).unwrap(),
            )
        };
        if cv != 0 || pb_out.is_null() {
            anyhow::bail!("CVPixelBufferCreateWithBytes failed: {cv}");
        }
        // SAFETY: Create rule (+1); take ownership so it releases at scope end.
        let pixel_buffer = unsafe { CFRetained::from_raw(NonNull::new(pb_out).unwrap()) };

        // Strictly-increasing PTS over an fps timescale.
        let timescale = self.fps.max(1.0) as i32;
        // SAFETY: builds a valid CMTime.
        let pts = unsafe { CMTime::new(self.frame_count, timescale) };
        // SAFETY: builds a valid CMTime.
        let dur = unsafe { CMTime::new(1, timescale) };
        self.frame_count += 1;

        // Per-frame ForceKeyFrame option dictionary when a keyframe is requested.
        let force_dict = if force {
            let key = unsafe { kVTEncodeFrameOptionKey_ForceKeyFrame };
            let val = CFBoolean::new(true);
            Some(CFDictionary::from_slices(&[key], &[val]))
        } else {
            None
        };
        // Erase the typed CFDictionary<CFString, CFBoolean> to the untyped
        // &CFDictionary the encode API expects.
        let force_ref: Option<&CFDictionary> = force_dict.as_deref().map(AsRef::as_ref);

        // SAFETY: valid session and pixel buffer; frame_properties optional dict is
        // valid for the call; no source refcon; info flags discarded.
        let st = unsafe {
            self.session.encode_frame(
                &pixel_buffer,
                pts,
                dur,
                force_ref,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if st != 0 {
            // The frame may already have been submitted and the callback may have
            // queued a message; drain it so the channel is empty for the next
            // encode() (contract: exactly one recv() per encode()).
            while self.rx.try_recv().is_ok() {}
            anyhow::bail!("VTCompressionSessionEncodeFrame failed: {st}");
        }

        // Flush this frame so the callback delivers it before we block on recv().
        // SAFETY: valid session; completes frames up to and including `pts`.
        let cf_st = unsafe { self.session.complete_frames(pts) };
        if cf_st != 0 {
            // CompleteFrames fires the output callback before returning its status,
            // so a stale sample may be queued even on failure; drain it to keep the
            // one-message-per-encode() invariant (else the next encode() would
            // recv() this frame's bytes and keyframe flag).
            while self.rx.try_recv().is_ok() {}
            anyhow::bail!("VTCompressionSessionCompleteFrames failed: {cf_st}");
        }

        let (data, keyframe) = self
            .rx
            .recv()
            .map_err(|_| anyhow::anyhow!("encoder callback channel closed"))?
            .map_err(|e| anyhow::anyhow!("VideoToolbox encode: {e}"))?;

        Ok(EncodedSample { data, duration: self.frame_dur, keyframe })
    }

    fn set_bitrate(&mut self, bitrate_bps: u32) {
        if bitrate_bps == self.bitrate_bps {
            return;
        }
        let num = CFNumber::new_i32(bitrate_bps as i32);
        // SAFETY: valid live session/key/value; adjusts bitrate without reopen.
        let st = unsafe {
            VTSessionSetProperty(
                self.session.as_ref(),
                kVTCompressionPropertyKey_AverageBitRate,
                Some(cf(&*num)),
            )
        };
        if st == 0 {
            self.bitrate_bps = bitrate_bps;
        } else {
            tracing::warn!("set_bitrate failed (status {st}); keeping {}", self.bitrate_bps);
        }
    }

    fn reset(&mut self) {
        // Defer the reopen to the next encode(): it can propagate a rebuild
        // failure as a per-frame error and forces a fresh keyframe.
        self.needs_reopen = true;
    }
}

impl Drop for VideoToolboxEncoder {
    fn drop(&mut self) {
        // SAFETY: valid session; deterministic teardown.
        unsafe { self.session.invalidate() };
        // SAFETY: tx_ptr came from Box::into_raw in new() and is not aliased after
        // the session is invalidated (no more callbacks can fire).
        drop(unsafe { Box::from_raw(self.tx_ptr) });
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::VideoToolboxEncoder;
    use crate::video::{Frame, VideoEncoder};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    /// VideoToolbox is a shared hardware encoder: creating several
    /// `VTCompressionSession`s concurrently across cargo's parallel test threads
    /// can make the H.264 encoder emit keyframes without SPS/PPS (observed as an
    /// SEI+IDR-only Annex-B stream) and can briefly degrade system-wide VT state.
    /// Serialize the encoder tests so at most one session is live at a time.
    fn vt_test_guard() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        // Recover from a poisoned lock: a panicking test still frees the hardware.
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner())
    }

    fn bgra(w: usize, h: usize) -> Frame {
        Frame { width: w as u32, height: h as u32, stride: w * 4, data: vec![128u8; w * h * 4], ts_micros: 0 }
    }
    fn nal_types(annexb: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i + 4 <= annexb.len() {
            if annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 0 && annexb[i + 3] == 1 {
                if i + 4 < annexb.len() { out.push(annexb[i + 4] & 0x1f); }
                i += 5;
            } else { i += 1; }
        }
        out
    }

    /// Extract the raw bytes of the first NAL of `want_type` from a 4-byte
    /// start-code Annex-B stream (includes the 1-byte NAL header).
    fn find_nal(annexb: &[u8], want_type: u8) -> Option<Vec<u8>> {
        let mut starts = Vec::new();
        let mut i = 0;
        while i + 4 <= annexb.len() {
            if annexb[i] == 0 && annexb[i + 1] == 0 && annexb[i + 2] == 0 && annexb[i + 3] == 1 {
                starts.push(i + 4);
                i += 4;
            } else {
                i += 1;
            }
        }
        for (idx, &s) in starts.iter().enumerate() {
            let end = if idx + 1 < starts.len() { starts[idx + 1] - 4 } else { annexb.len() };
            if s < end && (annexb[s] & 0x1f) == want_type {
                return Some(annexb[s..end].to_vec());
            }
        }
        None
    }

    /// Strip H.264 emulation-prevention (00 00 03) bytes to recover the RBSP.
    fn rbsp(nal: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(nal.len());
        let mut zeros = 0;
        for &b in nal {
            if zeros >= 2 && b == 0x03 {
                zeros = 0;
                continue;
            }
            out.push(b);
            if b == 0 { zeros += 1; } else { zeros = 0; }
        }
        out
    }

    /// Minimal MSB-first bit reader with Exp-Golomb decoders for SPS parsing.
    struct BitReader<'a> { d: &'a [u8], pos: usize }
    impl<'a> BitReader<'a> {
        fn new(d: &'a [u8]) -> Self { Self { d, pos: 0 } }
        fn bit(&mut self) -> u32 {
            let byte = self.pos / 8;
            let off = 7 - (self.pos % 8);
            self.pos += 1;
            ((self.d[byte] >> off) & 1) as u32
        }
        fn bits(&mut self, n: u32) -> u32 {
            let mut v = 0;
            for _ in 0..n { v = (v << 1) | self.bit(); }
            v
        }
        fn ue(&mut self) -> u32 {
            let mut z = 0;
            while self.bit() == 0 { z += 1; }
            if z == 0 { 0 } else { ((1u32 << z) - 1) + self.bits(z) }
        }
        fn se(&mut self) -> i32 {
            let k = self.ue();
            if k % 2 == 1 { k.div_ceil(2) as i32 } else { -((k / 2) as i32) }
        }
    }

    /// Decode the coded (cropped) luma dimensions from an H.264 SPS NAL.
    /// Handles Baseline/ConstrainedBaseline (what this encoder emits).
    fn sps_dimensions(sps_nal: &[u8]) -> (u32, u32) {
        let rbsp = rbsp(sps_nal);
        // Skip the 1-byte NAL header; parse the SPS RBSP body.
        let mut r = BitReader::new(&rbsp[1..]);
        let profile_idc = r.bits(8);
        let _constraints_and_reserved = r.bits(8);
        let _level_idc = r.bits(8);
        let _sps_id = r.ue();
        // 4:2:0 is the default chroma format when the high-profile block is absent.
        let mut chroma_format_idc = 1u32;
        if matches!(profile_idc, 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
            chroma_format_idc = r.ue();
            if chroma_format_idc == 3 { let _ = r.bit(); }
            let _ = r.ue(); // bit_depth_luma_minus8
            let _ = r.ue(); // bit_depth_chroma_minus8
            let _ = r.bit(); // qpprime_y_zero_transform_bypass_flag
            let seq_scaling_matrix_present = r.bit();
            assert_eq!(seq_scaling_matrix_present, 0, "scaling matrices not expected for baseline");
        }
        let _ = r.ue(); // log2_max_frame_num_minus4
        let poc_type = r.ue();
        if poc_type == 0 {
            let _ = r.ue(); // log2_max_pic_order_cnt_lsb_minus4
        } else if poc_type == 1 {
            let _ = r.bit(); // delta_pic_order_always_zero_flag
            let _ = r.se(); // offset_for_non_ref_pic
            let _ = r.se(); // offset_for_top_to_bottom_field
            let n = r.ue();
            for _ in 0..n { let _ = r.se(); }
        }
        let _ = r.ue(); // max_num_ref_frames
        let _ = r.bit(); // gaps_in_frame_num_value_allowed_flag
        let pic_width_in_mbs_minus1 = r.ue();
        let pic_height_in_map_units_minus1 = r.ue();
        let frame_mbs_only_flag = r.bit();
        if frame_mbs_only_flag == 0 { let _ = r.bit(); } // mb_adaptive_frame_field_flag
        let _ = r.bit(); // direct_8x8_inference_flag
        let (mut crop_l, mut crop_r, mut crop_t, mut crop_b) = (0u32, 0u32, 0u32, 0u32);
        if r.bit() == 1 {
            crop_l = r.ue();
            crop_r = r.ue();
            crop_t = r.ue();
            crop_b = r.ue();
        }
        let width = (pic_width_in_mbs_minus1 + 1) * 16;
        let height = (pic_height_in_map_units_minus1 + 1) * 16 * (2 - frame_mbs_only_flag);
        // ChromaArrayType==1 (4:2:0) → SubWidthC=SubHeightC=2.
        let (sub_w, sub_h) = if chroma_format_idc == 1 { (2, 2) } else { (1, 1) };
        let crop_unit_x = sub_w;
        let crop_unit_y = sub_h * (2 - frame_mbs_only_flag);
        let width = width - (crop_l + crop_r) * crop_unit_x;
        let height = height - (crop_t + crop_b) * crop_unit_y;
        (width, height)
    }

    #[test]
    fn first_frame_is_keyframe_with_parameter_sets() {
        let _guard = vt_test_guard();
        let mut enc = VideoToolboxEncoder::new(320, 240, 1_000_000, 30.0).unwrap();
        let s = enc.encode(&bgra(320, 240), true).unwrap();
        assert!(s.keyframe && !s.data.is_empty());
        let t = nal_types(&s.data);
        assert!(t.contains(&7) && t.contains(&8) && t.contains(&5), "want SPS+PPS+IDR, got {t:?}");
    }
    #[test]
    fn second_frame_encodes_and_bitrate_reset_ok() {
        let _guard = vt_test_guard();
        let mut enc = VideoToolboxEncoder::new(320, 240, 1_000_000, 30.0).unwrap();
        let _ = enc.encode(&bgra(320, 240), true).unwrap();
        enc.set_bitrate(2_000_000);
        let s = enc.encode(&bgra(320, 240), false).unwrap();
        assert!(!s.data.is_empty());
        enc.reset();
        let s2 = enc.encode(&bgra(320, 240), false).unwrap();
        assert!(s2.keyframe, "reset must re-open with a keyframe");
    }

    /// Resolution hot-switch: after reset(), a frame at a NEW size must reopen the
    /// VTCompressionSession at that size. Before the fix, reopen() recreated the
    /// session at the stale constructor dimensions (320x240) while being fed a
    /// 640x480 CVPixelBuffer, producing a per-frame encode error (or wrong-sized
    /// output) that froze the stream. We assert Ok + keyframe AND parse the SPS to
    /// prove the coded dimensions actually track the new frame size.
    #[test]
    fn reset_reopens_at_new_frame_resolution() {
        let _guard = vt_test_guard();
        let mut enc = VideoToolboxEncoder::new(320, 240, 1_000_000, 30.0).unwrap();
        let s0 = enc.encode(&bgra(320, 240), true).unwrap();
        assert!(s0.keyframe && !s0.data.is_empty());

        // Pipeline resizes to the new target, then resets the encoder.
        enc.reset();
        let s1 = enc.encode(&bgra(640, 480), false).unwrap();
        // A stale-dimension session would have errored on this encode; reaching a
        // non-empty keyframe here is the first-order signal that reopen adopted the
        // new size.
        assert!(s1.keyframe, "reset must re-open at the new size with a keyframe");
        assert!(!s1.data.is_empty(), "post-reset frame produced no data");

        // Stronger: the SPS carried in the reopened keyframe must code 640x480.
        let sps = find_nal(&s1.data, 7).expect("keyframe must carry an SPS");
        let (w, h) = sps_dimensions(&sps);
        assert_eq!((w, h), (640, 480), "SPS must code the new resolution, got {w}x{h}");
    }
}
