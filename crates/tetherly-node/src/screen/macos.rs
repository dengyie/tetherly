// SPDX-License-Identifier: Apache-2.0 OR MIT
//! macOS screen capture via CoreGraphics. Self-contained: it imports only
//! absolute paths so this file compiles for its own target on its own, the
//! same way `cred/macos.rs` does.
//!
//! Capture path: `CGDisplayCreateImage` grabs the main display, then the image
//! is drawn into an explicit BGRA8 `CGBitmapContext` so the bytes arrive in a
//! format the wire already knows (`PixelFormat::Bgra8`, tightly packed,
//! top-down). Normalising through a known context matters: a wide-gamut or
//! Retina display can hand back 16-bit or RGBA samples, and the decoder on the
//! far side must never have to guess.
//!
//! Presentation on macOS needs an AppKit `NSWindow` (a CoreGraphics bitmap
//! context has no way to reach the screen), which the spec defers to a later
//! iteration; `present` therefore returns an honest `ScreenRefused` naming the
//! missing piece instead of pretending.
//!
//! `CGDisplayCreateImage` is soft-deprecated in favour of ScreenCaptureKit. It
//! is kept because ScreenCaptureKit requires a signed app bundle plus a user
//! consent prompt and would widen the dependency surface to AppKit; the
//! deprecation warning is allowed at the one call site rather than silenced
//! crate-wide. Screen Recording permission is a hard runtime requirement of the
//! OS: with it denied, this returns `ScreenRefused`, not a blank frame.

use objc2_core_foundation::{CFRetained, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextGetBytesPerRow, CGBitmapContextGetData, CGBlendMode,
    CGColorSpace, CGContext, CGDirectDisplayID, CGDisplayPixelsHigh, CGDisplayPixelsWide, CGImage,
    CGImageAlphaInfo, CGImageByteOrderInfo, CGMainDisplayID,
};
// Imported apart from the rest so the soft-deprecation warning is allowed at
// this one line instead of being silenced across the whole module.
#[allow(deprecated)]
use objc2_core_graphics::CGDisplayCreateImage;
use tetherly_core::{CoreError, PixelFormat, ScreenFrame, ScreenRect, ScreenSource};

/// Bytes per pixel for every v1 screen format.
const BPP: usize = 4;

/// Bitmap flags for a packed BGRA8 buffer.
///
/// `CGBitmapInfo` packs the alpha kind and the byte order into one u32, and
/// this value is load-bearing: it decides which byte of each pixel is red and
/// which is blue, so a typo here ships colour-swapped frames that only a real
/// display would reveal.
///
/// The byte order is pinned to `Order32Little` rather than `Order32Host` on
/// purpose. `ByteOrder32Little` + `AlphaPremultipliedFirst` describes the
/// *memory* layout as B,G,R,A no matter what the CPU does, which is exactly
/// what `PixelFormat::Bgra8` means on the wire; `Order32Host` would describe a
/// host-dependent layout instead, and the two would silently disagree on a
/// big-endian machine. The named `CGBitmapInfo::ByteOrder32Little` constant is
/// avoided only because the binding crate marks it deprecated.
const BGRA8: u32 = CGImageAlphaInfo::PremultipliedFirst.0 | CGImageByteOrderInfo::Order32Little.0;

/// Row stride to ask CoreGraphics for: `w * 4` rounded up to 16 bytes.
///
/// CoreGraphics prefers an aligned row and is free to pad or to reject a less
/// convenient stride, so the request is aligned and the padding is dropped when
/// the pixels are copied out. Asking for exactly `w * 4` would leave e.g. a
/// 1365-pixel-wide display (5460 bytes per row) at the mercy of the framework.
fn stride_for(w: usize) -> usize {
    (w * BPP).div_ceil(16) * 16
}

/// Copy `h` rows of `w * 4` bytes out of a `stride`-wide buffer, dropping the
/// padding. The buffer length is validated first so a short buffer is refused
/// rather than read out of bounds.
fn pack_rows(data: &[u8], stride: usize, w: usize, h: usize) -> Result<Vec<u8>, CoreError> {
    let row = w * BPP;
    let needed = stride
        .checked_mul(h)
        .ok_or_else(|| CoreError::ScreenRefused("macOS frame stride overflows".into()))?;
    if stride < row || data.len() < needed {
        return Err(CoreError::ScreenRefused(format!(
            "macOS bitmap buffer is {} bytes, need {needed} for stride {stride}",
            data.len()
        )));
    }
    let mut out = Vec::with_capacity(row * h);
    for y in 0..h {
        let start = y * stride;
        out.extend_from_slice(&data[start..start + row]);
    }
    Ok(out)
}

/// Capture the main display as one packed BGRA8 frame.
///
/// `Ok(None)` means the display reported no pixels (no session, or the screen
/// is locked), which the runtime treats as "no new frame" exactly the way
/// DXGI's `WAIT_TIMEOUT` is treated.
pub fn grab() -> Result<Option<ScreenFrame>, CoreError> {
    let display: CGDirectDisplayID = CGMainDisplayID();
    let w = CGDisplayPixelsWide(display);
    let h = CGDisplayPixelsHigh(display);
    if w == 0 || h == 0 {
        return Ok(None);
    }
    // The wire carries the rect as u16; anything wider cannot be expressed and
    // must be refused rather than silently truncated.
    if w > u16::MAX as usize || h > u16::MAX as usize {
        return Err(CoreError::ScreenRefused(format!(
            "main display {w}x{h} exceeds the u16 frame rect"
        )));
    }
    let stride = stride_for(w);

    // Soft-deprecated in favour of ScreenCaptureKit; see the module note. It
    // returns `None` rather than a null pointer when Screen Recording
    // permission has not been granted, which is the usual first-run failure.
    #[allow(deprecated)]
    let image: CFRetained<CGImage> = CGDisplayCreateImage(display).ok_or_else(|| {
        CoreError::ScreenRefused(
            "CGDisplayCreateImage returned null (Screen Recording permission denied?)".into(),
        )
    })?;
    let image: &CGImage = &image;

    let color_space = CGColorSpace::new_device_rgb()
        .ok_or_else(|| CoreError::ScreenRefused("CGColorSpaceCreateDeviceRGB failed".into()))?;
    let color_space: &CGColorSpace = &color_space;

    // A null `data` makes the context allocate and own the backing store, so
    // there is no aliasing to reason about: the only pointer read below is the
    // one the context hands back, and it stays valid until the context drops at
    // the end of this function. The context is created per grab and never
    // stored in a `static` because `CGContext` is not `Send`/`Sync`.
    let ctx = unsafe {
        CGBitmapContextCreate(
            std::ptr::null_mut(),
            w,
            h,
            8,
            stride,
            Some(color_space),
            BGRA8,
        )
    }
    .ok_or_else(|| {
        CoreError::ScreenRefused(format!(
            "CGBitmapContextCreate returned null for {w}x{h} stride {stride}"
        ))
    })?;

    // The framework decides the real stride; use what it reports, not what we
    // asked for, and let `pack_rows` unpad it.
    let actual = CGBitmapContextGetBytesPerRow(Some(&ctx));

    // Copy, not blend: the context starts zeroed and the screenshot is wanted
    // verbatim, alpha included, without the ambient blend state tinting it.
    CGContext::set_blend_mode(Some(&ctx), CGBlendMode::Copy);
    CGContext::draw_image(
        Some(&ctx),
        CGRect::new(CGPoint::ZERO, CGSize::new(w as CGFloat, h as CGFloat)),
        Some(image),
    );

    let len = actual
        .checked_mul(h)
        .ok_or_else(|| CoreError::ScreenRefused("macOS bitmap size overflows".into()))?;
    let base = CGBitmapContextGetData(Some(&ctx));
    if base.is_null() {
        return Err(CoreError::ScreenRefused(
            "CGBitmapContextGetData returned null".into(),
        ));
    }
    // Safety: `base` is the live backing store of `ctx` and is exactly
    // `actual * h` bytes (that is the allocation CoreGraphics made for this
    // stride and height). The image was drawn over the full rect so every byte
    // is initialised, nothing mutates the buffer while it is read, and
    // `pack_rows` re-checks the length before slicing. `base` stays valid
    // until `ctx` drops, at the end of this function.
    let all = unsafe { std::slice::from_raw_parts(base as *const u8, len) };
    let bytes = pack_rows(all, actual, w, h)?;

    let rect = ScreenRect::new(0, 0, w as u16, h as u16)?;
    ScreenFrame::raw(0, rect, PixelFormat::Bgra8, bytes).map(Some)
}

/// macOS screen source. Stateless: CoreGraphics capture needs no persistent
/// handle, so `grab` can run concurrently without coordination.
#[derive(Debug, Clone, Default)]
pub struct MacSource;

impl MacSource {
    pub fn new() -> Self {
        Self
    }
}

impl ScreenSource for MacSource {
    fn grab(&self) -> Result<Option<ScreenFrame>, CoreError> {
        grab()
    }
}

/// Present a decoded frame. macOS has no CoreGraphics window, so this is the
/// honest boundary: state the missing piece plainly.
pub fn present(_frame: &ScreenFrame) -> Result<(), CoreError> {
    Err(CoreError::ScreenRefused(
        "macOS screen present needs an AppKit NSWindow; not implemented (spec defers it)".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra8_is_premultiplied_first_with_little_endian_32bit_words() {
        assert_eq!(CGImageAlphaInfo::PremultipliedFirst.0, 2);
        assert_eq!(CGImageByteOrderInfo::Order32Little.0, 2 << 12);
        assert_eq!(BGRA8, 2 | (2 << 12), "BGRA8 must name B,G,R,A in memory");
    }

    #[test]
    fn stride_for_rounds_up_to_sixteen_bytes() {
        assert_eq!(stride_for(1), 16);
        assert_eq!(stride_for(4), 16);
        assert_eq!(stride_for(5), 32);
        assert_eq!(stride_for(1920), 1920 * 4);
        // 1365 * 4 = 5460, which is not a multiple of 16.
        assert_eq!(stride_for(1365), 1365 * 4 + 12);
    }

    #[test]
    fn pack_rows_drops_row_padding() {
        // Two 1-pixel rows padded out to a 16-byte stride.
        let mut src = vec![0u8; 32];
        src[0..4].copy_from_slice(&[1, 2, 3, 4]);
        src[16..20].copy_from_slice(&[5, 6, 7, 8]);
        assert_eq!(
            pack_rows(&src, 16, 1, 2).unwrap(),
            vec![1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn pack_rows_accepts_a_stride_equal_to_a_row() {
        assert_eq!(pack_rows(&[1, 2, 3, 4], 4, 1, 1).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn pack_rows_rejects_a_stride_smaller_than_a_row() {
        assert!(pack_rows(&[0u8; 16], 4, 2, 1).is_err());
    }

    #[test]
    fn pack_rows_rejects_a_truncated_buffer() {
        assert!(pack_rows(&[0u8; 8], 16, 1, 2).is_err());
    }

    #[test]
    fn present_refuses_with_the_real_reason() {
        let rect = ScreenRect::new(0, 0, 4, 4).unwrap();
        let f = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![0u8; 4 * 4 * 4]).unwrap();
        let err = present(&f).expect_err("macOS present must refuse");
        match err {
            CoreError::ScreenRefused(msg) => assert!(msg.contains("NSWindow"), "{msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
